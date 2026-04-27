mod acp;
mod cli;
mod http;

use anyhow::Result;
use async_trait::async_trait;
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

pub use acp::{AcpAgent, AcpAgentConfig};
pub use cli::{CliAgent, CliAgentConfig};
pub use http::{HttpAgent, HttpAgentConfig};

#[derive(Debug, Clone, Default)]
pub struct AgentInfo {
    pub name: String,
    pub kind: String,
    pub model: String,
    pub command: String,
    pub pid: Option<u32>,
}

impl std::fmt::Display for AgentInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "name={}, type={}, model={}, command={}",
            self.name, self.kind, self.model, self.command
        )?;
        if let Some(pid) = self.pid {
            write!(f, ", pid={pid}")?;
        }
        Ok(())
    }
}

#[async_trait]
pub trait Agent: Send + Sync {
    async fn chat(&self, conversation_id: &str, message: &str) -> Result<String>;
    async fn reset_session(&self, conversation_id: &str) -> Result<Option<String>>;
    fn info(&self) -> AgentInfo;
    async fn set_cwd(&self, cwd: PathBuf);
}

pub type SharedAgent = Arc<dyn Agent>;

pub(crate) fn merge_env(extra: &HashMap<String, String>) -> Vec<(String, String)> {
    extra.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

#[derive(Debug, Clone)]
pub(crate) struct CwdCell(Arc<Mutex<PathBuf>>);

impl CwdCell {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self(Arc::new(Mutex::new(path)))
    }

    pub(crate) async fn get(&self) -> PathBuf {
        self.0.lock().await.clone()
    }

    pub(crate) async fn set(&self, path: PathBuf) {
        *self.0.lock().await = path;
    }
}
