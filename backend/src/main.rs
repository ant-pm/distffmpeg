mod minio;
mod state;
mod routes;
mod ws;
mod models;

use axum::{Router, routing::{get, post}};
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let colony_host = std::env::var("COLONY_HOST").expect("COLONY_HOST not set");
    colonyos::set_server_url(&format!("https://{colony_host}/api"));

    let s3_public = minio::create_public_client().await;
    let app_state = state::AppState::new(s3_public);

    let app = Router::new()
        .route("/api/jobs", post(routes::create_job))
        .route("/api/jobs", get(routes::list_jobs))
        .route("/api/jobs/{id}/uploaded", post(routes::mark_uploaded))
        .route("/api/jobs/{id}/download", get(routes::download_url))
        .route("/ws/client", get(ws::client_ws_handler))
        .layer(CorsLayer::permissive())
        .with_state(app_state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    tracing::info!("Backend listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
