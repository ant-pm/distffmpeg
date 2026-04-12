use anyhow::{anyhow, Result};
use aws_credential_types::Credentials;
use aws_sdk_s3::{Client, Config, config::Region, primitives::ByteStream};
use colonyos::core::{Executor, Software};
use serde::Deserialize;
use tempfile::tempdir;
use tokio::{process::Command, signal};

// ── Job types ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TranscodeChunkJob {
    input_key: String,
    start_time: f64,
    duration: f64,
    output_key: String,
    encoding: EncodingOptions,
}

#[derive(Debug, Deserialize)]
struct MergeChunksJob {
    chunk_keys: Vec<String>,
    output_key: String,
    output_format: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum VideoCodec { H264, H265, Av1, Vp9, Copy }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AudioCodec { Aac, Opus, Flac, Mp3, Copy }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Preset { Ultrafast, Fast, Medium, Slow, Veryslow }

#[derive(Debug, Deserialize)]
struct EncodingOptions {
    video_codec: VideoCodec,
    audio_codec: AudioCodec,
    preset: Preset,
    crf: u8,
    resolution: Option<String>,
}

// ── MinIO ─────────────────────────────────────────────────────────────────────

fn minio_client() -> Client {
    let endpoint = std::env::var("MINIO_ENDPOINT").expect("MINIO_ENDPOINT not set");
    let access_key = std::env::var("MINIO_ACCESS_KEY").expect("MINIO_ACCESS_KEY not set");
    let secret_key = std::env::var("MINIO_SECRET_KEY").expect("MINIO_SECRET_KEY not set");
    let creds = Credentials::new(access_key, secret_key, None, None, "static");
    let config = Config::builder()
        .endpoint_url(endpoint)
        .credentials_provider(creds)
        .region(Region::new("us-east-1"))
        .force_path_style(true)
        .build();
    Client::from_conf(config)
}

// ── Executor lifecycle ────────────────────────────────────────────────────────

pub async fn run_executor() -> Result<()> {
    let colony_name = std::env::var("COLONY").expect("COLONY not set");
    let prvkey = std::env::var("COLONY_PRIVATE_KEY").expect("COLONY_PRIVATE_KEY not set");
    let exec_prvkey = colonyos::crypto::gen_prvkey();
    let executor_id = colonyos::crypto::gen_id(&exec_prvkey);
    let executor_name = format!("ffmpeg-executor-{}", hostname::get()?.to_string_lossy());

    let mut executor =
        Executor::new(&executor_name, &executor_id, "ffmpeg-executor", &colony_name);
    executor.capabilities.software.push(Software {
        name: "ffmpeg".into(),
        software_type: "transcoder".into(),
        version: String::new(),
    });

    colonyos::add_executor(&executor, &prvkey).await?;
    colonyos::approve_executor(&colony_name, &executor_name, &prvkey).await?;
    tracing::info!("Executor registered: {executor_name}");

    let result = run_loop(&colony_name, &exec_prvkey).await;
    colonyos::remove_executor(&colony_name, &executor_name, &prvkey).await?;
    result
}

// ── Event loop ────────────────────────────────────────────────────────────────

async fn run_loop(colony_name: &str, exec_prvkey: &str) -> Result<()> {
    let s3 = minio_client();
    let input_bucket = std::env::var("MINIO_INPUT_BUCKET").unwrap_or_else(|_| "uploads".into());
    let output_bucket = std::env::var("MINIO_OUTPUT_BUCKET").unwrap_or_else(|_| "outputs".into());

    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                tracing::info!("Shutting down...");
                return Ok(());
            }
            result = colonyos::assign(colony_name, 10, exec_prvkey) => {
                match result {
                    Err(e) if !e.conn_err() => continue,
                    Err(e) => {
                        tracing::error!("Connection error: {e}");
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    }
                    Ok(process) => {
                        tracing::info!("Assigned: {} ({})", process.processid, process.spec.funcname);
                        let s3 = s3.clone();
                        let input_bucket = input_bucket.clone();
                        let output_bucket = output_bucket.clone();
                        let exec_prvkey = exec_prvkey.to_string();
                        tokio::spawn(async move {
                            let kwargs = serde_json::to_value(&process.spec.kwargs).unwrap();
                            let res = match process.spec.funcname.as_str() {
                                "transcode_chunk" => {
                                    handle_transcode_chunk(&s3, &input_bucket, &output_bucket, &kwargs).await
                                }
                                "merge_chunks" => {
                                    handle_merge_chunks(&s3, &output_bucket, &kwargs).await
                                }
                                other => Err(anyhow!("unknown function: {other}")),
                            };
                            match res {
                                Ok(output_key) => {
                                    if let Err(e) = colonyos::set_output(
                                        &process.processid,
                                        vec![output_key],
                                        &exec_prvkey,
                                    ).await {
                                        tracing::error!("set_output failed: {e}");
                                    } else if let Err(e) =
                                        colonyos::close(&process.processid, &exec_prvkey).await
                                    {
                                        tracing::error!("close failed: {e}");
                                    } else {
                                        tracing::info!("Process {} done", process.processid);
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Process {} failed: {e:#}", process.processid);
                                    let _ = colonyos::fail(&process.processid, &exec_prvkey).await;
                                }
                            }
                        });
                    }
                }
            }
        }
    }
}

// ── Chunk transcoding ─────────────────────────────────────────────────────────

async fn handle_transcode_chunk(
    s3: &Client,
    input_bucket: &str,
    output_bucket: &str,
    kwargs: &serde_json::Value,
) -> Result<String> {
    let job: TranscodeChunkJob = serde_json::from_value(kwargs.clone())?;

    let input_ext = job.input_key.rsplit('.').next().unwrap_or("mp4");
    let output_ext = job.output_key.rsplit('.').next().unwrap_or("mp4");

    let tmp = tempdir()?;
    let input_path = tmp.path().join(format!("input.{input_ext}"));
    let output_path = tmp.path().join(format!("output.{output_ext}"));

    // Download input from MinIO
    let resp = s3.get_object().bucket(input_bucket).key(&job.input_key).send().await?;
    let bytes = resp.body.collect().await?.into_bytes();
    tokio::fs::write(&input_path, bytes).await?;

    // Seek to keyframe-aligned start before -i, then limit output duration with -t.
    // -avoid_negative_ts make_zero resets timestamps so the chunk starts at 0.
    let mut args = vec![
        "-ss".into(), format!("{:.6}", job.start_time),
        "-i".into(), input_path.to_str().unwrap().to_string(),
        "-t".into(), format!("{:.6}", job.duration),
        "-y".into(),
    ];
    args.extend(build_encoding_flags(&job.encoding));
    args.extend([
        "-avoid_negative_ts".into(), "make_zero".into(),
        output_path.to_str().unwrap().to_string(),
    ]);

    tracing::info!("Chunk {}: ffmpeg {}", job.output_key, args.join(" "));

    let out = Command::new("ffmpeg").args(&args).output().await?;
    if !out.status.success() {
        return Err(anyhow!("{}", extract_ffmpeg_errors(&String::from_utf8_lossy(&out.stderr))));
    }

    // Upload chunk
    let out_bytes = tokio::fs::read(&output_path).await?;
    s3.put_object()
        .bucket(output_bucket)
        .key(&job.output_key)
        .body(ByteStream::from(out_bytes))
        .send()
        .await?;

    Ok(job.output_key)
}

// ── Chunk merging ─────────────────────────────────────────────────────────────

async fn handle_merge_chunks(
    s3: &Client,
    output_bucket: &str,
    kwargs: &serde_json::Value,
) -> Result<String> {
    let job: MergeChunksJob = serde_json::from_value(kwargs.clone())?;

    let tmp = tempdir()?;
    let mut chunk_paths = Vec::with_capacity(job.chunk_keys.len());

    // Download all chunks
    for (i, chunk_key) in job.chunk_keys.iter().enumerate() {
        let resp = s3.get_object().bucket(output_bucket).key(chunk_key).send().await?;
        let bytes = resp.body.collect().await?.into_bytes();
        let path = tmp.path().join(format!("chunk_{i:04}.{}", job.output_format));
        tokio::fs::write(&path, &bytes).await?;
        chunk_paths.push(path);
    }

    // Write concat manifest (absolute paths, safe=0 allows them)
    let manifest_path = tmp.path().join("list.txt");
    let manifest = chunk_paths
        .iter()
        .map(|p| format!("file '{}'\n", p.to_str().unwrap()))
        .collect::<String>();
    tokio::fs::write(&manifest_path, manifest).await?;

    // Concat with stream copy — chunks are already transcoded
    let output_path = tmp.path().join(format!("output.{}", job.output_format));
    let out = Command::new("ffmpeg")
        .args([
            "-f", "concat",
            "-safe", "0",
            "-i", manifest_path.to_str().unwrap(),
            "-c", "copy",
            "-y",
            output_path.to_str().unwrap(),
        ])
        .output()
        .await?;

    if !out.status.success() {
        return Err(anyhow!("{}", extract_ffmpeg_errors(&String::from_utf8_lossy(&out.stderr))));
    }

    // Upload final output
    let out_bytes = tokio::fs::read(&output_path).await?;
    s3.put_object()
        .bucket(output_bucket)
        .key(&job.output_key)
        .body(ByteStream::from(out_bytes))
        .send()
        .await?;

    // Clean up intermediate chunks from MinIO
    for chunk_key in &job.chunk_keys {
        let _ = s3.delete_object()
            .bucket(output_bucket)
            .key(chunk_key)
            .send()
            .await;
    }

    Ok(job.output_key)
}

// ── FFmpeg helpers ────────────────────────────────────────────────────────────

fn build_encoding_flags(enc: &EncodingOptions) -> Vec<String> {
    let mut args = Vec::new();

    match &enc.video_codec {
        VideoCodec::H264 => args.extend([
            "-c:v".into(), "libx264".into(),
            "-preset".into(), preset_str(&enc.preset).into(),
            "-crf".into(), enc.crf.to_string(),
        ]),
        VideoCodec::H265 => args.extend([
            "-c:v".into(), "libx265".into(),
            "-preset".into(), preset_str(&enc.preset).into(),
            "-crf".into(), enc.crf.to_string(),
        ]),
        VideoCodec::Av1 => args.extend([
            "-c:v".into(), "libsvtav1".into(),
            "-preset".into(), av1_preset(&enc.preset).into(),
            "-crf".into(), enc.crf.to_string(),
        ]),
        VideoCodec::Vp9 => args.extend([
            "-c:v".into(), "libvpx-vp9".into(),
            "-crf".into(), enc.crf.to_string(),
            "-b:v".into(), "0".into(),
            "-speed".into(), vp9_speed(&enc.preset).into(),
            "-row-mt".into(), "1".into(),
        ]),
        VideoCodec::Copy => args.extend(["-c:v".into(), "copy".into()]),
    }

    match &enc.audio_codec {
        AudioCodec::Aac  => args.extend(["-c:a".into(), "aac".into(),        "-b:a".into(), "192k".into()]),
        AudioCodec::Opus => args.extend(["-c:a".into(), "libopus".into(),    "-b:a".into(), "128k".into()]),
        AudioCodec::Flac => args.extend(["-c:a".into(), "flac".into()]),
        AudioCodec::Mp3  => args.extend(["-c:a".into(), "libmp3lame".into(), "-b:a".into(), "192k".into()]),
        AudioCodec::Copy => args.extend(["-c:a".into(), "copy".into()]),
    }

    if let Some(ref res) = enc.resolution {
        args.extend(["-vf".into(), format!("scale={res}")]);
    }

    args
}

fn preset_str(p: &Preset) -> &'static str {
    match p {
        Preset::Ultrafast => "ultrafast",
        Preset::Fast      => "fast",
        Preset::Medium    => "medium",
        Preset::Slow      => "slow",
        Preset::Veryslow  => "veryslow",
    }
}

fn av1_preset(p: &Preset) -> &'static str {
    match p {
        Preset::Ultrafast => "12",
        Preset::Fast      => "8",
        Preset::Medium    => "6",
        Preset::Slow      => "4",
        Preset::Veryslow  => "2",
    }
}

fn vp9_speed(p: &Preset) -> &'static str {
    match p {
        Preset::Ultrafast => "5",
        Preset::Fast      => "4",
        Preset::Medium    => "2",
        Preset::Slow      => "1",
        Preset::Veryslow  => "0",
    }
}

// ── Error extraction ──────────────────────────────────────────────────────────

fn extract_ffmpeg_errors(stderr: &str) -> String {
    let is_noise = |line: &str| -> bool {
        if line.starts_with("ffmpeg version") { return true; }
        if line.starts_with("  built with")
            || line.starts_with("  configuration:")
            || line.starts_with("  lib")
        { return true; }
        if line.starts_with("Input #")
            || line.starts_with("Output #")
            || line.starts_with("  Duration:")
            || line.starts_with("  Metadata:")
        { return true; }
        if line.starts_with("    ") { return true; }
        if line.starts_with("  Stream #") || line.starts_with("Stream mapping:") { return true; }
        if line.starts_with("x265 [info]") { return true; }
        if line.starts_with("Press [q]")
            || line.starts_with("frame=")
            || line.starts_with("encoded ")
        { return true; }
        line.trim().is_empty()
    };

    let clean = |line: &str| -> String {
        let mut out = String::with_capacity(line.len());
        let mut rest = line;
        while let Some(at) = rest.find(" @ 0x") {
            out.push_str(&rest[..at]);
            let hex_start = at + " @ 0x".len();
            let hex_len = rest[hex_start..]
                .find(|c: char| !c.is_ascii_hexdigit())
                .unwrap_or(rest[hex_start..].len());
            rest = &rest[hex_start + hex_len..];
        }
        out.push_str(rest);
        out
    };

    let lines: Vec<String> = stderr
        .lines()
        .filter(|l| !is_noise(l))
        .map(|l| clean(l))
        .collect();

    if lines.is_empty() {
        stderr
            .lines()
            .filter(|l| !l.trim().is_empty())
            .last()
            .unwrap_or("unknown error")
            .to_string()
    } else {
        lines.join("\n")
    }
}
