mod executor;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let colony_host = std::env::var("COLONY_HOST").expect("COLONY_HOST not set");
    colonyos::set_server_url(&format!("https://{colony_host}/api"));

    executor::run_executor().await
}
