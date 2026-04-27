use super::{merge_env, Agent, AgentInfo, CwdCell};
use crate::config::default_workspace;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::Mutex,
};

#[derive(Debug, Clone)]
pub struct CliAgentConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    pub model: String,
    pub system_prompt: String,
}

pub struct CliAgent {
    cfg: CliAgentConfig,
    cwd: CwdCell,
    sessions: Arc<Mutex<HashMap<String, String>>>,
}

#[derive(Debug, Deserialize)]
struct StreamEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    result: String,
    #[serde(default)]
    is_error: bool,
    message: Option<StreamMessage>,
}

#[derive(Debug, Deserialize)]
struct StreamMessage {
    #[serde(default)]
    content: Vec<StreamContent>,
}

#[derive(Debug, Deserialize)]
struct StreamContent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

impl CliAgent {
    pub fn new(mut cfg: CliAgentConfig) -> Self {
        if cfg.cwd.as_os_str().is_empty() {
            cfg.cwd = default_workspace();
        }
        Self {
            cwd: CwdCell::new(cfg.cwd.clone()),
            cfg,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn chat_claude(&self, conversation_id: &str, message: &str) -> Result<String> {
        let mut args = vec![
            "-p".to_string(),
            message.to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
        ];
        if !self.cfg.model.is_empty() {
            args.extend(["--model".to_string(), self.cfg.model.clone()]);
        }
        if !self.cfg.system_prompt.is_empty() {
            args.extend([
                "--append-system-prompt".to_string(),
                self.cfg.system_prompt.clone(),
            ]);
        }
        args.extend(self.cfg.args.clone());

        if let Some(session_id) = self.sessions.lock().await.get(conversation_id).cloned() {
            args.extend(["--resume".to_string(), session_id]);
        }

        let mut cmd = Command::new(&self.cfg.command);
        cmd.args(args)
            .current_dir(self.cwd.get().await)
            .stderr(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());
        for (k, v) in merge_env(&self.cfg.env) {
            cmd.env(k, v);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("start {}", self.cfg.name))?;
        let stdout = child.stdout.take().context("create stdout pipe")?;
        let stderr = child.stderr.take();
        let mut lines = BufReader::new(stdout).lines();
        let stderr_task = stderr.map(|s| {
            tokio::spawn(async move {
                let mut lines = BufReader::new(s).lines();
                let mut out = String::new();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&line);
                }
                out
            })
        });

        let mut result = String::new();
        let mut new_session = String::new();
        let mut assistant_texts = Vec::new();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(event) = serde_json::from_str::<StreamEvent>(&line) else {
                continue;
            };
            if !event.session_id.is_empty() {
                new_session = event.session_id;
            }
            match event.kind.as_str() {
                "result" => {
                    if event.is_error {
                        anyhow::bail!("{} returned error: {}", self.cfg.name, event.result);
                    }
                    result = event.result;
                }
                "assistant" => {
                    if let Some(message) = event.message {
                        for block in message.content {
                            if block.kind == "text" && !block.text.is_empty() {
                                assistant_texts.push(block.text);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let status = child.wait().await?;
        let stderr = match stderr_task {
            Some(task) => task.await.unwrap_or_default(),
            None => String::new(),
        };
        if result.is_empty() && !assistant_texts.is_empty() {
            result = assistant_texts.join("");
        }
        if !status.success() && result.is_empty() {
            let detail = stderr.trim();
            if detail.is_empty() {
                anyhow::bail!("{} exited with {}", self.cfg.name, status);
            }
            anyhow::bail!("{} exited with {}: {}", self.cfg.name, status, detail);
        }
        if !new_session.is_empty() {
            self.sessions
                .lock()
                .await
                .insert(conversation_id.to_string(), new_session);
        }
        let result = result.trim().to_string();
        if result.is_empty() {
            anyhow::bail!("{} returned empty response", self.cfg.name);
        }
        Ok(result)
    }

    async fn chat_codex(&self, message: &str) -> Result<String> {
        let mut args = vec!["exec".to_string(), message.to_string()];
        if !self.cfg.model.is_empty() {
            args.extend(["--model".to_string(), self.cfg.model.clone()]);
        }
        args.extend(self.cfg.args.clone());

        let mut cmd = Command::new(&self.cfg.command);
        cmd.args(args)
            .current_dir(self.cwd.get().await)
            .stderr(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());
        for (k, v) in merge_env(&self.cfg.env) {
            cmd.env(k, v);
        }
        let out = cmd.output().await.context("codex exec")?;
        if !out.status.success() {
            anyhow::bail!(
                "codex error: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let result = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if result.is_empty() {
            anyhow::bail!("codex returned empty response");
        }
        Ok(result)
    }
}

#[async_trait]
impl Agent for CliAgent {
    async fn chat(&self, conversation_id: &str, message: &str) -> Result<String> {
        if self.cfg.name == "codex" {
            self.chat_codex(message).await
        } else {
            self.chat_claude(conversation_id, message).await
        }
    }

    async fn reset_session(&self, conversation_id: &str) -> Result<Option<String>> {
        self.sessions.lock().await.remove(conversation_id);
        Ok(None)
    }

    fn info(&self) -> AgentInfo {
        AgentInfo {
            name: self.cfg.name.clone(),
            kind: "cli".to_string(),
            model: self.cfg.model.clone(),
            command: self.cfg.command.clone(),
            pid: None,
        }
    }

    async fn set_cwd(&self, cwd: PathBuf) {
        self.cwd.set(cwd).await;
    }
}
