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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ChunkStatus {
    Queued,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkInfo {
    pub index: u32,
    pub status: ChunkStatus,
    pub worker: Option<String>,
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
    pub crf: u8,
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
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub progress: Option<f32>,
    pub chunk_count: Option<u32>,
    pub chunks_completed: u32,
    pub chunks: Vec<ChunkInfo>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "job_update")]
    JobUpdate { job: Job },
    #[serde(rename = "jobs_list")]
    JobsList { jobs: Vec<Job> },
}
