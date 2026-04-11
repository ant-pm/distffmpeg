use crate::models::{ClientMessage, Job};
use aws_sdk_s3::Client;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    /// S3 client whose presigned URLs use the public-facing endpoint (browser-accessible).
    pub s3_public: Client,
    pub jobs: Arc<RwLock<HashMap<Uuid, Job>>>,
    pub client_broadcast: broadcast::Sender<ClientMessage>,
    pub colony_name: String,
    pub colony_prvkey: String,
}

impl AppState {
    pub fn new(s3_public: Client) -> Self {
        let (client_broadcast, _) = broadcast::channel(256);
        Self {
            s3_public,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            client_broadcast,
            colony_name: std::env::var("COLONY").expect("COLONY not set"),
            colony_prvkey: std::env::var("COLONY_PRIVATE_KEY").expect("COLONY_PRIVATE_KEY not set"),
        }
    }
}
