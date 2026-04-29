use std::sync::Arc;

use crate::Config;
use crate::stream::Orchestrator;

pub struct AppState {
    pub config: Arc<Config>,
    pub orch: Arc<Orchestrator>,
}
