use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    PendingUpload,
    Queued,
    Processing,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    H264,
    H265,
    Av1,
    Vp9,
    Copy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    Aac,
    Opus,
    Flac,
    Mp3,
    Copy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Ultrafast,
    Fast,
    Medium,
    Slow,
    Veryslow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodingOptions {
    pub video_codec: VideoCodec,
    pub audio_codec: AudioCodec,
    pub preset: Preset,
    /// Constant Rate Factor (0-51 for x264/x265, 0-63 for AV1/VP9). Lower = better quality, bigger file.
    pub crf: u8,
    /// Target resolution, e.g. "1920x1080", "1280x720". None keeps original.
    pub resolution: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub filename: String,
    pub output_format: String,
    pub encoding: EncodingOptions,
    pub status: JobStatus,
    pub created_at: DateTime<Utc>,
    pub error: Option<String>,
    pub progress: Option<f32>,
    pub chunk_count: Option<u32>,
    pub chunks_completed: u32,
}

#[derive(Debug, Deserialize)]
pub struct CreateJobRequest {
    pub filename: String,
    pub output_format: String,
    pub encoding: EncodingOptions,
}

#[derive(Debug, Serialize)]
pub struct CreateJobResponse {
    pub job: Job,
    pub upload_url: String,
}

#[derive(Debug, Serialize)]
pub struct DownloadResponse {
    pub download_url: String,
}

/// Messages sent from backend to client dashboard
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "job_update")]
    JobUpdate { job: Job },
    #[serde(rename = "jobs_list")]
    JobsList { jobs: Vec<Job> },
}
