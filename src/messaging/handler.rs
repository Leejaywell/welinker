use super::{
    extract_image_urls, save_inbound_media_item, send_media_from_url, send_text_reply,
    send_typing_state,
};
use crate::{
    agent::SharedAgent,
    ilink::{
        Client, WeixinMessage, ITEM_TYPE_TEXT, ITEM_TYPE_VOICE, MESSAGE_STATE_FINISH,
        MESSAGE_TYPE_USER,
    },
};
use anyhow::Result;
use futures::future::BoxFuture;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

pub type AgentFactory =
    Arc<dyn Fn(String) -> BoxFuture<'static, Option<SharedAgent>> + Send + Sync + 'static>;
pub type SaveDefault =
    Arc<dyn Fn(String) -> BoxFuture<'static, Result<()>> + Send + Sync + 'static>;

#[derive(Debug, Clone)]
pub struct AgentMeta {
    pub name: String,
    pub kind: String,
    pub command: String,
    pub model: String,
}

pub struct Handler {
    default_name: RwLock<String>,
    agents: RwLock<HashMap<String, SharedAgent>>,
    agent_metas: RwLock<Vec<AgentMeta>>,
    agent_work_dirs: RwLock<HashMap<String, PathBuf>>,
    custom_aliases: RwLock<HashMap<String, String>>,
    factory: AgentFactory,
    save_default: SaveDefault,
    seen_msgs: Mutex<HashMap<i64, Instant>>,
    save_dir: RwLock<Option<PathBuf>>,
}

impl Handler {
    pub fn new(factory: AgentFactory, save_default: SaveDefault) -> Arc<Self> {
        Arc::new(Self {
            default_name: RwLock::new(String::new()),
            agents: RwLock::new(HashMap::new()),
            agent_metas: RwLock::new(Vec::new()),
            agent_work_dirs: RwLock::new(HashMap::new()),
            custom_aliases: RwLock::new(HashMap::new()),
            factory,
            save_default,
            seen_msgs: Mutex::new(HashMap::new()),
            save_dir: RwLock::new(None),
        })
    }

    pub async fn set_default_agent(&self, name: String, agent: SharedAgent) {
        *self.default_name.write().await = name.clone();
        self.agents.write().await.insert(name, agent);
    }

    pub async fn set_agent_metas(&self, metas: Vec<AgentMeta>) {
        *self.agent_metas.write().await = metas;
    }

    pub async fn set_agent_work_dirs(&self, dirs: HashMap<String, PathBuf>) {
        *self.agent_work_dirs.write().await = dirs;
    }

    pub async fn set_custom_aliases(&self, aliases: HashMap<String, String>) {
        *self.custom_aliases.write().await = aliases;
    }

    pub async fn set_save_dir(&self, save_dir: Option<PathBuf>) {
        *self.save_dir.write().await = save_dir;
    }

    pub async fn default_agent_name(&self) -> String {
        self.default_name.read().await.clone()
    }

    pub async fn local_chat(&self, conversation_id: &str, text: &str, agent: &str) -> String {
        let trimmed = text.trim();
        if trimmed == "/info" {
            return self.build_status().await;
        }
        if trimmed == "/help" {
            return build_help_text();
        }
        if trimmed == "/new" || trimmed == "/clear" {
            if agent.trim().is_empty() {
                return self.reset_default_session(conversation_id).await;
            }
            let name = self.resolve_alias(agent.trim()).await;
            return self.reset_named_session(conversation_id, &name).await;
        }
        if trimmed.starts_with("/cwd") {
            return self.handle_cwd(trimmed).await;
        }
        if !agent.trim().is_empty() {
            let name = self.resolve_alias(agent.trim()).await;
            return self.send_to_named(conversation_id, &name, text).await;
        }

        let (agent_names, message) = self.parse_command(text).await;
        if agent_names.is_empty() {
            return self.send_to_default(conversation_id, text).await;
        }
        if message.is_empty() {
            if agent_names.len() == 1 && self.is_known_agent(&agent_names[0]).await {
                return self.switch_default(agent_names[0].clone()).await;
            }
            if agent_names.len() == 1 {
                return self.send_to_default(conversation_id, text).await;
            }
            return "Usage: specify one agent to switch, or add a message to broadcast".to_string();
        }

        let known = self.filter_known_agents(agent_names).await;
        if known.is_empty() {
            return self.send_to_default(conversation_id, text).await;
        }
        if known.len() == 1 {
            return self
                .send_to_named(conversation_id, &known[0], &message)
                .await;
        }

        let mut replies = Vec::new();
        for name in known {
            let reply = self.send_to_named(conversation_id, &name, &message).await;
            replies.push(format!("[{name}] {reply}"));
        }
        replies.join("\n\n")
    }

    pub async fn handle_message(self: Arc<Self>, client: Client, msg: WeixinMessage) {
        if msg.message_type != MESSAGE_TYPE_USER || msg.message_state != MESSAGE_STATE_FINISH {
            return;
        }
        if msg.message_id != 0 && !self.mark_seen(msg.message_id).await {
            return;
        }
        let Some(text) = extract_text(&msg).or_else(|| extract_voice_text(&msg)) else {
            if let Some(reply) = self.try_save_inbound_media(&msg).await {
                self.send_reply_with_media(&client, &msg, &reply, Some(Uuid::new_v4().to_string()))
                    .await;
            } else {
                tracing::info!(from = %msg.from_user_id, "received non-text message, skipping");
            }
            return;
        };
        let trimmed = text.trim();
        let client_id = Uuid::new_v4().to_string();

        let reply = if trimmed == "/info" {
            self.build_status().await
        } else if trimmed == "/help" {
            build_help_text()
        } else if trimmed == "/new" || trimmed == "/clear" {
            self.reset_default_session(&msg.from_user_id).await
        } else if trimmed.starts_with("/cwd") {
            self.handle_cwd(trimmed).await
        } else {
            let (agent_names, message) = self.parse_command(&text).await;
            if agent_names.is_empty() {
                let _ = send_typing_state(&client, &msg.from_user_id, &msg.context_token).await;
                self.send_to_default(&msg.from_user_id, &text).await
            } else if message.is_empty() {
                if agent_names.len() == 1 && self.is_known_agent(&agent_names[0]).await {
                    self.switch_default(agent_names[0].clone()).await
                } else if agent_names.len() == 1 {
                    self.send_to_default(&msg.from_user_id, &text).await
                } else {
                    "Usage: specify one agent to switch, or add a message to broadcast".to_string()
                }
            } else {
                let known = self.filter_known_agents(agent_names).await;
                if known.is_empty() {
                    self.send_to_default(&msg.from_user_id, &text).await
                } else if known.len() == 1 {
                    let _ = send_typing_state(&client, &msg.from_user_id, &msg.context_token).await;
                    self.send_to_named(&msg.from_user_id, &known[0], &message)
                        .await
                } else {
                    self.broadcast(&client, &msg, known, message).await;
                    return;
                }
            }
        };

        self.send_reply_with_media(&client, &msg, &reply, Some(client_id))
            .await;
    }

    async fn mark_seen(&self, message_id: i64) -> bool {
        let mut seen = self.seen_msgs.lock().await;
        let cutoff = Instant::now() - Duration::from_secs(300);
        seen.retain(|_, t| *t >= cutoff);
        seen.insert(message_id, Instant::now()).is_none()
    }

    async fn try_save_inbound_media(&self, msg: &WeixinMessage) -> Option<String> {
        let save_dir = self.save_dir.read().await.clone()?;
        let mut saved = Vec::new();
        let mut failed = Vec::new();
        for item in &msg.item_list {
            match save_inbound_media_item(item, &save_dir).await {
                Ok(Some(path)) => saved.push(path),
                Ok(None) => {}
                Err(err) => failed.push(err.to_string()),
            }
        }
        if saved.is_empty() && failed.is_empty() {
            return None;
        }
        let mut lines = Vec::new();
        for path in saved {
            lines.push(format!("已保存: {}", path.display()));
        }
        for err in failed {
            lines.push(format!("保存失败: {err}"));
        }
        Some(lines.join("\n"))
    }

    async fn get_agent(&self, name: &str) -> Result<SharedAgent> {
        if let Some(agent) = self.agents.read().await.get(name).cloned() {
            return Ok(agent);
        }
        let agent = (self.factory)(name.to_string())
            .await
            .ok_or_else(|| anyhow::anyhow!("agent {name:?} not available"))?;
        self.agents
            .write()
            .await
            .insert(name.to_string(), Arc::clone(&agent));
        Ok(agent)
    }

    async fn send_to_default(&self, user_id: &str, text: &str) -> String {
        let name = self.default_name.read().await.clone();
        if name.is_empty() {
            return format!("[echo] {text}");
        }
        match self.get_agent(&name).await {
            Ok(agent) => match agent.chat(user_id, text).await {
                Ok(reply) => reply,
                Err(err) => format!("Error: {err}"),
            },
            Err(err) => format!("[echo] {text}\n\nAgent not ready: {err}"),
        }
    }

    async fn send_to_named(&self, user_id: &str, name: &str, text: &str) -> String {
        match self.get_agent(name).await {
            Ok(agent) => match agent.chat(user_id, text).await {
                Ok(reply) => reply,
                Err(err) => format!("Error: {err}"),
            },
            Err(err) => format!("Agent {name:?} is not available: {err}"),
        }
    }

    async fn broadcast(
        &self,
        client: &Client,
        msg: &WeixinMessage,
        names: Vec<String>,
        message: String,
    ) {
        for name in names {
            let reply = self.send_to_named(&msg.from_user_id, &name, &message).await;
            let reply = format!("[{name}] {reply}");
            self.send_reply_with_media(client, msg, &reply, None).await;
        }
    }

    async fn send_reply_with_media(
        &self,
        client: &Client,
        msg: &WeixinMessage,
        reply: &str,
        client_id: Option<String>,
    ) {
        if let Err(err) = send_text_reply(
            client,
            &msg.from_user_id,
            reply,
            &msg.context_token,
            client_id,
        )
        .await
        {
            tracing::warn!(to = %msg.from_user_id, error = %err, "failed to send reply");
        }
        for url in extract_image_urls(reply) {
            if let Err(err) =
                send_media_from_url(client, &msg.from_user_id, &url, &msg.context_token).await
            {
                tracing::warn!(to = %msg.from_user_id, url, error = %err, "failed to send image");
            }
        }
    }

    async fn switch_default(&self, name: String) -> String {
        match self.get_agent(&name).await {
            Ok(agent) => {
                *self.default_name.write().await = name.clone();
                self.agents.write().await.insert(name.clone(), agent);
                if let Err(err) = (self.save_default)(name.clone()).await {
                    tracing::warn!(error = %err, "failed to save default agent");
                }
                format!("switch to {name}")
            }
            Err(err) => format!("Failed to switch to {name:?}: {err}"),
        }
    }

    async fn reset_default_session(&self, user_id: &str) -> String {
        let name = self.default_name.read().await.clone();
        if name.is_empty() {
            return "No agent running.".to_string();
        }
        self.reset_named_session(user_id, &name).await
    }

    async fn reset_named_session(&self, user_id: &str, name: &str) -> String {
        match self.get_agent(name).await {
            Ok(agent) => match agent.reset_session(user_id).await {
                Ok(Some(id)) => format!("已创建新的{}会话\n{}", agent.info().name, id),
                Ok(None) => format!("已创建新的{}会话", agent.info().name),
                Err(err) => format!("Failed to reset session: {err}"),
            },
            Err(_) => "No agent running.".to_string(),
        }
    }

    async fn handle_cwd(&self, text: &str) -> String {
        let arg = text.trim_start_matches("/cwd").trim();
        if arg.is_empty() {
            let name = self.default_name.read().await.clone();
            return format!("cwd: (check agent config)\nagent: {name}");
        }
        let path = expand_home(arg);
        let Ok(abs) = path.canonicalize() else {
            return format!("Path not found: {}", path.display());
        };
        if !abs.is_dir() {
            return format!("Not a directory: {}", abs.display());
        }
        let agents = self.agents.read().await.clone();
        for (name, agent) in agents {
            agent.set_cwd(abs.clone()).await;
            self.agent_work_dirs.write().await.insert(name, abs.clone());
        }
        format!("cwd: {}", abs.display())
    }

    async fn build_status(&self) -> String {
        let name = self.default_name.read().await.clone();
        if name.is_empty() {
            return "agent: none (echo mode)".to_string();
        }
        if let Some(agent) = self.agents.read().await.get(&name) {
            let info = agent.info();
            return format!("agent: {name}\ntype: {}\nmodel: {}", info.kind, info.model);
        }
        if let Some(meta) = self
            .agent_metas
            .read()
            .await
            .iter()
            .find(|meta| meta.name == name)
        {
            return format!(
                "agent: {} (not started)\ntype: {}\nmodel: {}\ncommand: {}",
                meta.name, meta.kind, meta.model, meta.command
            );
        }
        format!("agent: {name} (not started)")
    }

    async fn parse_command(&self, text: &str) -> (Vec<String>, String) {
        if !text.starts_with('/') && !text.starts_with('@') {
            return (Vec::new(), text.to_string());
        }
        let mut names = Vec::new();
        let mut rest = text.trim().to_string();
        while rest.starts_with('/') || rest.starts_with('@') {
            let after = &rest[1..];
            let idx = after.find([' ', '/', '@']);
            let (token, new_rest) = match idx {
                None => (after, ""),
                Some(i) if after.as_bytes()[i] == b'/' || after.as_bytes()[i] == b'@' => {
                    (&after[..i], &after[i..])
                }
                Some(i) => (&after[..i], after[i + 1..].trim()),
            };
            if !token.is_empty() {
                names.push(self.resolve_alias(token).await);
            }
            rest = new_rest.trim().to_string();
            if rest.is_empty() {
                break;
            }
        }
        let mut seen = HashSet::new();
        names.retain(|n| seen.insert(n.clone()));
        (names, rest)
    }

    async fn resolve_alias(&self, name: &str) -> String {
        if let Some(full) = self.custom_aliases.read().await.get(name).cloned() {
            return full;
        }
        built_in_aliases()
            .get(name)
            .copied()
            .unwrap_or(name)
            .to_string()
    }

    async fn is_known_agent(&self, name: &str) -> bool {
        if self.agents.read().await.contains_key(name) {
            return true;
        }
        self.agent_metas.read().await.iter().any(|m| m.name == name)
    }

    async fn filter_known_agents(&self, names: Vec<String>) -> Vec<String> {
        let mut out = Vec::new();
        for name in names {
            if self.is_known_agent(&name).await {
                out.push(name);
            }
        }
        out
    }
}

fn extract_text(msg: &WeixinMessage) -> Option<String> {
    msg.item_list
        .iter()
        .find(|item| item.kind == ITEM_TYPE_TEXT)
        .and_then(|item| item.text_item.as_ref())
        .map(|item| item.text.clone())
        .filter(|s| !s.is_empty())
}

fn extract_voice_text(msg: &WeixinMessage) -> Option<String> {
    msg.item_list
        .iter()
        .find(|item| item.kind == ITEM_TYPE_VOICE)
        .and_then(|item| item.voice_item.as_ref())
        .map(|item| item.text.clone())
        .filter(|s| !s.is_empty())
}

fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn built_in_aliases() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("cc", "claude"),
        ("cx", "codex"),
        ("oc", "openclaw"),
        ("zc", "zeroclaw"),
        ("cs", "cursor"),
        ("km", "kimi"),
        ("gm", "gemini"),
        ("ocd", "opencode"),
        ("pi", "pi"),
        ("cp", "copilot"),
        ("dr", "droid"),
        ("if", "iflow"),
        ("kr", "kiro"),
        ("qw", "qwen"),
        ("hm", "hermes"),
        ("hh", "hermes-http"),
    ])
}

fn build_help_text() -> String {
    "Available commands:\n@agent or /agent - Switch default agent\n@agent msg or /agent msg - Send to a specific agent\n@a @b msg - Broadcast to multiple agents\n/new or /clear - Start a new session\n/cwd /path - Switch workspace directory\n/info - Show current agent info\n/help - Show this help message\n\nAliases: /cc(claude) /cx(codex) /cs(cursor) /km(kimi) /gm(gemini) /oc(openclaw) /zc(zeroclaw) /ocd(opencode) /pi(pi) /cp(copilot) /dr(droid) /if(iflow) /kr(kiro) /qw(qwen) /hm(hermes) /hh(hermes-http)".to_string()
}
