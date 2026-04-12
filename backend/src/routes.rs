use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use uuid::Uuid;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::process::Command;

use crate::minio;
use crate::models::*;
use crate::state::AppState;

const CHUNK_DURATION_SECS: f64 = 60.0;

// ── HTTP handlers ─────────────────────────────────────────────────────────────

pub async fn create_job(
    State(state): State<AppState>,
    Json(req): Json<CreateJobRequest>,
) -> Result<Json<CreateJobResponse>, StatusCode> {
    let job_id = Uuid::new_v4();
    let input_key = format!("{}/{}", job_id, req.filename);

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
        chunk_count: None,
        chunks_completed: 0,
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
    let final_output_key = format!("{}/{}.{}", job_clone.id, stem, job_clone.output_format);

    // Use internal MinIO URL for server-side ffprobe
    let probe_url = minio::presigned_get(&state.s3_internal, "uploads", &input_key).await;

    let _ = state.client_broadcast.send(ClientMessage::JobUpdate { job: job_clone.clone() });

    let state2 = state.clone();
    let output_format = job_clone.output_format.clone();
    let encoding = job_clone.encoding.clone();
    let job_id = job_clone.id;
    tokio::spawn(async move {
        if let Err(e) = orchestrate_job(
            state2.clone(),
            job_id,
            input_key,
            final_output_key,
            probe_url,
            output_format,
            encoding,
        )
        .await
        {
            tracing::error!("Job {job_id} failed: {e:#}");
            let mut jobs = state2.jobs.write().await;
            if let Some(j) = jobs.get_mut(&job_id) {
                j.status = JobStatus::Failed;
                j.error = Some(e.to_string());
                let _ = state2.client_broadcast.send(ClientMessage::JobUpdate { job: j.clone() });
            }
        }
    });

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

    let download_url = minio::presigned_get(&state.s3_public, "outputs", &output_key).await;
    Ok(Json(DownloadResponse { download_url }))
}

// ── Orchestration ─────────────────────────────────────────────────────────────

async fn orchestrate_job(
    state: AppState,
    job_id: Uuid,
    input_key: String,
    final_output_key: String,
    probe_url: String,
    output_format: String,
    encoding: EncodingOptions,
) -> anyhow::Result<()> {
    // Probe duration and keyframes
    let duration = probe_duration(&probe_url).await?;
    let keyframes = probe_keyframes(&probe_url).await?;
    let chunks = keyframe_aligned_chunks(duration, &keyframes);
    let total = chunks.len() as u32;

    tracing::info!("Job {job_id}: {duration:.1}s → {total} chunks");

    {
        let mut jobs = state.jobs.write().await;
        if let Some(j) = jobs.get_mut(&job_id) {
            j.status = JobStatus::Processing;
            j.chunk_count = Some(total);
            j.chunks_completed = 0;
            let _ = state.client_broadcast.send(ClientMessage::JobUpdate { job: j.clone() });
        }
    }

    // Submit one ColonyOS process per chunk
    let mut procs = Vec::with_capacity(chunks.len());
    let mut chunk_keys = Vec::with_capacity(chunks.len());

    for (i, (start, end)) in chunks.iter().enumerate() {
        let chunk_key = format!("{}/chunks/chunk_{:04}.{}", job_id, i, output_format);
        let mut spec = colonyos::core::FunctionSpec::new(
            "transcode_chunk",
            "ffmpeg-executor",
            &state.colony_name,
        );
        spec.kwargs.insert("input_key".into(), serde_json::Value::String(input_key.clone()));
        spec.kwargs.insert("start_time".into(), serde_json::json!(start));
        spec.kwargs.insert("duration".into(), serde_json::json!(end - start));
        spec.kwargs.insert("output_key".into(), serde_json::Value::String(chunk_key.clone()));
        spec.kwargs.insert("encoding".into(), serde_json::to_value(&encoding)?);
        spec.maxexectime = -1;

        let proc = colonyos::submit(&spec, &state.colony_prvkey)
            .await
            .map_err(|e| anyhow::anyhow!("submit chunk {i}: {e}"))?;

        procs.push(proc);
        chunk_keys.push(chunk_key);
    }

    // Subscribe to each chunk; update progress as they complete
    let completed = Arc::new(AtomicU32::new(0));
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<(), String>>(total as usize);

    for proc in procs {
        let prvkey = state.colony_prvkey.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let result = colonyos::subscribe_process(
                &proc,
                colonyos::core::SUCCESS,
                3600,
                &prvkey,
            )
            .await;
            let _ = tx.send(result.map(|_| ()).map_err(|e| e.to_string())).await;
        });
    }
    drop(tx);

    while let Some(result) = rx.recv().await {
        match result {
            Ok(()) => {
                let done = completed.fetch_add(1, Ordering::SeqCst) + 1;
                let mut jobs = state.jobs.write().await;
                if let Some(j) = jobs.get_mut(&job_id) {
                    j.chunks_completed = done;
                    // Reserve last 10% for the merge step
                    j.progress = Some(done as f32 / total as f32 * 0.9);
                    let _ = state.client_broadcast.send(ClientMessage::JobUpdate { job: j.clone() });
                }
            }
            Err(e) => return Err(anyhow::anyhow!("chunk failed: {e}")),
        }
    }

    // All chunks done — submit merge job
    let mut spec = colonyos::core::FunctionSpec::new(
        "merge_chunks",
        "ffmpeg-executor",
        &state.colony_name,
    );
    spec.kwargs.insert("chunk_keys".into(), serde_json::to_value(&chunk_keys)?);
    spec.kwargs.insert("output_key".into(), serde_json::Value::String(final_output_key.clone()));
    spec.kwargs.insert("output_format".into(), serde_json::Value::String(output_format.clone()));
    spec.maxexectime = -1;

    let merge_proc = colonyos::submit(&spec, &state.colony_prvkey)
        .await
        .map_err(|e| anyhow::anyhow!("submit merge: {e}"))?;

    colonyos::subscribe_process(
        &merge_proc,
        colonyos::core::SUCCESS,
        7200,
        &state.colony_prvkey,
    )
    .await
    .map_err(|e| anyhow::anyhow!("merge failed: {e}"))?;

    {
        let mut jobs = state.jobs.write().await;
        if let Some(j) = jobs.get_mut(&job_id) {
            j.status = JobStatus::Completed;
            j.progress = Some(1.0);
            let _ = state.client_broadcast.send(ClientMessage::JobUpdate { job: j.clone() });
        }
    }

    Ok(())
}

// ── ffprobe helpers ───────────────────────────────────────────────────────────

async fn probe_duration(url: &str) -> anyhow::Result<f64> {
    let out = Command::new("ffprobe")
        .args([
            "-v", "quiet",
            "-show_entries", "format=duration",
            "-of", "csv=p=0",
            url,
        ])
        .output()
        .await?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.trim()
        .parse::<f64>()
        .map_err(|_| anyhow::anyhow!("ffprobe returned unexpected duration: {:?}", s.trim()))
}

/// Returns sorted keyframe timestamps for the first video stream.
async fn probe_keyframes(url: &str) -> anyhow::Result<Vec<f64>> {
    let out = Command::new("ffprobe")
        .args([
            "-v", "quiet",
            "-select_streams", "v:0",
            "-show_packets",
            "-show_entries", "packet=pts_time,flags",
            "-of", "csv=print_section=0",
            url,
        ])
        .output()
        .await?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut kfs: Vec<f64> = text
        .lines()
        .filter(|l| l.contains(",K"))
        .filter_map(|l| l.split(',').next()?.parse().ok())
        .collect();
    kfs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Ok(kfs)
}

/// Split [0, duration] into segments whose boundaries snap to the nearest keyframe.
fn keyframe_aligned_chunks(duration: f64, keyframes: &[f64]) -> Vec<(f64, f64)> {
    let n = (duration / CHUNK_DURATION_SECS).ceil() as usize;
    if n <= 1 {
        return vec![(0.0, duration)];
    }

    let mut starts = vec![0.0_f64];
    for i in 1..n {
        let desired = i as f64 * CHUNK_DURATION_SECS;
        let nearest = keyframes
            .iter()
            .min_by(|a, b| {
                (*a - desired)
                    .abs()
                    .partial_cmp(&(*b - desired).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
            .unwrap_or(desired);
        starts.push(nearest);
    }

    starts
        .windows(2)
        .map(|w| (w[0], w[1]))
        .chain(std::iter::once((*starts.last().unwrap(), duration)))
        .collect()
}
