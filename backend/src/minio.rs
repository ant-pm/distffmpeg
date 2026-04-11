use aws_config::Region;
use aws_credential_types::Credentials;
use aws_sdk_s3::config::Builder;
use aws_sdk_s3::Client;
use aws_sdk_s3::presigning::PresigningConfig;
use std::env;
use std::time::Duration;

fn make_client(endpoint: &str) -> Client {
    let access_key = env::var("MINIO_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".into());
    let secret_key = env::var("MINIO_SECRET_KEY").unwrap_or_else(|_| "minioadmin".into());
    let creds = Credentials::new(&access_key, &secret_key, None, None, "env");

    let config = Builder::new()
        .endpoint_url(endpoint)
        .region(Region::new("us-east-1"))
        .credentials_provider(creds)
        .force_path_style(true)
        .behavior_version_latest()
        .build();

    Client::from_conf(config)
}

/// Client whose presigned URLs use the public-facing endpoint (browser-accessible).
pub async fn create_public_client() -> Client {
    let endpoint = env::var("MINIO_PUBLIC_ENDPOINT")
        .unwrap_or_else(|_| env::var("MINIO_ENDPOINT").unwrap_or_else(|_| "http://localhost:9000".into()));
    make_client(&endpoint)
}

fn presign_config() -> PresigningConfig {
    PresigningConfig::builder()
        .expires_in(Duration::from_secs(3600))
        .build()
        .unwrap()
}

/// Presigned PUT URL using the public endpoint — for the browser to upload directly.
pub async fn presigned_put(public_client: &Client, bucket: &str, key: &str) -> String {
    public_client
        .put_object()
        .bucket(bucket)
        .key(key)
        .presigned(presign_config())
        .await
        .unwrap()
        .uri()
        .to_string()
}

/// Presigned GET URL using the public endpoint — for the browser to download.
pub async fn presigned_get(public_client: &Client, bucket: &str, key: &str) -> String {
    public_client
        .get_object()
        .bucket(bucket)
        .key(key)
        .presigned(presign_config())
        .await
        .unwrap()
        .uri()
        .to_string()
}

