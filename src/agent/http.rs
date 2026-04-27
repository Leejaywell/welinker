use super::{Agent, AgentInfo};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct HttpAgentConfig {
    pub endpoint: String,
    pub api_key: String,
    pub headers: HashMap<String, String>,
    pub model: String,
    pub system_prompt: String,
    pub max_history: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

pub struct HttpAgent {
    cfg: HttpAgentConfig,
    client: Client,
    history: Arc<Mutex<HashMap<String, Vec<ChatMessage>>>>,
}

impl HttpAgent {
    pub fn new(mut cfg: HttpAgentConfig) -> Self {
        if cfg.model.is_empty() {
            cfg.model = "gpt-4o-mini".to_string();
        }
        if cfg.max_history == 0 {
            cfg.max_history = 20;
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("reqwest client");
        Self {
            cfg,
            client,
            history: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn build_messages(&self, conversation_id: &str, message: &str) -> Vec<ChatMessage> {
        let mut messages = Vec::new();
        if !self.cfg.system_prompt.is_empty() {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: self.cfg.system_prompt.clone(),
            });
        }
        if let Some(hist) = self.history.lock().await.get(conversation_id) {
            messages.extend(hist.clone());
        }
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: message.to_string(),
        });
        messages
    }
}

#[async_trait]
impl Agent for HttpAgent {
    async fn chat(&self, conversation_id: &str, message: &str) -> Result<String> {
        let messages = self.build_messages(conversation_id, message).await;
        let body = serde_json::json!({
            "model": self.cfg.model,
            "messages": messages,
        });
        let mut req = self
            .client
            .post(&self.cfg.endpoint)
            .header("Content-Type", "application/json")
            .json(&body);
        if !self.cfg.api_key.is_empty() {
            req = req.bearer_auth(&self.cfg.api_key);
        }
        for (key, value) in &self.cfg.headers {
            req = req.header(key, value);
        }

        let resp = req.send().await.context("HTTP request")?;
        let status = resp.status();
        let bytes = resp.bytes().await.context("read response")?;
        if !status.is_success() {
            anyhow::bail!(
                "API error HTTP {}: {}",
                status.as_u16(),
                String::from_utf8_lossy(&bytes)
            );
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes).context("parse response")?;
        let reply = value["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if reply.is_empty() {
            anyhow::bail!("no choices in response");
        }

        let mut history = self.history.lock().await;
        let entry = history.entry(conversation_id.to_string()).or_default();
        entry.push(ChatMessage {
            role: "user".to_string(),
            content: message.to_string(),
        });
        entry.push(ChatMessage {
            role: "assistant".to_string(),
            content: reply.clone(),
        });
        let max = self.cfg.max_history * 2;
        if entry.len() > max {
            let drop = entry.len() - max;
            entry.drain(0..drop);
        }

        Ok(reply)
    }

    async fn reset_session(&self, conversation_id: &str) -> Result<Option<String>> {
        self.history.lock().await.remove(conversation_id);
        Ok(None)
    }

    fn info(&self) -> AgentInfo {
        AgentInfo {
            name: "http".to_string(),
            kind: "http".to_string(),
            model: self.cfg.model.clone(),
            command: self.cfg.endpoint.clone(),
            pid: None,
        }
    }

    async fn set_cwd(&self, _cwd: PathBuf) {}
}
