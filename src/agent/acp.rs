use super::{merge_env, Agent, AgentInfo, CwdCell};
use crate::config::default_workspace;
use anyhow::{Context, Result};
use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{mpsc, oneshot, Mutex},
};

#[derive(Debug, Clone)]
pub struct AcpAgentConfig {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    pub model: String,
    #[allow(dead_code)]
    pub system_prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Protocol {
    LegacyAcp,
    CodexAppServer,
    ZeroClawAcp,
}

pub struct AcpAgent {
    cfg: AcpAgentConfig,
    cwd: CwdCell,
    protocol: Protocol,
    state: Arc<Mutex<State>>,
    next_id: AtomicI64,
    sessions: Arc<Mutex<HashMap<String, String>>>,
    threads: Arc<Mutex<HashMap<String, String>>>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<RpcMessage>>>>,
    session_channels: Arc<Mutex<HashMap<String, mpsc::Sender<SessionUpdate>>>>,
    turn_channels: Arc<Mutex<HashMap<String, mpsc::Sender<CodexTurnEvent>>>>,
}

#[derive(Default)]
struct State {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    pid: Option<u32>,
    started: bool,
}

#[derive(Debug, Deserialize, Clone)]
struct RpcMessage {
    id: Option<i64>,
    method: Option<String>,
    #[serde(default)]
    result: Value,
    #[serde(default)]
    params: Value,
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize, Clone)]
struct RpcError {
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionUpdateParams {
    session_id: String,
    update: SessionUpdate,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SessionUpdate {
    #[serde(default)]
    session_update: String,
    #[serde(default)]
    content: Value,
    #[serde(default)]
    text: String,
}

#[derive(Debug, Clone)]
struct CodexTurnEvent {
    kind: String,
    delta: String,
    text: String,
}

impl AcpAgent {
    pub fn new(mut cfg: AcpAgentConfig) -> Self {
        if cfg.command.is_empty() {
            cfg.command = "claude-agent-acp".to_string();
        }
        if cfg.cwd.as_os_str().is_empty() {
            cfg.cwd = default_workspace();
        }
        let protocol = detect_protocol(&cfg.command, &cfg.args);
        Self {
            cwd: CwdCell::new(cfg.cwd.clone()),
            cfg,
            protocol,
            state: Arc::new(Mutex::new(State::default())),
            next_id: AtomicI64::new(0),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            threads: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            session_channels: Arc::new(Mutex::new(HashMap::new())),
            turn_channels: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn start(&self) -> Result<()> {
        {
            let state = self.state.lock().await;
            if state.started {
                return Ok(());
            }
        }

        let mut cmd = Command::new(&self.cfg.command);
        cmd.args(&self.cfg.args)
            .current_dir(self.cwd.get().await)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut env = self.cfg.env.clone();
        if self.protocol == Protocol::ZeroClawAcp {
            env.insert("NO_COLOR".to_string(), "1".to_string());
            env.insert("CLICOLOR".to_string(), "0".to_string());
            env.insert("CLICOLOR_FORCE".to_string(), "0".to_string());
            env.insert("RUST_LOG".to_string(), "off".to_string());
        }
        for (k, v) in merge_env(&env) {
            cmd.env(k, v);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("start acp agent {}", self.cfg.command))?;
        let stdin = child.stdin.take().context("create stdin pipe")?;
        let stdout = child.stdout.take().context("create stdout pipe")?;
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::warn!(target: "acp-stderr", "{line}");
                }
            });
        }
        let pid = child.id();
        {
            let mut state = self.state.lock().await;
            state.child = Some(child);
            state.stdin = Some(stdin);
            state.pid = pid;
            state.started = true;
        }

        self.spawn_read_loop(stdout);

        let init = async {
            if self.protocol == Protocol::CodexAppServer {
                let value = self
                    .rpc("initialize", json!({"clientInfo": {"name": "welinker", "version": env!("CARGO_PKG_VERSION")}}))
                    .await?;
                self.notify("initialized", Value::Null).await?;
                Ok(value)
            } else {
                self.rpc(
                    "initialize",
                    json!({
                        "protocolVersion": 1,
                        "clientCapabilities": {
                            "fs": { "readTextFile": true, "writeTextFile": true }
                        }
                    }),
                )
                .await
            }
        };
        let init_result = tokio::time::timeout(Duration::from_secs(30), init)
            .await
            .context("agent initialize timed out")??;
        tracing::info!(pid = ?pid, result = %init_result, "acp initialized");
        Ok(())
    }

    fn spawn_read_loop(&self, stdout: tokio::process::ChildStdout) {
        let pending = Arc::clone(&self.pending);
        let sessions = Arc::clone(&self.session_channels);
        let turns = Arc::clone(&self.turn_channels);
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            let re = Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").expect("ansi regex");
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = re.replace_all(&line, "").trim().to_string();
                if line.is_empty() || !line.starts_with('{') {
                    continue;
                }
                let Ok(msg) = serde_json::from_str::<RpcMessage>(&line) else {
                    tracing::warn!(line = %line, "failed to parse acp message");
                    continue;
                };
                if let Some(id) = msg.id {
                    if msg.method.is_none() {
                        if let Some(tx) = pending.lock().await.remove(&id) {
                            let _ = tx.send(msg);
                        }
                        continue;
                    }
                }
                match msg.method.as_deref() {
                    Some("session/update") => handle_session_update(&sessions, msg.params).await,
                    Some("session/event") => handle_zeroclaw_event(&sessions, msg.params).await,
                    Some("session/request_permission") | Some("turn/approval/request") => {
                        handle_permission_request(&state, &line).await;
                    }
                    Some("codex/event/agent_message_delta") => {
                        handle_codex_delta(&turns, msg.params).await
                    }
                    Some("item/agentMessage/delta") => {
                        handle_codex_item_delta(&turns, msg.params).await
                    }
                    Some("item/started") => handle_codex_item_started(&turns, msg.params).await,
                    Some("turn/completed") => handle_codex_turn_completed(&turns, msg.params).await,
                    Some(method) => {
                        tracing::debug!(method, "unhandled acp method");
                    }
                    None => {}
                }
            }
            tracing::info!("acp read loop ended");
        });
    }

    async fn write_json(&self, value: &Value) -> Result<()> {
        let mut state = self.state.lock().await;
        let stdin = state.stdin.as_mut().context("agent stdin unavailable")?;
        let mut data = serde_json::to_vec(value)?;
        data.push(b'\n');
        stdin.write_all(&data).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        if let Err(err) = self.write_json(&req).await {
            self.pending.lock().await.remove(&id);
            return Err(err);
        }
        let msg = rx.await.context("agent response channel closed")?;
        if let Some(err) = msg.error {
            anyhow::bail!("agent error: {}", err.message);
        }
        Ok(msg.result)
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let req = if params.is_null() {
            json!({"jsonrpc": "2.0", "method": method})
        } else {
            json!({"jsonrpc": "2.0", "method": method, "params": params})
        };
        self.write_json(&req).await
    }

    async fn get_or_create_session(&self, conversation_id: &str) -> Result<(String, bool)> {
        if let Some(id) = self.sessions.lock().await.get(conversation_id).cloned() {
            return Ok((id, false));
        }
        let result = self
            .rpc(
                "session/new",
                json!({"cwd": self.cwd.get().await, "mcpServers": []}),
            )
            .await?;
        let session_id = result["sessionId"].as_str().unwrap_or_default().to_string();
        if session_id.is_empty() {
            anyhow::bail!("session/new returned empty session id");
        }
        self.sessions
            .lock()
            .await
            .insert(conversation_id.to_string(), session_id.clone());
        Ok((session_id, true))
    }

    async fn get_or_create_thread(&self, conversation_id: &str) -> Result<(String, bool)> {
        if let Some(id) = self.threads.lock().await.get(conversation_id).cloned() {
            return Ok((id, false));
        }
        let mut params = json!({
            "approvalPolicy": "never",
            "cwd": self.cwd.get().await,
            "sandbox": "danger-full-access",
        });
        if !self.cfg.model.is_empty() {
            params["model"] = Value::String(self.cfg.model.clone());
        }
        let result = self.rpc("thread/start", params).await?;
        let thread_id = result["thread"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if thread_id.is_empty() {
            anyhow::bail!("thread/start returned empty thread id");
        }
        self.threads
            .lock()
            .await
            .insert(conversation_id.to_string(), thread_id.clone());
        Ok((thread_id, true))
    }

    async fn chat_legacy_acp(&self, conversation_id: &str, message: &str) -> Result<String> {
        let (session_id, _) = self.get_or_create_session(conversation_id).await?;
        let (tx, mut rx) = mpsc::channel(256);
        self.session_channels
            .lock()
            .await
            .insert(session_id.clone(), tx);

        let params = if self.protocol == Protocol::ZeroClawAcp {
            json!({"sessionId": session_id, "prompt": message})
        } else {
            json!({"sessionId": session_id, "prompt": [{"type": "text", "text": message}]})
        };
        let rpc_future = self.rpc("session/prompt", params);
        tokio::pin!(rpc_future);
        let mut parts = Vec::new();
        let result = loop {
            tokio::select! {
                update = rx.recv() => {
                    if let Some(update) = update {
                        if update.session_update == "agent_message_chunk" {
                            if let Some(text) = extract_chunk_text(&update) {
                                parts.push(text);
                            }
                        }
                    }
                }
                done = &mut rpc_future => {
                    break done;
                }
            }
        };
        self.session_channels.lock().await.remove(&session_id);
        let result = result?;
        while let Ok(update) = rx.try_recv() {
            if update.session_update == "agent_message_chunk" {
                if let Some(text) = extract_chunk_text(&update) {
                    parts.push(text);
                }
            }
        }
        let mut text = parts.join("").trim().to_string();
        if text.is_empty() {
            text = extract_prompt_result_text(&result);
        }
        if text.is_empty() {
            anyhow::bail!("agent returned empty response");
        }
        Ok(text)
    }

    async fn chat_codex_app_server(&self, conversation_id: &str, message: &str) -> Result<String> {
        let (thread_id, _) = self.get_or_create_thread(conversation_id).await?;
        let (tx, mut rx) = mpsc::channel(256);
        self.turn_channels
            .lock()
            .await
            .insert(thread_id.clone(), tx);
        let mut params = json!({
            "threadId": thread_id,
            "approvalPolicy": "never",
            "input": [{"type": "text", "text": message}],
            "sandboxPolicy": {"type": "dangerFullAccess"},
            "cwd": self.cwd.get().await,
        });
        if !self.cfg.model.is_empty() {
            params["model"] = Value::String(self.cfg.model.clone());
        }
        self.rpc("turn/start", params).await?;

        let mut parts = Vec::new();
        while let Some(evt) = rx.recv().await {
            if !evt.delta.is_empty() {
                parts.push(evt.delta);
            }
            if !evt.text.is_empty() {
                parts.push(evt.text);
            }
            if evt.kind == "completed" {
                break;
            }
        }
        self.turn_channels.lock().await.remove(&thread_id);
        let text = parts.join("").trim().to_string();
        if text.is_empty() {
            anyhow::bail!("agent returned empty response");
        }
        Ok(text)
    }
}

#[async_trait]
impl Agent for AcpAgent {
    async fn chat(&self, conversation_id: &str, message: &str) -> Result<String> {
        self.start().await?;
        if self.protocol == Protocol::CodexAppServer {
            self.chat_codex_app_server(conversation_id, message).await
        } else {
            self.chat_legacy_acp(conversation_id, message).await
        }
    }

    async fn reset_session(&self, conversation_id: &str) -> Result<Option<String>> {
        if self.protocol == Protocol::CodexAppServer {
            self.threads.lock().await.remove(conversation_id);
            let (id, _) = self.get_or_create_thread(conversation_id).await?;
            Ok(Some(id))
        } else {
            self.sessions.lock().await.remove(conversation_id);
            let (id, _) = self.get_or_create_session(conversation_id).await?;
            Ok(Some(id))
        }
    }

    fn info(&self) -> AgentInfo {
        let pid = self.state.try_lock().ok().and_then(|s| s.pid);
        AgentInfo {
            name: self.cfg.command.clone(),
            kind: "acp".to_string(),
            model: self.cfg.model.clone(),
            command: self.cfg.command.clone(),
            pid,
        }
    }

    async fn set_cwd(&self, cwd: PathBuf) {
        self.cwd.set(cwd).await;
    }
}

impl Drop for AcpAgent {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.try_lock() {
            if let Some(child) = state.child.as_mut() {
                let _ = child.start_kill();
            }
        }
    }
}

fn detect_protocol(command: &str, args: &[String]) -> Protocol {
    let base = Path::new(command)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if (base == "zeroclaw" || base == "zeroclaw.exe") && args.iter().any(|a| a == "acp") {
        return Protocol::ZeroClawAcp;
    }
    if (base == "codex" || base == "codex.exe") && args.iter().any(|a| a == "app-server") {
        return Protocol::CodexAppServer;
    }
    Protocol::LegacyAcp
}

async fn handle_session_update(
    sessions: &Arc<Mutex<HashMap<String, mpsc::Sender<SessionUpdate>>>>,
    params: Value,
) {
    let Ok(parsed) = serde_json::from_value::<SessionUpdateParams>(params) else {
        return;
    };
    if let Some(tx) = sessions.lock().await.get(&parsed.session_id).cloned() {
        let _ = tx.send(parsed.update).await;
    }
}

async fn handle_zeroclaw_event(
    sessions: &Arc<Mutex<HashMap<String, mpsc::Sender<SessionUpdate>>>>,
    params: Value,
) {
    let session_id = params["sessionId"].as_str().unwrap_or_default().to_string();
    if session_id.is_empty() {
        return;
    }
    let kind = params["type"].as_str().unwrap_or_default();
    let content = params["content"].as_str().unwrap_or_default().to_string();
    let update = SessionUpdate {
        session_update: if kind == "chunk" {
            "agent_message_chunk".to_string()
        } else if kind == "done" {
            "agent_message_complete".to_string()
        } else {
            kind.to_string()
        },
        text: content,
        ..SessionUpdate::default()
    };
    if let Some(tx) = sessions.lock().await.get(&session_id).cloned() {
        let _ = tx.send(update).await;
    }
}

async fn handle_codex_delta(
    turns: &Arc<Mutex<HashMap<String, mpsc::Sender<CodexTurnEvent>>>>,
    params: Value,
) {
    let key = params["conversationId"]
        .as_str()
        .or_else(|| params["threadId"].as_str())
        .unwrap_or_default()
        .to_string();
    let delta = params["msg"]["delta"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if delta.is_empty() {
        return;
    }
    dispatch_turn(
        turns,
        &key,
        CodexTurnEvent {
            kind: String::new(),
            delta,
            text: String::new(),
        },
    )
    .await;
}

async fn handle_codex_item_delta(
    turns: &Arc<Mutex<HashMap<String, mpsc::Sender<CodexTurnEvent>>>>,
    params: Value,
) {
    let key = params["threadId"].as_str().unwrap_or_default().to_string();
    let delta = params["delta"].as_str().unwrap_or_default().to_string();
    if !delta.is_empty() {
        dispatch_turn(
            turns,
            &key,
            CodexTurnEvent {
                kind: String::new(),
                delta,
                text: String::new(),
            },
        )
        .await;
    }
}

async fn handle_codex_item_started(
    turns: &Arc<Mutex<HashMap<String, mpsc::Sender<CodexTurnEvent>>>>,
    params: Value,
) {
    let key = params["threadId"].as_str().unwrap_or_default().to_string();
    if params["item"]["type"].as_str().unwrap_or_default() != "agentMessage" {
        return;
    }
    if let Some(items) = params["item"]["content"].as_array() {
        for item in items {
            if item["type"].as_str().unwrap_or_default() == "text" {
                let text = item["text"].as_str().unwrap_or_default().to_string();
                if !text.is_empty() {
                    dispatch_turn(
                        turns,
                        &key,
                        CodexTurnEvent {
                            kind: String::new(),
                            delta: String::new(),
                            text,
                        },
                    )
                    .await;
                }
            }
        }
    }
}

async fn handle_codex_turn_completed(
    turns: &Arc<Mutex<HashMap<String, mpsc::Sender<CodexTurnEvent>>>>,
    params: Value,
) {
    let key = params["threadId"].as_str().unwrap_or_default().to_string();
    dispatch_turn(
        turns,
        &key,
        CodexTurnEvent {
            kind: "completed".to_string(),
            delta: String::new(),
            text: String::new(),
        },
    )
    .await;
}

async fn dispatch_turn(
    turns: &Arc<Mutex<HashMap<String, mpsc::Sender<CodexTurnEvent>>>>,
    key: &str,
    event: CodexTurnEvent,
) {
    let tx = {
        let map = turns.lock().await;
        map.get(key)
            .cloned()
            .or_else(|| map.values().next().cloned())
    };
    if let Some(tx) = tx {
        let _ = tx.send(event).await;
    }
}

async fn handle_permission_request(state: &Arc<Mutex<State>>, raw: &str) {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        tracing::warn!("failed to parse permission request");
        return;
    };
    let id = value["id"].clone();
    if id.is_null() {
        tracing::warn!("permission request missing id");
        return;
    }
    let option_id = value["params"]["options"]
        .as_array()
        .and_then(|options| {
            options.iter().find_map(|opt| {
                if opt["kind"].as_str() == Some("allow") {
                    opt["optionId"].as_str().map(ToString::to_string)
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| "allow".to_string());

    let response = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "outcome": {
                "outcome": "selected",
                "optionId": option_id,
            }
        }
    });
    let mut data = match serde_json::to_vec(&response) {
        Ok(data) => data,
        Err(err) => {
            tracing::warn!(error = %err, "failed to serialize permission response");
            return;
        }
    };
    data.push(b'\n');
    let mut locked = state.lock().await;
    if let Some(stdin) = locked.stdin.as_mut() {
        if let Err(err) = stdin.write_all(&data).await {
            tracing::warn!(error = %err, "failed to write permission response");
        }
    }
}

fn extract_chunk_text(update: &SessionUpdate) -> Option<String> {
    if !update.text.is_empty() {
        return Some(update.text.clone());
    }
    if update.content.is_object() {
        if let Some(text) = update.content["text"].as_str() {
            return Some(text.to_string());
        }
    }
    None
}

fn extract_prompt_result_text(result: &Value) -> String {
    if let Some(content) = result["content"].as_str() {
        return content.to_string();
    }
    if let Some(text) = result["text"].as_str() {
        return text.to_string();
    }
    if let Some(items) = result["content"].as_array() {
        let mut out = String::new();
        for item in items {
            if item["type"].as_str().unwrap_or_default() == "text" {
                out.push_str(item["text"].as_str().unwrap_or_default());
            }
        }
        return out;
    }
    String::new()
}
