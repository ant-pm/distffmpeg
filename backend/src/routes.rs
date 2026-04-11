use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use uuid::Uuid;

use crate::minio;
use crate::models::*;
use crate::state::AppState;

pub async fn create_job(
    State(state): State<AppState>,
    Json(req): Json<CreateJobRequest>,
) -> Result<Json<CreateJobResponse>, StatusCode> {
    let job_id = Uuid::new_v4();
    let input_key = format!("{}/{}", job_id, req.filename);

    // Public client: browser will PUT directly to this URL
    let upload_url = minio::presigned_put(&state.s3_public, "uploads", &input_key).await;

    let job = Job {
        id: job_id,
        filename: req.filename,
        output_format: req.output_format,
        encoding: req.encoding,
        status: JobStatus::PendingUpload,
        created_at: Utc::now(),
        error: None,
        progress: None,
    };

    state.jobs.write().await.insert(job_id, job.clone());
    let _ = state.client_broadcast.send(ClientMessage::JobUpdate { job: job.clone() });

    Ok(Json(CreateJobResponse { job, upload_url }))
}

pub async fn list_jobs(State(state): State<AppState>) -> Json<Vec<Job>> {
    let jobs = state.jobs.read().await;
    let mut list: Vec<Job> = jobs.values().cloned().collect();
    list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Json(list)
}

pub async fn mark_uploaded(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Job>, StatusCode> {
    let mut jobs = state.jobs.write().await;
    let job = jobs.get_mut(&id).ok_or(StatusCode::NOT_FOUND)?;

    if job.status != JobStatus::PendingUpload {
        return Err(StatusCode::CONFLICT);
    }

    job.status = JobStatus::Queued;
    let job_clone = job.clone();
    drop(jobs);

    let input_key = format!("{}/{}", job_clone.id, job_clone.filename);
    let stem = std::path::Path::new(&job_clone.filename)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());
    let output_key = format!("{}/{}.{}", job_clone.id, stem, job_clone.output_format);

    // Submit transcode job to ColonyOS
    let mut fn_spec = colonyos::core::FunctionSpec::new(
        "transcode",
        "ffmpeg-executor",
        &state.colony_name,
    );
    fn_spec.kwargs.insert("input_key".into(), serde_json::Value::String(input_key));
    fn_spec.kwargs.insert("output_key".into(), serde_json::Value::String(output_key));
    fn_spec.kwargs.insert(
        "encoding".into(),
        serde_json::to_value(&job_clone.encoding).unwrap(),
    );
    fn_spec.maxexectime = -1;

    let proc = colonyos::submit(&fn_spec, &state.colony_prvkey)
        .await
        .map_err(|e| {
            tracing::error!("ColonyOS submit failed: {e}");
            StatusCode::SERVICE_UNAVAILABLE
        })?;

    // Spawn a background task that waits for the process to finish and
    // updates the job status when it does.
    let state2 = state.clone();
    let prvkey = state.colony_prvkey.clone();
    tokio::spawn(async move {
        match colonyos::subscribe_process(&proc, colonyos::core::SUCCESS, 3600, &prvkey).await {
            Ok(_) => {
                let mut jobs = state2.jobs.write().await;
                if let Some(job) = jobs.get_mut(&job_clone.id) {
                    job.status = JobStatus::Completed;
                    let _ = state2.client_broadcast.send(ClientMessage::JobUpdate { job: job.clone() });
                }
            }
            Err(e) => {
                tracing::error!("Job {} failed: {e}", job_clone.id);
                let mut jobs = state2.jobs.write().await;
                if let Some(job) = jobs.get_mut(&job_clone.id) {
                    job.status = JobStatus::Failed;
                    job.error = Some(e.to_string());
                    let _ = state2.client_broadcast.send(ClientMessage::JobUpdate { job: job.clone() });
                }
            }
        }
    });

    let _ = state.client_broadcast.send(ClientMessage::JobUpdate { job: job_clone.clone() });
    Ok(Json(job_clone))
}

pub async fn download_url(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<DownloadResponse>, StatusCode> {
    let jobs = state.jobs.read().await;
    let job = jobs.get(&id).ok_or(StatusCode::NOT_FOUND)?;

    if job.status != JobStatus::Completed {
        return Err(StatusCode::CONFLICT);
    }

    let stem = std::path::Path::new(&job.filename)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());
    let output_key = format!("{}/{}.{}", job.id, stem, job.output_format);

    // Public client: browser will download from this URL
    let download_url = minio::presigned_get(&state.s3_public, "outputs", &output_key).await;
    Ok(Json(DownloadResponse { download_url }))
}
