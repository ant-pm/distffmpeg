use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use aws_sdk_s3::primitives::ByteStream;
use chrono::Utc;
use uuid::Uuid;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::process::Command;
use tokio::time::{sleep, Duration};

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
        started_at: None,
        finished_at: None,
        error: None,
        progress: None,
        chunk_count: None,
        chunks_completed: 0,
        chunks: vec![],
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

    let probe_url = minio::presigned_get(&state.s3_internal, "uploads", &input_key).await;
    let public_input_url = minio::presigned_get(&state.s3_public, "uploads", &input_key).await;

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
            public_input_url,
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
                j.finished_at = Some(Utc::now());
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
    _input_key: String,
    final_output_key: String,
    probe_url: String,
    public_input_url: String,
    output_format: String,
    encoding: EncodingOptions,
) -> anyhow::Result<()> {
    // Probe duration and keyframes
    let duration = probe_duration(&probe_url).await?;
    let keyframes = probe_keyframes(&probe_url).await?;
    let chunks = keyframe_aligned_chunks(duration, &keyframes);
    let total = chunks.len() as u32;

    tracing::info!("Job {job_id}: {duration:.1}s → {total} chunks");

    // Build executor ID → name map for worker attribution
    let executor_map: Arc<HashMap<String, String>> = Arc::new(
        colonyos::get_executors(&state.colony_name, &state.colony_prvkey)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|e| (e.executorid, e.executorname))
            .collect(),
    );

    {
        let mut jobs = state.jobs.write().await;
        if let Some(j) = jobs.get_mut(&job_id) {
            j.status = JobStatus::Processing;
            j.chunk_count = Some(total);
            j.chunks_completed = 0;
            j.started_at = Some(Utc::now());
            j.chunks = (0..total)
                .map(|i| ChunkInfo { index: i, status: ChunkStatus::Queued, worker: None })
                .collect();
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
        spec.kwargs.insert("input_url".into(), serde_json::Value::String(public_input_url.clone()));
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

    // Spawn per-chunk pollers — they update chunk status, worker, and progress
    let completed = Arc::new(AtomicU32::new(0));
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<(), String>>(total as usize);

    for (chunk_idx, proc) in procs.into_iter().enumerate() {
        let tx = tx.clone();
        let state_clone = state.clone();
        let prvkey = state.colony_prvkey.clone();
        let completed_clone = completed.clone();
        let executor_map_clone = executor_map.clone();
        tokio::spawn(async move {
            let result = poll_chunk(
                proc.processid,
                chunk_idx as u32,
                3600,
                prvkey,
                executor_map_clone,
                state_clone,
                job_id,
                total,
                completed_clone,
            )
            .await;
            let _ = tx.send(result.map_err(|e| e.to_string())).await;
        });
    }
    drop(tx);

    while let Some(result) = rx.recv().await {
        if let Err(e) = result {
            return Err(anyhow::anyhow!("chunk failed: {e}"));
        }
    }

    // Merge locally — backend has direct internal MinIO access so there is no
    // Cloudflare upload-size limit and no extra network hops.
    merge_chunks_local(&state, &chunk_keys, &final_output_key, &output_format).await
        .map_err(|e| anyhow::anyhow!("merge failed: {e}"))?;

    {
        let mut jobs = state.jobs.write().await;
        if let Some(j) = jobs.get_mut(&job_id) {
            j.status = JobStatus::Completed;
            j.progress = Some(1.0);
            j.finished_at = Some(Utc::now());
            let _ = state.client_broadcast.send(ClientMessage::JobUpdate { job: j.clone() });
        }
    }

    Ok(())
}

// ── ColonyOS polling ──────────────────────────────────────────────────────────

/// Polls a single chunk process, updating chunk status and worker attribution in job state.
async fn poll_chunk(
    processid: String,
    chunk_idx: u32,
    timeout_secs: u64,
    prvkey: String,
    executor_map: Arc<HashMap<String, String>>,
    state: AppState,
    job_id: Uuid,
    total: u32,
    completed: Arc<AtomicU32>,
) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    let mut last_status = ChunkStatus::Queued;
    let mut last_worker: Option<String> = None;

    loop {
        if tokio::time::Instant::now() > deadline {
            return Err(anyhow::anyhow!("timed out after {timeout_secs}s"));
        }

        let proc = colonyos::get_process(&processid, &prvkey)
            .await
            .map_err(|e| anyhow::anyhow!("get_process: {e}"))?;

        let worker = if proc.isassigned && !proc.assignedexecutorid.is_empty() {
            Some(
                executor_map
                    .get(&proc.assignedexecutorid)
                    .cloned()
                    .unwrap_or_else(|| proc.assignedexecutorid[..proc.assignedexecutorid.len().min(8)].to_string()),
            )
        } else {
            None
        };

        let chunk_status = match proc.state {
            colonyos::core::RUNNING  => ChunkStatus::Running,
            colonyos::core::SUCCESS  => ChunkStatus::Done,
            colonyos::core::FAILED   => ChunkStatus::Failed,
            _                        => ChunkStatus::Queued,
        };

        if chunk_status != last_status || worker != last_worker {
            last_status = chunk_status.clone();
            last_worker = worker.clone();

            let mut jobs = state.jobs.write().await;
            if let Some(j) = jobs.get_mut(&job_id) {
                if let Some(c) = j.chunks.get_mut(chunk_idx as usize) {
                    c.status = chunk_status.clone();
                    if worker.is_some() { c.worker = worker; }
                }
                if chunk_status == ChunkStatus::Done {
                    let done = completed.fetch_add(1, Ordering::SeqCst) + 1;
                    j.chunks_completed = done;
                    j.progress = Some(done as f32 / total as f32 * 0.9);
                }
                let _ = state.client_broadcast.send(ClientMessage::JobUpdate { job: j.clone() });
            }
        }

        match proc.state {
            colonyos::core::SUCCESS => return Ok(()),
            colonyos::core::FAILED => {
                let reason = proc.errors.into_iter().next().unwrap_or_else(|| "unknown".into());
                return Err(anyhow::anyhow!("{reason}"));
            }
            _ => {}
        }

        sleep(Duration::from_secs(5)).await;
    }
}

/// Merge already-transcoded chunks by running ffmpeg concat locally in the backend.
/// The backend has direct internal MinIO access so the upload is not limited by Cloudflare.
async fn merge_chunks_local(
    state: &AppState,
    chunk_keys: &[String],
    final_output_key: &str,
    output_format: &str,
) -> anyhow::Result<()> {
    // Build presigned GET URLs using the internal client so ffmpeg can reach MinIO
    // directly without going through Cloudflare.
    let mut chunk_urls = Vec::with_capacity(chunk_keys.len());
    for key in chunk_keys {
        chunk_urls.push(minio::presigned_get(&state.s3_internal, "outputs", key).await);
    }

    let tmp_dir = std::env::temp_dir().join(format!("ffmerge-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&tmp_dir).await?;

    let manifest_path = tmp_dir.join("list.txt");
    let manifest = chunk_urls.iter()
        .map(|url| format!("file '{}'\n", url))
        .collect::<String>();
    tokio::fs::write(&manifest_path, &manifest).await?;

    let output_path = tmp_dir.join(format!("output.{output_format}"));
    let out = Command::new("ffmpeg")
        .args([
            "-f", "concat",
            "-safe", "0",
            "-protocol_whitelist", "file,http,https,tcp,tls,crypto",
            "-i", manifest_path.to_str().unwrap(),
            "-c", "copy",
            "-y",
            output_path.to_str().unwrap(),
        ])
        .output()
        .await?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // Clean up before returning
        let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
        return Err(anyhow::anyhow!("ffmpeg: {}", stderr.lines()
            .filter(|l| l.contains("Error") || l.contains("error") || l.contains("Invalid") || l.contains("No such"))
            .collect::<Vec<_>>()
            .join("; ")
            .chars().take(500).collect::<String>()));
    }

    let bytes = ByteStream::from_path(&output_path).await
        .map_err(|e| anyhow::anyhow!("read merged file: {e}"))?;

    state.s3_internal
        .put_object()
        .bucket("outputs")
        .key(final_output_key)
        .body(bytes)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("upload merged file: {e}"))?;

    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
    Ok(())
}

// ── ffprobe helpers ───────────────────────────────────────────────────────────

async fn probe_duration(url: &str) -> anyhow::Result<f64> {
    let out = Command::new("ffprobe")
        .args(["-v", "quiet", "-show_entries", "format=duration", "-of", "csv=p=0", url])
        .output()
        .await?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.trim()
        .parse::<f64>()
        .map_err(|_| anyhow::anyhow!("ffprobe returned unexpected duration: {:?}", s.trim()))
}

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
