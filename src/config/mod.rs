use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::PathBuf,
    process::Command,
    time::Duration,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub default_agent: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_addr: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub save_dir: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub route_tag: String,
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub system_prompt: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub max_history: usize,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

#[derive(Debug, Clone)]
struct AgentCandidate {
    name: &'static str,
    binary: &'static str,
    args: &'static [&'static str],
    check_args: &'static [&'static str],
    kind: &'static str,
    model: &'static str,
}

const AGENT_CANDIDATES: &[AgentCandidate] = &[
    AgentCandidate {
        name: "claude",
        binary: "claude-agent-acp",
        args: &[],
        check_args: &[],
        kind: "acp",
        model: "sonnet",
    },
    AgentCandidate {
        name: "claude",
        binary: "claude",
        args: &[],
        check_args: &[],
        kind: "cli",
        model: "sonnet",
    },
    AgentCandidate {
        name: "codex",
        binary: "codex-acp",
        args: &[],
        check_args: &[],
        kind: "acp",
        model: "",
    },
    AgentCandidate {
        name: "codex",
        binary: "codex",
        args: &["app-server", "--listen", "stdio://"],
        check_args: &["app-server", "--help"],
        kind: "acp",
        model: "",
    },
    AgentCandidate {
        name: "codex",
        binary: "codex",
        args: &[],
        check_args: &[],
        kind: "cli",
        model: "",
    },
    AgentCandidate {
        name: "cursor",
        binary: "agent",
        args: &["acp"],
        check_args: &[],
        kind: "acp",
        model: "",
    },
    AgentCandidate {
        name: "kimi",
        binary: "kimi",
        args: &["acp"],
        check_args: &[],
        kind: "acp",
        model: "",
    },
    AgentCandidate {
        name: "gemini",
        binary: "gemini",
        args: &["--acp"],
        check_args: &[],
        kind: "acp",
        model: "",
    },
    AgentCandidate {
        name: "opencode",
        binary: "opencode",
        args: &["acp"],
        check_args: &[],
        kind: "acp",
        model: "",
    },
    AgentCandidate {
        name: "openclaw",
        binary: "openclaw",
        args: &[],
        check_args: &[],
        kind: "acp",
        model: "openclaw:main",
    },
    AgentCandidate {
        name: "zeroclaw",
        binary: "zeroclaw",
        args: &["acp"],
        check_args: &["acp", "--help"],
        kind: "acp",
        model: "zeroclaw",
    },
    AgentCandidate {
        name: "pi",
        binary: "pi-acp",
        args: &[],
        check_args: &[],
        kind: "acp",
        model: "",
    },
    AgentCandidate {
        name: "copilot",
        binary: "copilot",
        args: &["--acp", "--stdio"],
        check_args: &[],
        kind: "acp",
        model: "",
    },
    AgentCandidate {
        name: "droid",
        binary: "droid",
        args: &["exec", "--output-format", "acp"],
        check_args: &[],
        kind: "acp",
        model: "",
    },
    AgentCandidate {
        name: "iflow",
        binary: "iflow",
        args: &["--experimental-acp"],
        check_args: &[],
        kind: "acp",
        model: "",
    },
    AgentCandidate {
        name: "kiro",
        binary: "kiro-cli",
        args: &["acp"],
        check_args: &[],
        kind: "acp",
        model: "",
    },
    AgentCandidate {
        name: "qwen",
        binary: "qwen",
        args: &["--acp"],
        check_args: &[],
        kind: "acp",
        model: "",
    },
    AgentCandidate {
        name: "hermes",
        binary: "hermes",
        args: &["acp"],
        check_args: &["acp", "--help"],
        kind: "acp",
        model: "",
    },
];

const DEFAULT_ORDER: &[&str] = &[
    "gemini",
    "claude",
    "codex",
    "cursor",
    "kimi",
    "opencode",
    "openclaw",
    "zeroclaw",
    "pi",
    "copilot",
    "droid",
    "iflow",
    "kiro",
    "qwen",
    "hermes",
    "hermes-http",
];

pub fn app_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(env::temp_dir)
        .join(".welinker")
}

pub fn config_path() -> PathBuf {
    app_dir().join("config.json")
}

pub fn load() -> Result<Config> {
    let path = config_path();
    let mut cfg = if path.exists() {
        let data = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_slice::<Config>(&data)
            .with_context(|| format!("parse {}", path.display()))?
    } else {
        Config::default()
    };
    load_env(&mut cfg);
    Ok(cfg)
}

fn load_env(cfg: &mut Config) {
    if let Ok(v) = env::var("WELINKER_DEFAULT_AGENT") {
        if !v.is_empty() {
            cfg.default_agent = v;
        }
    }
    if let Ok(v) = env::var("WELINKER_API_ADDR") {
        if !v.is_empty() {
            cfg.api_addr = v;
        }
    }
    if let Ok(v) = env::var("WELINKER_SAVE_DIR") {
        if !v.is_empty() {
            cfg.save_dir = v;
        }
    }
    if let Ok(v) = env::var("WELINKER_ROUTE_TAG") {
        if !v.is_empty() {
            cfg.route_tag = v;
        }
    }
}

pub fn save(cfg: &Config) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let data = serde_json::to_vec_pretty(cfg)?;
    fs::write(&path, data).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn build_alias_map(agents: &HashMap<String, AgentConfig>) -> HashMap<String, String> {
    let reserved: HashSet<&str> = ["info", "help", "new", "clear", "cwd"]
        .into_iter()
        .collect();
    let mut out = HashMap::new();
    for (name, cfg) in agents {
        for alias in &cfg.aliases {
            if reserved.contains(alias.as_str()) {
                tracing::warn!(
                    alias,
                    agent = name,
                    "alias conflicts with built-in command, ignored"
                );
                continue;
            }
            if let Some(existing) = out.insert(alias.clone(), name.clone()) {
                tracing::warn!(
                    alias,
                    existing,
                    agent = name,
                    "duplicate alias, using later agent"
                );
            }
        }
    }
    out
}

pub fn detect_and_configure(cfg: &mut Config) -> bool {
    let mut modified = false;

    for candidate in AGENT_CANDIDATES {
        if cfg.agents.contains_key(candidate.name) {
            continue;
        }
        let Ok(path) = look_path(candidate.binary) else {
            continue;
        };
        if !candidate.check_args.is_empty() && !command_probe(&path, candidate.check_args) {
            tracing::warn!(agent = candidate.name, command = %path.display(), "capability probe failed");
            continue;
        }
        tracing::info!(agent = candidate.name, command = %path.display(), kind = candidate.kind, "auto-detected agent");
        cfg.agents.insert(
            candidate.name.to_string(),
            AgentConfig {
                kind: candidate.kind.to_string(),
                command: path.to_string_lossy().into_owned(),
                args: candidate.args.iter().map(|s| s.to_string()).collect(),
                model: candidate.model.to_string(),
                ..AgentConfig::default()
            },
        );
        modified = true;
    }

    if let Some(openclaw) = cfg.agents.get("openclaw").cloned() {
        if openclaw.kind == "acp" && openclaw.args.is_empty() {
            let (url, token, password) = load_openclaw_gateway();
            if !url.is_empty() {
                let endpoint = url
                    .replace("wss://", "https://")
                    .replace("ws://", "http://")
                    .trim_end_matches('/')
                    .to_string()
                    + "/v1/chat/completions";
                let mut headers = HashMap::new();
                headers.insert(
                    "x-openclaw-scopes".to_string(),
                    "operator.write".to_string(),
                );
                cfg.agents.insert(
                    "openclaw".to_string(),
                    AgentConfig {
                        kind: "http".to_string(),
                        endpoint,
                        api_key: token.clone(),
                        headers,
                        model: "openclaw:main".to_string(),
                        ..AgentConfig::default()
                    },
                );
                if !cfg.agents.contains_key("openclaw-acp") {
                    let mut args = vec!["acp".to_string(), "--url".to_string(), url];
                    if !token.is_empty() {
                        args.extend(["--token".to_string(), token]);
                    } else if !password.is_empty() {
                        args.extend(["--password".to_string(), password]);
                    }
                    cfg.agents.insert(
                        "openclaw-acp".to_string(),
                        AgentConfig {
                            kind: "acp".to_string(),
                            command: openclaw.command,
                            args,
                            model: "openclaw:main".to_string(),
                            ..AgentConfig::default()
                        },
                    );
                }
                modified = true;
            } else {
                cfg.agents.remove("openclaw");
                modified = true;
            }
        }
    }

    if !cfg.agents.contains_key("openclaw") {
        let (url, token, _) = load_openclaw_gateway();
        if !url.is_empty() {
            let endpoint = url
                .replace("wss://", "https://")
                .replace("ws://", "http://")
                .trim_end_matches('/')
                .to_string()
                + "/v1/chat/completions";
            let mut headers = HashMap::new();
            headers.insert(
                "x-openclaw-scopes".to_string(),
                "operator.write".to_string(),
            );
            cfg.agents.insert(
                "openclaw".to_string(),
                AgentConfig {
                    kind: "http".to_string(),
                    endpoint,
                    api_key: token,
                    headers,
                    model: "openclaw:main".to_string(),
                    ..AgentConfig::default()
                },
            );
            modified = true;
        }
    }

    if !cfg.agents.contains_key("hermes-http") {
        if let Some(hermes) = load_hermes_api_server() {
            tracing::info!(endpoint = %hermes.endpoint, "auto-configured hermes-http agent");
            cfg.agents.insert(
                "hermes-http".to_string(),
                AgentConfig {
                    kind: "http".to_string(),
                    endpoint: hermes.endpoint,
                    api_key: hermes.api_key,
                    model: hermes.model,
                    ..AgentConfig::default()
                },
            );
            modified = true;
        }
    }

    if cfg.default_agent.is_empty() || !cfg.agents.contains_key(&cfg.default_agent) {
        if let Some(name) = DEFAULT_ORDER
            .iter()
            .find(|name| cfg.agents.contains_key(**name))
        {
            cfg.default_agent = (*name).to_string();
            modified = true;
        }
    }

    modified
}

fn command_probe(binary: &PathBuf, args: &[&str]) -> bool {
    Command::new(binary)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut child| {
            let start = std::time::Instant::now();
            loop {
                if let Some(status) = child.try_wait()? {
                    return Ok(status.success());
                }
                if start.elapsed() > Duration::from_secs(3) {
                    let _ = child.kill();
                    return Ok(false);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        })
        .unwrap_or(false)
}

fn look_path(binary: &str) -> Result<PathBuf> {
    if let Ok(path) = which::which(binary) {
        return Ok(path);
    }
    let shell = if cfg!(target_os = "macos") {
        "zsh"
    } else {
        "bash"
    };
    let output = Command::new(shell)
        .args(["-lic", &format!("which {}", binary)])
        .output()
        .with_context(|| format!("resolve {binary} via login shell"))?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() && !path.contains("not found") {
            return Ok(PathBuf::from(path));
        }
    }
    anyhow::bail!("not found: {binary}")
}

fn load_openclaw_gateway() -> (String, String, String) {
    let env_url = env::var("OPENCLAW_GATEWAY_URL").unwrap_or_default();
    if !env_url.is_empty() {
        return (
            env_url,
            env::var("OPENCLAW_GATEWAY_TOKEN").unwrap_or_default(),
            env::var("OPENCLAW_GATEWAY_PASSWORD").unwrap_or_default(),
        );
    }

    let Some(home) = dirs::home_dir() else {
        return Default::default();
    };
    let path = home.join(".openclaw").join("openclaw.json");
    let Ok(data) = fs::read(path) else {
        return Default::default();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&data) else {
        return Default::default();
    };
    let gw = &value["gateway"];
    if let Some(url) = gw["remote"]["url"].as_str() {
        if !url.is_empty() {
            return (
                url.to_string(),
                gw["remote"]["token"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                String::new(),
            );
        }
    }
    if let Some(port) = gw["port"].as_u64() {
        if port > 0 {
            let mut token = String::new();
            let mut password = String::new();
            match gw["auth"]["mode"].as_str().unwrap_or_default() {
                "token" => token = gw["auth"]["token"].as_str().unwrap_or_default().to_string(),
                "password" => {
                    password = gw["auth"]["password"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string()
                }
                _ => {}
            }
            return (format!("ws://127.0.0.1:{port}"), token, password);
        }
    }
    Default::default()
}

struct HermesApiServer {
    endpoint: String,
    api_key: String,
    model: String,
}

fn load_hermes_api_server() -> Option<HermesApiServer> {
    let env_file = load_hermes_env_file();
    let get = |key: &str| -> String {
        env::var(key)
            .ok()
            .filter(|v| !v.is_empty())
            .or_else(|| env_file.get(key).cloned().filter(|v| !v.is_empty()))
            .unwrap_or_default()
    };

    let explicit_url = env::var("WELINKER_HERMES_HTTP_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            env::var("HERMES_API_SERVER_URL")
                .ok()
                .filter(|v| !v.is_empty())
        });
    let api_key = env::var("WELINKER_HERMES_HTTP_KEY")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| get("API_SERVER_KEY"));
    let model = env::var("WELINKER_HERMES_HTTP_MODEL")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            let configured = get("API_SERVER_MODEL_NAME");
            if configured.is_empty() {
                "hermes-agent".to_string()
            } else {
                configured
            }
        });

    let base_url = if let Some(url) = explicit_url {
        url
    } else {
        let enabled = get("API_SERVER_ENABLED");
        let has_server_config = enabled.eq_ignore_ascii_case("true") || !api_key.is_empty();
        if !has_server_config {
            return None;
        }
        let host = {
            let configured = get("API_SERVER_HOST");
            if configured.is_empty() || configured == "0.0.0.0" {
                "127.0.0.1".to_string()
            } else {
                configured
            }
        };
        let port = {
            let configured = get("API_SERVER_PORT");
            if configured.is_empty() {
                "8642".to_string()
            } else {
                configured
            }
        };
        format!("http://{host}:{port}/v1")
    };

    Some(HermesApiServer {
        endpoint: normalize_openai_chat_endpoint(&base_url),
        api_key,
        model,
    })
}

fn load_hermes_env_file() -> HashMap<String, String> {
    let Some(home) = dirs::home_dir() else {
        return HashMap::new();
    };
    let path = home.join(".hermes").join(".env");
    let Ok(text) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let value = value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            Some((key.trim().to_string(), value))
        })
        .collect()
}

fn normalize_openai_chat_endpoint(raw: &str) -> String {
    let raw = raw.trim().trim_end_matches('/');
    if raw.ends_with("/v1/chat/completions") {
        return raw.to_string();
    }
    if raw.ends_with("/chat/completions") {
        return raw.to_string();
    }
    if raw.ends_with("/v1") {
        return format!("{raw}/chat/completions");
    }
    format!("{raw}/v1/chat/completions")
}

pub fn default_workspace() -> PathBuf {
    let dir = app_dir().join("workspace");
    let _ = fs::create_dir_all(&dir);
    dir
}
