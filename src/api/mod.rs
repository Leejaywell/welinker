use crate::{
    config::{self, Config},
    ilink::Client,
    messaging::{extract_image_urls, send_media_from_url, send_text_reply, Handler},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{env, fs, net::SocketAddr, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const MAX_REQUEST_SIZE: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct Server {
    clients: Arc<Vec<Client>>,
    addr: String,
    handler: Option<Arc<Handler>>,
}

#[derive(Debug, Deserialize)]
struct SendRequest {
    #[serde(default)]
    account_id: String,
    to: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    media_url: String,
}

#[derive(Debug, Deserialize)]
struct ChatRequest {
    #[serde(default = "default_conversation_id")]
    conversation_id: String,
    #[serde(default)]
    agent: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    status: &'static str,
    account_count: usize,
}

#[derive(Debug, Serialize)]
struct AccountResponse {
    account_id: String,
    normalized_id: String,
    user_id: String,
    base_url: String,
}

#[derive(Debug, Serialize)]
struct ConfigResponse {
    path: String,
    config: Config,
    reload_required: bool,
}

#[derive(Debug, Serialize)]
struct ChatResponse {
    status: &'static str,
    conversation_id: String,
    agent: String,
    reply: String,
}

fn default_conversation_id() -> String {
    "web".to_string()
}

fn allow_remote_api() -> bool {
    env::var("WELINKER_ALLOW_REMOTE_API")
        .map(|value| {
            let value = value.trim();
            value == "1" || value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
}

impl Server {
    pub fn new(clients: Vec<Client>, addr: String) -> Self {
        Self {
            clients: Arc::new(clients),
            addr: if addr.is_empty() {
                "127.0.0.1:18011".to_string()
            } else {
                addr
            },
            handler: None,
        }
    }

    pub fn with_handler(mut self, handler: Arc<Handler>) -> Self {
        self.handler = Some(handler);
        self
    }

    pub async fn run(self) -> Result<()> {
        let addr: SocketAddr = self.addr.parse()?;
        if !addr.ip().is_loopback() && !allow_remote_api() {
            anyhow::bail!(
                "refusing to bind API to non-loopback address {addr}; set WELINKER_ALLOW_REMOTE_API=1 to allow remote access"
            );
        }
        let listener = TcpListener::bind(addr).await?;
        tracing::info!(addr = %self.addr, "api listening");
        loop {
            let (mut stream, _) = listener.accept().await?;
            let this = self.clone();
            tokio::spawn(async move {
                let response = match read_http_request(&mut stream).await {
                    Ok(request) => this.handle_raw(&request).await,
                    Err(response) => response,
                };
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    }

    async fn handle_raw(&self, data: &[u8]) -> String {
        let request = String::from_utf8_lossy(data);
        let Some((head, body)) = request.split_once("\r\n\r\n") else {
            return http_response(400, "text/plain", "invalid HTTP request");
        };
        let first = head.lines().next().unwrap_or_default();
        if first.starts_with("GET / ") || first.starts_with("GET /index.html ") {
            return http_response(200, "text/html; charset=utf-8", WEB_UI_HTML);
        }
        if first.starts_with("GET /health ") {
            return http_response(200, "text/plain", "ok\n");
        }
        if first.starts_with("GET /api/status ") {
            return json_response(
                200,
                &StatusResponse {
                    status: "ok",
                    account_count: self.clients.len(),
                },
            );
        }
        if first.starts_with("GET /api/accounts ") {
            let accounts = self
                .clients
                .iter()
                .map(|client| AccountResponse {
                    account_id: client.bot_id().to_string(),
                    normalized_id: client.normalized_bot_id(),
                    user_id: client.user_id().to_string(),
                    base_url: client.base_url().to_string(),
                })
                .collect::<Vec<_>>();
            return json_response(200, &accounts);
        }
        if first.starts_with("GET /api/config ") {
            return self.handle_get_config();
        }
        if first.starts_with("PUT /api/config ") {
            return self.handle_put_config(body);
        }
        if first.starts_with("POST /api/chat ") {
            return self.handle_chat(body).await;
        }
        if first.starts_with("POST /api/send ") {
            return self.handle_send(body).await;
        }
        http_response(404, "text/plain", "not found")
    }

    fn handle_get_config(&self) -> String {
        let path = config::config_path();
        let cfg = if path.exists() {
            match fs::read(&path)
                .map_err(|err| anyhow::anyhow!("read {}: {err}", path.display()))
                .and_then(|data| {
                    serde_json::from_slice::<Config>(&data)
                        .map_err(|err| anyhow::anyhow!("parse {}: {err}", path.display()))
                }) {
                Ok(cfg) => cfg,
                Err(err) => {
                    return http_response(500, "text/plain", &format!("load config failed: {err}"));
                }
            }
        } else {
            Config::default()
        };
        json_response(
            200,
            &ConfigResponse {
                path: path.to_string_lossy().into_owned(),
                config: cfg,
                reload_required: false,
            },
        )
    }

    fn handle_put_config(&self, body: &str) -> String {
        let cfg: Config = match serde_json::from_str(body) {
            Ok(cfg) => cfg,
            Err(err) => return http_response(400, "text/plain", &format!("invalid JSON: {err}")),
        };
        if let Err(err) = config::save(&cfg) {
            return http_response(500, "text/plain", &format!("save config failed: {err}"));
        }
        json_response(
            200,
            &ConfigResponse {
                path: config::config_path().to_string_lossy().into_owned(),
                config: cfg,
                reload_required: true,
            },
        )
    }

    async fn handle_chat(&self, body: &str) -> String {
        let req: ChatRequest = match serde_json::from_str(body) {
            Ok(req) => req,
            Err(err) => return http_response(400, "text/plain", &format!("invalid JSON: {err}")),
        };
        let message = req.message.trim();
        if message.is_empty() {
            return http_response(400, "text/plain", "\"message\" is required");
        }
        let Some(handler) = &self.handler else {
            return http_response(503, "text/plain", "agent runtime is not configured");
        };
        let conversation_id = if req.conversation_id.trim().is_empty() {
            default_conversation_id()
        } else {
            req.conversation_id.trim().to_string()
        };
        let reply = handler
            .local_chat(&conversation_id, message, req.agent.trim())
            .await;
        let agent = if req.agent.trim().is_empty() {
            handler.default_agent_name().await
        } else {
            req.agent.trim().to_string()
        };
        json_response(
            200,
            &ChatResponse {
                status: "ok",
                conversation_id,
                agent,
                reply,
            },
        )
    }

    async fn handle_send(&self, body: &str) -> String {
        let req: SendRequest = match serde_json::from_str(body) {
            Ok(req) => req,
            Err(err) => return http_response(400, "text/plain", &format!("invalid JSON: {err}")),
        };
        if req.to.is_empty() {
            return http_response(400, "text/plain", "\"to\" is required");
        }
        if req.text.is_empty() && req.media_url.is_empty() {
            return http_response(400, "text/plain", "\"text\" or \"media_url\" is required");
        }
        let Some(client) = self.select_client(&req.account_id) else {
            return http_response(503, "text/plain", "no accounts configured");
        };
        if !req.text.is_empty() {
            if let Err(err) = send_text_reply(client, &req.to, &req.text, "", None).await {
                return http_response(500, "text/plain", &format!("send text failed: {err}"));
            }
            for url in extract_image_urls(&req.text) {
                let _ = send_media_from_url(client, &req.to, &url, "").await;
            }
        }
        if !req.media_url.is_empty() {
            if let Err(err) = send_media_from_url(client, &req.to, &req.media_url, "").await {
                return http_response(500, "text/plain", &format!("send media failed: {err}"));
            }
        }
        http_response(200, "application/json", r#"{"status":"ok"}"#)
    }

    fn select_client(&self, account_id: &str) -> Option<&Client> {
        if account_id.is_empty() {
            return self.clients.first();
        }
        self.clients.iter().find(|client| {
            client.bot_id() == account_id
                || crate::ilink::normalize_account_id(client.bot_id()) == account_id
        })
    }
}

async fn read_http_request(stream: &mut TcpStream) -> std::result::Result<Vec<u8>, String> {
    let mut buf = Vec::with_capacity(8192);
    let mut header_end = None;
    loop {
        let mut chunk = [0_u8; 8192];
        let n = stream.read(&mut chunk).await.map_err(|err| {
            http_response(400, "text/plain", &format!("read request failed: {err}"))
        })?;
        if n == 0 {
            break;
        }
        if buf.len() + n > MAX_REQUEST_SIZE {
            return Err(http_response(413, "text/plain", "request too large"));
        }
        buf.extend_from_slice(&chunk[..n]);
        if header_end.is_none() {
            header_end = find_header_end(&buf);
        }
        let Some(end) = header_end else {
            continue;
        };
        let content_length = parse_content_length(&buf[..end]).map_err(|err| {
            http_response(400, "text/plain", &format!("invalid Content-Length: {err}"))
        })?;
        let total = end + 4 + content_length;
        if total > MAX_REQUEST_SIZE {
            return Err(http_response(413, "text/plain", "request too large"));
        }
        while buf.len() < total {
            let mut chunk = [0_u8; 8192];
            let n = stream.read(&mut chunk).await.map_err(|err| {
                http_response(
                    400,
                    "text/plain",
                    &format!("read request body failed: {err}"),
                )
            })?;
            if n == 0 {
                return Err(http_response(400, "text/plain", "incomplete request body"));
            }
            if buf.len() + n > MAX_REQUEST_SIZE {
                return Err(http_response(413, "text/plain", "request too large"));
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        buf.truncate(total);
        return Ok(buf);
    }
    if find_header_end(&buf).is_none() {
        return Err(http_response(400, "text/plain", "invalid HTTP request"));
    }
    Ok(buf)
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(head: &[u8]) -> Result<usize> {
    let head = std::str::from_utf8(head).context("headers are not UTF-8")?;
    for line in head.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            return value
                .trim()
                .parse::<usize>()
                .context("not a valid positive integer");
        }
    }
    Ok(0)
}

fn http_response(status: u16, content_type: &str, body: &str) -> String {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "OK",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn json_response<T: Serialize>(status: u16, value: &T) -> String {
    match serde_json::to_string(value) {
        Ok(body) => http_response(status, "application/json", &body),
        Err(err) => http_response(
            500,
            "text/plain",
            &format!("failed to serialize response: {err}"),
        ),
    }
}

const WEB_UI_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Welinker</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f7f8fb;
      --surface: #ffffff;
      --surface-2: #f2f5f8;
      --text: #151a22;
      --muted: #687385;
      --line: #dfe5ed;
      --line-strong: #c7d0dc;
      --primary: #0f766e;
      --primary-strong: #0a5c57;
      --primary-soft: #e6f4f2;
      --blue: #2563eb;
      --amber: #b7791f;
      --danger: #b42318;
      --ok: #16794c;
      --focus: #2f80ed;
      --shadow: 0 18px 45px rgba(28, 39, 54, 0.08);
    }

    * {
      box-sizing: border-box;
    }

    body {
      margin: 0;
      min-height: 100vh;
      color: var(--text);
      background:
        linear-gradient(rgba(15, 23, 42, 0.035) 1px, transparent 1px),
        linear-gradient(90deg, rgba(15, 23, 42, 0.035) 1px, transparent 1px),
        var(--bg);
      background-size: 28px 28px;
      font: 14px/1.45 ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }

    button,
    input,
    textarea,
    select {
      font: inherit;
    }

    button {
      cursor: pointer;
    }

    .shell {
      width: min(1240px, calc(100vw - 32px));
      margin: 0 auto;
    }

    .topbar {
      position: sticky;
      top: 0;
      z-index: 5;
      border-bottom: 1px solid rgba(199, 208, 220, 0.72);
      background: rgba(247, 248, 251, 0.9);
      backdrop-filter: blur(14px);
    }

    .topbar-inner {
      min-height: 68px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 18px;
    }

    .brand {
      display: flex;
      align-items: center;
      gap: 12px;
      min-width: 0;
    }

    .brand-mark {
      width: 34px;
      height: 34px;
      display: grid;
      place-items: center;
      flex: 0 0 auto;
      border: 1px solid rgba(15, 118, 110, 0.28);
      border-radius: 8px;
      background: linear-gradient(135deg, #ffffff, #e6f4f2);
      color: var(--primary-strong);
      box-shadow: 0 8px 24px rgba(15, 118, 110, 0.11);
    }

    h1,
    h2,
    h3,
    p {
      margin: 0;
    }

    h1 {
      font-size: 18px;
      font-weight: 720;
      letter-spacing: 0;
      line-height: 1.1;
    }

    .brand-subtitle {
      margin-top: 3px;
      color: var(--muted);
      font-size: 12px;
      overflow-wrap: anywhere;
    }

    .toolbar {
      display: flex;
      align-items: center;
      justify-content: flex-end;
      gap: 10px;
      flex-wrap: wrap;
    }

    .status-pill,
    .metric,
    .endpoint-pill {
      display: inline-flex;
      align-items: center;
      gap: 8px;
      min-height: 34px;
      border: 1px solid var(--line);
      border-radius: 999px;
      background: rgba(255, 255, 255, 0.82);
      padding: 7px 11px;
      color: var(--muted);
      white-space: nowrap;
    }

    .endpoint-pill code {
      color: var(--text);
      font-size: 12px;
    }

    .dot {
      width: 8px;
      height: 8px;
      flex: 0 0 auto;
      border-radius: 50%;
      background: var(--amber);
      box-shadow: 0 0 0 4px rgba(183, 121, 31, 0.12);
    }

    .dot.ok {
      background: var(--ok);
      box-shadow: 0 0 0 4px rgba(22, 121, 76, 0.12);
    }

    .dot.error {
      background: var(--danger);
      box-shadow: 0 0 0 4px rgba(180, 35, 24, 0.12);
    }

    main.shell {
      padding: 22px 0 42px;
    }

    .view-tabs {
      display: inline-flex;
      align-items: center;
      gap: 4px;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: rgba(255, 255, 255, 0.9);
      padding: 4px;
      margin-bottom: 16px;
      box-shadow: 0 10px 28px rgba(28, 39, 54, 0.06);
    }

    .tab-button {
      min-height: 34px;
      border: 0;
      border-radius: 6px;
      background: transparent;
      color: var(--muted);
      padding: 7px 12px;
      font-weight: 700;
    }

    .tab-button[aria-selected="true"] {
      background: var(--primary);
      color: #ffffff;
      box-shadow: 0 8px 18px rgba(15, 118, 110, 0.17);
    }

    .view[hidden] {
      display: none !important;
    }

    .layout {
      display: grid;
      grid-template-columns: 340px minmax(0, 1fr);
      gap: 18px;
      align-items: start;
    }

    .panel {
      position: relative;
      overflow: hidden;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: rgba(255, 255, 255, 0.94);
      box-shadow: var(--shadow);
    }

    .panel.feature {
      isolation: isolate;
    }

    .panel.feature::before {
      content: "";
      position: absolute;
      inset: -1px;
      z-index: -1;
      border-radius: inherit;
      background:
        linear-gradient(90deg, transparent, rgba(15, 118, 110, 0.34), transparent),
        linear-gradient(180deg, rgba(37, 99, 235, 0.14), transparent 32%);
      background-size: 220% 100%, 100% 100%;
      animation: borderSweep 7s linear infinite;
    }

    .panel.feature::after {
      content: "";
      position: absolute;
      inset: 1px;
      z-index: -1;
      border-radius: 7px;
      background: rgba(255, 255, 255, 0.97);
    }

    @keyframes borderSweep {
      from {
        background-position: 220% 0, 0 0;
      }
      to {
        background-position: -220% 0, 0 0;
      }
    }

    .panel-head {
      min-height: 58px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      border-bottom: 1px solid var(--line);
      padding: 14px 16px;
    }

    .panel-title {
      display: flex;
      align-items: center;
      gap: 10px;
      min-width: 0;
    }

    .panel-title svg {
      width: 18px;
      height: 18px;
      flex: 0 0 auto;
      color: var(--primary);
    }

    h2 {
      font-size: 14px;
      font-weight: 700;
      letter-spacing: 0;
    }

    .panel-description {
      margin-top: 2px;
      color: var(--muted);
      font-size: 12px;
      overflow-wrap: anywhere;
    }

    .panel-body {
      padding: 16px;
    }

    .account-list {
      display: grid;
      gap: 9px;
    }

    .account-card {
      width: 100%;
      display: grid;
      gap: 8px;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: var(--surface);
      padding: 12px;
      color: var(--text);
      text-align: left;
      transition: border-color 140ms ease, box-shadow 140ms ease, transform 140ms ease;
    }

    .account-card:hover {
      border-color: var(--line-strong);
      transform: translateY(-1px);
    }

    .account-card[aria-pressed="true"] {
      border-color: rgba(15, 118, 110, 0.75);
      box-shadow: 0 0 0 3px rgba(15, 118, 110, 0.13);
    }

    .account-top {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 10px;
    }

    .account-name {
      min-width: 0;
      font-size: 13px;
      font-weight: 700;
      overflow-wrap: anywhere;
    }

    .badge {
      display: inline-flex;
      align-items: center;
      min-height: 22px;
      border: 1px solid transparent;
      border-radius: 999px;
      padding: 3px 8px;
      font-size: 11px;
      font-weight: 650;
      white-space: nowrap;
    }

    .badge.primary {
      border-color: rgba(15, 118, 110, 0.2);
      background: var(--primary-soft);
      color: var(--primary-strong);
    }

    .badge.neutral {
      border-color: var(--line);
      background: var(--surface-2);
      color: var(--muted);
    }

    .account-meta {
      display: grid;
      gap: 4px;
      color: var(--muted);
      font-size: 12px;
      overflow-wrap: anywhere;
    }

    .empty-state {
      display: grid;
      justify-items: center;
      gap: 10px;
      border: 1px dashed var(--line-strong);
      border-radius: 8px;
      background: var(--surface-2);
      padding: 22px 16px;
      color: var(--muted);
      text-align: center;
    }

    .empty-state svg {
      width: 26px;
      height: 26px;
      color: var(--primary);
    }

    form {
      display: grid;
      gap: 15px;
    }

    .form-grid {
      display: grid;
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
      gap: 12px;
    }

    .field {
      display: grid;
      gap: 7px;
      min-width: 0;
    }

    .field-row {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
    }

    label {
      color: #26303d;
      font-size: 12px;
      font-weight: 700;
    }

    .hint {
      color: var(--muted);
      font-size: 12px;
      overflow-wrap: anywhere;
    }

    .control {
      width: 100%;
      min-height: 42px;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: var(--surface);
      color: var(--text);
      padding: 10px 12px;
      outline: none;
      transition: border-color 140ms ease, box-shadow 140ms ease, background 140ms ease;
    }

    textarea.control {
      min-height: 166px;
      resize: vertical;
      line-height: 1.5;
    }

    .control::placeholder {
      color: #9aa4b2;
    }

    .control:hover {
      border-color: var(--line-strong);
    }

    .control:focus,
    button:focus-visible {
      border-color: var(--focus);
      box-shadow: 0 0 0 3px rgba(47, 128, 237, 0.16);
      outline: none;
    }

    .composer-meta {
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
      gap: 10px;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: var(--surface-2);
      padding: 10px;
    }

    .mini-stat {
      min-width: 0;
    }

    .mini-stat span {
      display: block;
      color: var(--muted);
      font-size: 11px;
    }

    .mini-stat strong {
      display: block;
      margin-top: 2px;
      font-size: 12px;
      font-weight: 700;
      overflow-wrap: anywhere;
    }

    .actions {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      border-top: 1px solid var(--line);
      margin-top: 2px;
      padding-top: 15px;
    }

    .button-group {
      display: flex;
      align-items: center;
      justify-content: flex-end;
      gap: 9px;
      flex-wrap: wrap;
    }

    .btn {
      min-height: 38px;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      gap: 8px;
      border-radius: 8px;
      border: 1px solid var(--line);
      background: var(--surface);
      color: var(--text);
      padding: 9px 12px;
      font-weight: 700;
      text-decoration: none;
      transition: background 140ms ease, border-color 140ms ease, transform 140ms ease;
      white-space: nowrap;
    }

    .btn svg {
      width: 16px;
      height: 16px;
      flex: 0 0 auto;
    }

    .btn:hover {
      border-color: var(--line-strong);
      background: #fafbfc;
    }

    .btn.primary {
      border-color: var(--primary);
      background: var(--primary);
      color: #ffffff;
      box-shadow: 0 10px 22px rgba(15, 118, 110, 0.2);
    }

    .btn.primary:hover {
      border-color: var(--primary-strong);
      background: var(--primary-strong);
    }

    .btn[disabled] {
      cursor: not-allowed;
      opacity: 0.58;
      transform: none;
    }

    .notice {
      min-height: 22px;
      display: inline-flex;
      align-items: center;
      gap: 8px;
      color: var(--muted);
      overflow-wrap: anywhere;
    }

    .notice.ok {
      color: var(--ok);
    }

    .notice.error {
      color: var(--danger);
    }

    .activity {
      margin-top: 18px;
    }

    .chat-panel {
      margin-top: 18px;
    }

    .chat-log {
      min-height: 220px;
      max-height: 420px;
      display: grid;
      align-content: start;
      gap: 10px;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: var(--surface-2);
      padding: 12px;
      overflow: auto;
    }

    .chat-bubble {
      width: min(86%, 720px);
      display: grid;
      gap: 5px;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: var(--surface);
      padding: 10px 12px;
    }

    .chat-bubble.user {
      justify-self: end;
      border-color: rgba(15, 118, 110, 0.24);
      background: var(--primary-soft);
    }

    .chat-bubble.agent {
      justify-self: start;
    }

    .chat-role {
      color: var(--muted);
      font-size: 11px;
      font-weight: 700;
      text-transform: uppercase;
    }

    .chat-text {
      white-space: pre-wrap;
      overflow-wrap: anywhere;
    }

    .activity-list {
      display: grid;
      gap: 8px;
    }

    .activity-item {
      display: flex;
      align-items: flex-start;
      justify-content: space-between;
      gap: 12px;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: var(--surface);
      padding: 10px 12px;
    }

    .activity-main {
      min-width: 0;
    }

    .activity-title {
      font-size: 13px;
      font-weight: 700;
      overflow-wrap: anywhere;
    }

    .activity-detail {
      margin-top: 2px;
      color: var(--muted);
      font-size: 12px;
      overflow-wrap: anywhere;
    }

    .activity-time {
      color: var(--muted);
      flex: 0 0 auto;
      font-size: 12px;
      white-space: nowrap;
    }

    .config-layout {
      display: grid;
      grid-template-columns: minmax(0, 1fr) 320px;
      gap: 18px;
      align-items: start;
    }

    .config-editor {
      min-height: 560px;
      font: 13px/1.55 ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", monospace;
      tab-size: 2;
      white-space: pre;
      overflow: auto;
    }

    .config-summary {
      display: grid;
      gap: 10px;
    }

    .config-path {
      font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", monospace;
      font-size: 12px;
      overflow-wrap: anywhere;
    }

    .agent-list {
      display: grid;
      gap: 8px;
      max-height: 360px;
      overflow: auto;
      padding-right: 2px;
    }

    .agent-row {
      display: grid;
      gap: 4px;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: var(--surface);
      padding: 10px;
    }

    .agent-row strong {
      font-size: 13px;
      overflow-wrap: anywhere;
    }

    .agent-row span {
      color: var(--muted);
      font-size: 12px;
      overflow-wrap: anywhere;
    }

    @media (prefers-reduced-motion: reduce) {
      *,
      *::before,
      *::after {
        animation-duration: 1ms !important;
        animation-iteration-count: 1 !important;
        scroll-behavior: auto !important;
        transition-duration: 1ms !important;
      }
    }

    @media (max-width: 900px) {
      .layout {
        grid-template-columns: 1fr;
      }

      .config-layout {
        grid-template-columns: 1fr;
      }

      .form-grid,
      .composer-meta {
        grid-template-columns: 1fr;
      }
    }

    @media (max-width: 680px) {
      .shell {
        width: min(100vw - 24px, 1240px);
      }

      .topbar-inner {
        align-items: flex-start;
        flex-direction: column;
        padding: 14px 0;
      }

      .toolbar {
        justify-content: flex-start;
      }

      .panel-head,
      .actions,
      .activity-item {
        align-items: stretch;
        flex-direction: column;
      }

      .button-group {
        justify-content: stretch;
      }

      .btn {
        width: 100%;
      }
    }
  </style>
</head>
<body>
  <header class="topbar">
    <div class="shell topbar-inner">
      <div class="brand">
        <div class="brand-mark" aria-hidden="true">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M4 8.5c0-2.2 1.8-4 4-4h8c2.2 0 4 1.8 4 4v4.8c0 2.2-1.8 4-4 4h-2.8L9 20.5v-3.2H8c-2.2 0-4-1.8-4-4V8.5Z"></path>
            <path d="M8 9.5h8"></path>
            <path d="M8 13h5"></path>
          </svg>
        </div>
        <div>
          <h1>Welinker</h1>
          <p class="brand-subtitle">iLink message console</p>
        </div>
      </div>
      <div class="toolbar" aria-live="polite">
        <div class="status-pill">
          <span id="statusDot" class="dot"></span>
          <span id="statusText">Loading</span>
        </div>
        <div class="metric">
          <span>Accounts</span>
          <strong id="accountMetric">0</strong>
        </div>
        <div class="endpoint-pill">
          <span>POST</span>
          <code>/api/send</code>
        </div>
      </div>
    </div>
  </header>

  <main class="shell">
    <nav class="view-tabs" aria-label="Welinker sections">
      <button class="tab-button" id="messageTab" type="button" aria-selected="true" data-view-target="messageView">Message</button>
      <button class="tab-button" id="configTab" type="button" aria-selected="false" data-view-target="configView">Config</button>
    </nav>

    <div class="view layout" id="messageView">
      <section class="panel" aria-labelledby="accountsTitle">
        <div class="panel-head">
          <div class="panel-title">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"></path>
              <circle cx="9" cy="7" r="4"></circle>
              <path d="M22 21v-2a4 4 0 0 0-3-3.87"></path>
              <path d="M16 3.13a4 4 0 0 1 0 7.75"></path>
            </svg>
            <div>
              <h2 id="accountsTitle">Accounts</h2>
              <p class="panel-description">Active iLink sessions</p>
            </div>
          </div>
          <button class="btn" id="refresh" type="button" title="Refresh accounts">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M21 12a9 9 0 1 1-2.64-6.36"></path>
              <path d="M21 3v6h-6"></path>
            </svg>
            Refresh
          </button>
        </div>
        <div class="panel-body">
          <div id="accounts" class="account-list"></div>
        </div>
      </section>

      <div>
        <section class="panel feature" aria-labelledby="sendTitle">
          <div class="panel-head">
            <div class="panel-title">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="m22 2-7 20-4-9-9-4Z"></path>
                <path d="M22 2 11 13"></path>
              </svg>
              <div>
                <h2 id="sendTitle">Send Message</h2>
                <p class="panel-description">Text, markdown image links, and direct media URL</p>
              </div>
            </div>
            <span class="badge primary" id="selectedBadge">No account</span>
          </div>
          <div class="panel-body">
            <form id="sendForm">
              <div class="form-grid">
                <div class="field">
                  <div class="field-row">
                    <label for="accountSelect">Account</label>
                    <span class="hint">Required</span>
                  </div>
                  <select class="control" id="accountSelect" name="account_id"></select>
                </div>
                <div class="field">
                  <div class="field-row">
                    <label for="to">Recipient</label>
                    <span class="hint">user_id@im.wechat</span>
                  </div>
                  <input class="control" id="to" name="to" autocomplete="off" placeholder="user_id@im.wechat" required />
                </div>
              </div>

              <div class="field">
                <div class="field-row">
                  <label for="text">Text</label>
                  <span class="hint" id="textCounter">0 chars</span>
                </div>
                <textarea class="control" id="text" name="text" placeholder="Type message text. Markdown image links are sent as media automatically."></textarea>
              </div>

              <div class="field">
                <div class="field-row">
                  <label for="mediaUrl">Media URL</label>
                  <span class="hint">Optional</span>
                </div>
                <input class="control" id="mediaUrl" name="media_url" autocomplete="off" placeholder="https://example.com/image.png" />
              </div>

              <div class="composer-meta" aria-label="Message details">
                <div class="mini-stat">
                  <span>Selected account</span>
                  <strong id="selectedAccount">None</strong>
                </div>
                <div class="mini-stat">
                  <span>Recipient</span>
                  <strong id="recipientPreview">Not set</strong>
                </div>
                <div class="mini-stat">
                  <span>Attachment</span>
                  <strong id="mediaPreview">None</strong>
                </div>
              </div>

              <div class="actions">
                <div id="notice" class="notice" aria-live="polite"></div>
                <div class="button-group">
                  <button class="btn" id="clearForm" type="button">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                      <path d="M3 6h18"></path>
                      <path d="M8 6V4h8v2"></path>
                      <path d="m19 6-1 14H6L5 6"></path>
                      <path d="M10 11v5"></path>
                      <path d="M14 11v5"></path>
                    </svg>
                    Clear
                  </button>
                  <button class="btn primary" id="sendButton" type="submit">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                      <path d="m22 2-7 20-4-9-9-4Z"></path>
                      <path d="M22 2 11 13"></path>
                    </svg>
                    Send
                  </button>
                </div>
              </div>
            </form>
          </div>
        </section>

        <section class="panel chat-panel" aria-labelledby="chatTitle">
          <div class="panel-head">
            <div class="panel-title">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M21 15a4 4 0 0 1-4 4H8l-5 3V7a4 4 0 0 1 4-4h10a4 4 0 0 1 4 4Z"></path>
                <path d="M8 9h8"></path>
                <path d="M8 13h5"></path>
              </svg>
              <div>
                <h2 id="chatTitle">Local Agent Chat</h2>
                <p class="panel-description">Talk to Welinker without a WeChat login</p>
              </div>
            </div>
            <span class="badge neutral" id="chatSessionBadge">web</span>
          </div>
          <div class="panel-body">
            <form id="chatForm">
              <div class="form-grid">
                <div class="field">
                  <div class="field-row">
                    <label for="chatAgent">Agent</label>
                    <span class="hint">Optional</span>
                  </div>
                  <input class="control" id="chatAgent" name="agent" autocomplete="off" placeholder="default, hermes, hermes-http, codex" />
                </div>
                <div class="field">
                  <div class="field-row">
                    <label for="chatConversation">Conversation</label>
                    <span class="hint">Keeps context</span>
                  </div>
                  <input class="control" id="chatConversation" name="conversation_id" autocomplete="off" value="web" />
                </div>
              </div>
              <div class="chat-log" id="chatLog" aria-live="polite"></div>
              <div class="field">
                <div class="field-row">
                  <label for="chatMessage">Message</label>
                  <span class="hint">Use /new to reset</span>
                </div>
                <textarea class="control" id="chatMessage" name="message" placeholder="Ask the configured agent anything."></textarea>
              </div>
              <div class="actions">
                <div id="chatNotice" class="notice" aria-live="polite"></div>
                <div class="button-group">
                  <button class="btn" id="newChat" type="button">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                      <path d="M12 5v14"></path>
                      <path d="M5 12h14"></path>
                    </svg>
                    New
                  </button>
                  <button class="btn primary" id="chatButton" type="submit">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                      <path d="m22 2-7 20-4-9-9-4Z"></path>
                      <path d="M22 2 11 13"></path>
                    </svg>
                    Send
                  </button>
                </div>
              </div>
            </form>
          </div>
        </section>

        <section class="panel activity" aria-labelledby="activityTitle">
          <div class="panel-head">
            <div class="panel-title">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M3 12h4l3 8 4-16 3 8h4"></path>
              </svg>
              <div>
                <h2 id="activityTitle">Activity</h2>
                <p class="panel-description">Recent console events</p>
              </div>
            </div>
            <span class="badge neutral" id="activityCount">0 events</span>
          </div>
          <div class="panel-body">
            <div id="activity" class="activity-list"></div>
          </div>
        </section>
      </div>
    </div>

    <div class="view config-layout" id="configView" hidden>
      <section class="panel feature" aria-labelledby="configTitle">
        <div class="panel-head">
          <div class="panel-title">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.38a2 2 0 0 0-.73-2.73l-.15-.09a2 2 0 0 1-1-1.74v-.51a2 2 0 0 1 1-1.72l.15-.1a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2Z"></path>
              <circle cx="12" cy="12" r="3"></circle>
            </svg>
            <div>
              <h2 id="configTitle">Configuration</h2>
              <p class="panel-description">Persistent JSON file; restart applies runtime changes</p>
            </div>
          </div>
          <span class="badge neutral" id="configState">Not loaded</span>
        </div>
        <div class="panel-body">
          <form id="configForm">
            <div class="field">
              <div class="field-row">
                <label for="configEditor">Config JSON</label>
                <span class="hint" id="configPath">-</span>
              </div>
              <textarea class="control config-editor" id="configEditor" spellcheck="false" autocomplete="off"></textarea>
            </div>
            <div class="actions">
              <div id="configNotice" class="notice" aria-live="polite"></div>
              <div class="button-group">
                <button class="btn" id="reloadConfig" type="button">
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <path d="M21 12a9 9 0 1 1-2.64-6.36"></path>
                    <path d="M21 3v6h-6"></path>
                  </svg>
                  Reload
                </button>
                <button class="btn" id="formatConfig" type="button">
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <path d="M21 10H3"></path>
                    <path d="M21 6H3"></path>
                    <path d="M21 14H3"></path>
                    <path d="M21 18H3"></path>
                  </svg>
                  Format
                </button>
                <button class="btn primary" id="saveConfig" type="submit">
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                    <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2Z"></path>
                    <path d="M17 21v-8H7v8"></path>
                    <path d="M7 3v5h8"></path>
                  </svg>
                  Save
                </button>
              </div>
            </div>
          </form>
        </div>
      </section>

      <section class="panel" aria-labelledby="configSummaryTitle">
        <div class="panel-head">
          <div class="panel-title">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M8 6h13"></path>
              <path d="M8 12h13"></path>
              <path d="M8 18h13"></path>
              <path d="M3 6h.01"></path>
              <path d="M3 12h.01"></path>
              <path d="M3 18h.01"></path>
            </svg>
            <div>
              <h2 id="configSummaryTitle">Summary</h2>
              <p class="panel-description">Current editor contents</p>
            </div>
          </div>
        </div>
        <div class="panel-body config-summary">
          <div class="mini-stat">
            <span>Config path</span>
            <strong class="config-path" id="configPathFull">-</strong>
          </div>
          <div class="mini-stat">
            <span>Default agent</span>
            <strong id="configDefaultAgent">-</strong>
          </div>
          <div class="mini-stat">
            <span>API address</span>
            <strong id="configApiAddr">-</strong>
          </div>
          <div class="mini-stat">
            <span>Agents</span>
            <strong id="configAgentCount">0</strong>
          </div>
          <div class="agent-list" id="configAgents"></div>
        </div>
      </section>
    </div>
  </main>

  <script>
    const accountsEl = document.querySelector('#accounts');
    const selectEl = document.querySelector('#accountSelect');
    const statusDotEl = document.querySelector('#statusDot');
    const statusEl = document.querySelector('#statusText');
    const accountMetricEl = document.querySelector('#accountMetric');
    const selectedBadgeEl = document.querySelector('#selectedBadge');
    const selectedAccountEl = document.querySelector('#selectedAccount');
    const recipientPreviewEl = document.querySelector('#recipientPreview');
    const mediaPreviewEl = document.querySelector('#mediaPreview');
    const textCounterEl = document.querySelector('#textCounter');
    const noticeEl = document.querySelector('#notice');
    const activityEl = document.querySelector('#activity');
    const activityCountEl = document.querySelector('#activityCount');
    const sendButtonEl = document.querySelector('#sendButton');
    const textEl = document.querySelector('#text');
    const toEl = document.querySelector('#to');
    const mediaUrlEl = document.querySelector('#mediaUrl');
    const chatAgentEl = document.querySelector('#chatAgent');
    const chatConversationEl = document.querySelector('#chatConversation');
    const chatSessionBadgeEl = document.querySelector('#chatSessionBadge');
    const chatLogEl = document.querySelector('#chatLog');
    const chatMessageEl = document.querySelector('#chatMessage');
    const chatNoticeEl = document.querySelector('#chatNotice');
    const chatButtonEl = document.querySelector('#chatButton');
    const configEditorEl = document.querySelector('#configEditor');
    const configPathEl = document.querySelector('#configPath');
    const configPathFullEl = document.querySelector('#configPathFull');
    const configStateEl = document.querySelector('#configState');
    const configNoticeEl = document.querySelector('#configNotice');
    const configDefaultAgentEl = document.querySelector('#configDefaultAgent');
    const configApiAddrEl = document.querySelector('#configApiAddr');
    const configAgentCountEl = document.querySelector('#configAgentCount');
    const configAgentsEl = document.querySelector('#configAgents');
    const saveConfigEl = document.querySelector('#saveConfig');
    let accounts = [];
    let selected = '';
    let activity = [];
    let configLoaded = false;

    function icon(name) {
      const icons = {
        ok: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 6 9 17l-5-5"></path></svg>',
        error: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10"></circle><path d="m15 9-6 6"></path><path d="m9 9 6 6"></path></svg>',
        pending: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 6v6l4 2"></path><circle cx="12" cy="12" r="10"></circle></svg>',
      };
      return icons[name] || '';
    }

    function setNotice(text, type = '') {
      noticeEl.className = `notice ${type}`;
      noticeEl.innerHTML = text ? `${icon(type || 'pending')}<span></span>` : '';
      const span = noticeEl.querySelector('span');
      if (span) span.textContent = text;
    }

    function setConfigNotice(text, type = '') {
      configNoticeEl.className = `notice ${type}`;
      configNoticeEl.innerHTML = text ? `${icon(type || 'pending')}<span></span>` : '';
      const span = configNoticeEl.querySelector('span');
      if (span) span.textContent = text;
      configStateEl.textContent = text || (configLoaded ? 'Loaded' : 'Not loaded');
      configStateEl.className = type === 'ok' ? 'badge primary' : 'badge neutral';
    }

    function setChatNotice(text, type = '') {
      chatNoticeEl.className = `notice ${type}`;
      chatNoticeEl.innerHTML = text ? `${icon(type || 'pending')}<span></span>` : '';
      const span = chatNoticeEl.querySelector('span');
      if (span) span.textContent = text;
    }

    function setStatus(text, type = '') {
      statusEl.textContent = text;
      statusDotEl.className = `dot ${type}`;
    }

    function accountLabel(account) {
      return account.account_id || account.normalized_id || 'unknown';
    }

    function selectedAccount() {
      return accounts.find((account) => account.account_id === selected);
    }

    function shortValue(value, fallback = 'None') {
      if (!value) return fallback;
      return value.length > 42 ? `${value.slice(0, 39)}...` : value;
    }

    function addActivity(title, detail, type = 'neutral') {
      activity = [
        {
          title,
          detail,
          type,
          time: new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' }),
        },
        ...activity,
      ].slice(0, 5);
      renderActivity();
    }

    function renderActivity() {
      activityEl.innerHTML = '';
      activityCountEl.textContent = `${activity.length} event${activity.length === 1 ? '' : 's'}`;
      if (!activity.length) {
        const empty = document.createElement('div');
        empty.className = 'empty-state';
        empty.textContent = 'No events yet.';
        activityEl.appendChild(empty);
        return;
      }
      for (const item of activity) {
        const row = document.createElement('div');
        row.className = 'activity-item';

        const main = document.createElement('div');
        main.className = 'activity-main';

        const title = document.createElement('div');
        title.className = 'activity-title';
        title.textContent = item.title;

        const detail = document.createElement('div');
        detail.className = 'activity-detail';
        detail.textContent = item.detail;

        const time = document.createElement('div');
        time.className = 'activity-time';
        time.textContent = item.time;

        main.append(title, detail);
        row.append(main, time);
        activityEl.appendChild(row);
      }
    }

    function parseConfigEditor() {
      const value = configEditorEl.value.trim();
      if (!value) return {};
      return JSON.parse(value);
    }

    function renderConfigSummary(config, path = '') {
      const agents = config && config.agents && typeof config.agents === 'object' ? config.agents : {};
      const agentNames = Object.keys(agents).sort();
      configPathEl.textContent = path ? shortValue(path, '-') : '-';
      configPathFullEl.textContent = path || '-';
      configDefaultAgentEl.textContent = config.default_agent || '-';
      configApiAddrEl.textContent = config.api_addr || '127.0.0.1:18011';
      configAgentCountEl.textContent = String(agentNames.length);
      configAgentsEl.innerHTML = '';

      if (!agentNames.length) {
        const empty = document.createElement('div');
        empty.className = 'empty-state';
        empty.textContent = 'No agents configured.';
        configAgentsEl.appendChild(empty);
        return;
      }

      for (const name of agentNames) {
        const agent = agents[name] || {};
        const row = document.createElement('div');
        row.className = 'agent-row';

        const title = document.createElement('strong');
        title.textContent = name;

        const detail = document.createElement('span');
        detail.textContent = `${agent.type || 'unknown'} · ${agent.model || agent.command || agent.endpoint || 'default'}`;

        row.append(title, detail);
        configAgentsEl.appendChild(row);
      }
    }

    function updateConfigSummaryFromEditor() {
      try {
        renderConfigSummary(parseConfigEditor(), configPathFullEl.textContent === '-' ? '' : configPathFullEl.textContent);
        configNoticeEl.innerHTML = '';
        configNoticeEl.className = 'notice';
        configStateEl.textContent = configLoaded ? 'Edited' : 'Not loaded';
        configStateEl.className = 'badge neutral';
      } catch (err) {
        setConfigNotice(err.message, 'error');
      }
    }

    async function loadConfig() {
      setConfigNotice('Loading', 'pending');
      const resp = await fetch('/api/config');
      if (!resp.ok) throw new Error(await resp.text());
      const payload = await resp.json();
      configLoaded = true;
      configEditorEl.value = JSON.stringify(payload.config || {}, null, 2);
      renderConfigSummary(payload.config || {}, payload.path || '');
      setConfigNotice('Loaded', 'ok');
      addActivity('Config loaded', payload.path || 'config.json', 'ok');
    }

    async function saveConfig() {
      const cfg = parseConfigEditor();
      configEditorEl.value = JSON.stringify(cfg, null, 2);
      setConfigNotice('Saving', 'pending');
      saveConfigEl.disabled = true;
      try {
        const resp = await fetch('/api/config', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(cfg),
        });
        if (!resp.ok) throw new Error(await resp.text());
        const payload = await resp.json();
        configLoaded = true;
        configEditorEl.value = JSON.stringify(payload.config || {}, null, 2);
        renderConfigSummary(payload.config || {}, payload.path || '');
        const savedMessage = payload.reload_required ? 'Saved; restart to apply runtime changes' : 'Saved';
        setConfigNotice(savedMessage, 'ok');
        addActivity('Config saved', savedMessage, 'ok');
      } finally {
        saveConfigEl.disabled = false;
      }
    }

    function renderAccounts() {
      accountsEl.innerHTML = '';
      selectEl.innerHTML = '';
      accountMetricEl.textContent = String(accounts.length);

      if (!accounts.length) {
        selected = '';
        const empty = document.createElement('div');
        empty.className = 'empty-state';
        empty.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 9v4"></path><path d="M12 17h.01"></path><path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z"></path></svg><span>No accounts are logged in.</span>';
        accountsEl.appendChild(empty);

        const option = document.createElement('option');
        option.value = '';
        option.textContent = 'No account';
        selectEl.appendChild(option);
        updatePreview();
        return;
      }

      if (!selected || !accounts.some((account) => account.account_id === selected)) {
        selected = accounts[0].account_id;
      }

      for (const account of accounts) {
        const id = account.account_id;
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'account-card';
        button.setAttribute('aria-pressed', String(id === selected));

        const top = document.createElement('div');
        top.className = 'account-top';

        const name = document.createElement('div');
        name.className = 'account-name';
        name.textContent = accountLabel(account);

        const badge = document.createElement('span');
        badge.className = id === selected ? 'badge primary' : 'badge neutral';
        badge.textContent = id === selected ? 'Selected' : 'Ready';

        const meta = document.createElement('div');
        meta.className = 'account-meta';

        const user = document.createElement('span');
        user.textContent = account.user_id || 'user unknown';

        const base = document.createElement('span');
        base.textContent = account.base_url || 'default endpoint';

        top.append(name, badge);
        meta.append(user, base);
        button.append(top, meta);
        button.addEventListener('click', () => {
          selected = id;
          selectEl.value = id;
          renderAccounts();
        });
        accountsEl.appendChild(button);

        const option = document.createElement('option');
        option.value = id;
        option.textContent = accountLabel(account);
        selectEl.appendChild(option);
      }

      selectEl.value = selected;
      updatePreview();
    }

    function updatePreview() {
      const account = selectedAccount();
      const label = account ? accountLabel(account) : 'No account';
      selectedBadgeEl.textContent = label;
      selectedAccountEl.textContent = shortValue(label);
      recipientPreviewEl.textContent = shortValue(toEl.value.trim(), 'Not set');
      mediaPreviewEl.textContent = shortValue(mediaUrlEl.value.trim(), 'None');
      textCounterEl.textContent = `${textEl.value.length} chars`;
    }

    function appendChat(role, text, label = '') {
      if (!chatLogEl.children.length) {
        chatLogEl.innerHTML = '';
      }
      const bubble = document.createElement('div');
      bubble.className = `chat-bubble ${role}`;

      const roleEl = document.createElement('div');
      roleEl.className = 'chat-role';
      roleEl.textContent = label || role;

      const textNode = document.createElement('div');
      textNode.className = 'chat-text';
      textNode.textContent = text;

      bubble.append(roleEl, textNode);
      chatLogEl.appendChild(bubble);
      chatLogEl.scrollTop = chatLogEl.scrollHeight;
    }

    function renderEmptyChat() {
      chatLogEl.innerHTML = '<div class="empty-state">No local chat messages yet.</div>';
    }

    async function sendLocalChat(messageOverride = '') {
      const message = messageOverride || chatMessageEl.value.trim();
      if (!message) {
        setChatNotice('Message is required', 'error');
        return;
      }
      const conversationId = chatConversationEl.value.trim() || 'web';
      chatConversationEl.value = conversationId;
      chatSessionBadgeEl.textContent = shortValue(conversationId, 'web');
      setChatNotice('Sending', 'pending');
      chatButtonEl.disabled = true;
      if (!messageOverride) {
        appendChat('user', message, 'You');
      }
      try {
        const resp = await fetch('/api/chat', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            conversation_id: conversationId,
            agent: chatAgentEl.value.trim(),
            message,
          }),
        });
        if (!resp.ok) throw new Error(await resp.text());
        const payload = await resp.json();
        appendChat('agent', payload.reply || '', payload.agent || 'Agent');
        setChatNotice('Ready', 'ok');
        addActivity('Local chat', payload.agent || 'default agent', 'ok');
      } catch (err) {
        const detail = err.message || String(err);
        setChatNotice(detail, 'error');
        addActivity('Local chat failed', detail, 'error');
      } finally {
        chatButtonEl.disabled = false;
        if (!messageOverride) {
          chatMessageEl.value = '';
        }
      }
    }

    async function load() {
      setNotice('');
      setStatus('Loading', '');
      const [statusResp, accountsResp] = await Promise.all([
        fetch('/api/status'),
        fetch('/api/accounts'),
      ]);
      if (!statusResp.ok) throw new Error(await statusResp.text());
      if (!accountsResp.ok) throw new Error(await accountsResp.text());
      const status = await statusResp.json();
      accounts = await accountsResp.json();
      setStatus(`${status.status} · ${status.account_count} account${status.account_count === 1 ? '' : 's'}`, 'ok');
      renderAccounts();
      addActivity('Status refreshed', `${status.account_count} account${status.account_count === 1 ? '' : 's'} available`, 'ok');
    }

    document.querySelector('#refresh').addEventListener('click', () => {
      load().catch((err) => {
        setStatus('offline', 'error');
        setNotice(err.message, 'error');
        addActivity('Refresh failed', err.message, 'error');
      });
    });

    document.querySelectorAll('.tab-button').forEach((button) => {
      button.addEventListener('click', () => {
        document.querySelectorAll('.tab-button').forEach((tab) => {
          tab.setAttribute('aria-selected', String(tab === button));
        });
        document.querySelectorAll('.view').forEach((view) => {
          view.hidden = view.id !== button.dataset.viewTarget;
        });
        if (button.dataset.viewTarget === 'configView' && !configLoaded) {
          loadConfig().catch((err) => {
            setConfigNotice(err.message, 'error');
            addActivity('Config load failed', err.message, 'error');
          });
        }
      });
    });

    selectEl.addEventListener('change', () => {
      selected = selectEl.value;
      renderAccounts();
    });

    textEl.addEventListener('input', updatePreview);
    toEl.addEventListener('input', updatePreview);
    mediaUrlEl.addEventListener('input', updatePreview);
    chatConversationEl.addEventListener('input', () => {
      chatSessionBadgeEl.textContent = shortValue(chatConversationEl.value.trim(), 'web');
    });
    configEditorEl.addEventListener('input', updateConfigSummaryFromEditor);

    document.querySelector('#chatForm').addEventListener('submit', (event) => {
      event.preventDefault();
      sendLocalChat();
    });

    document.querySelector('#newChat').addEventListener('click', () => {
      const id = `web-${Date.now().toString(36)}`;
      chatConversationEl.value = id;
      chatSessionBadgeEl.textContent = shortValue(id, 'web');
      renderEmptyChat();
      sendLocalChat('/new');
    });

    document.querySelector('#reloadConfig').addEventListener('click', () => {
      loadConfig().catch((err) => {
        setConfigNotice(err.message, 'error');
        addActivity('Config reload failed', err.message, 'error');
      });
    });

    document.querySelector('#formatConfig').addEventListener('click', () => {
      try {
        const cfg = parseConfigEditor();
        configEditorEl.value = JSON.stringify(cfg, null, 2);
        renderConfigSummary(cfg, configPathFullEl.textContent === '-' ? '' : configPathFullEl.textContent);
        setConfigNotice('Formatted', 'ok');
      } catch (err) {
        setConfigNotice(err.message, 'error');
      }
    });

    document.querySelector('#configForm').addEventListener('submit', (event) => {
      event.preventDefault();
      saveConfig().catch((err) => {
        setConfigNotice(err.message, 'error');
        addActivity('Config save failed', err.message, 'error');
      });
    });

    document.querySelector('#clearForm').addEventListener('click', () => {
      textEl.value = '';
      toEl.value = '';
      mediaUrlEl.value = '';
      setNotice('');
      updatePreview();
      addActivity('Composer cleared', 'Draft fields were reset');
    });

    document.querySelector('#sendForm').addEventListener('submit', async (event) => {
      event.preventDefault();
      setNotice('Sending', 'pending');
      sendButtonEl.disabled = true;
      const payload = {
        account_id: selectEl.value,
        to: toEl.value.trim(),
        text: textEl.value,
        media_url: mediaUrlEl.value.trim(),
      };
      try {
        const resp = await fetch('/api/send', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(payload),
        });
        if (!resp.ok) throw new Error(await resp.text());
        setNotice('Sent', 'ok');
        addActivity('Message sent', `Delivered request to ${payload.to}`, 'ok');
      } catch (err) {
        const message = err.message || String(err);
        setNotice(message, 'error');
        addActivity('Send failed', message, 'error');
      } finally {
        sendButtonEl.disabled = false;
      }
    });

    renderActivity();
    renderEmptyChat();
    updatePreview();
    load().catch((err) => {
      setStatus('offline', 'error');
      setNotice(err.message, 'error');
      addActivity('API unavailable', err.message, 'error');
    });
  </script>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::Handler;
    use futures::FutureExt;

    #[tokio::test]
    async fn serves_web_ui() {
        let server = Server::new(Vec::new(), String::new());
        let response = server
            .handle_raw(b"GET / HTTP/1.1\r\nHost: test\r\n\r\n")
            .await;
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("<title>Welinker</title>"));
    }

    #[tokio::test]
    async fn serves_status_json() {
        let server = Server::new(Vec::new(), String::new());
        let response = server
            .handle_raw(b"GET /api/status HTTP/1.1\r\nHost: test\r\n\r\n")
            .await;
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains(r#""account_count":0"#));
    }

    #[tokio::test]
    async fn send_requires_account() {
        let server = Server::new(Vec::new(), String::new());
        let response = server
            .handle_raw(
                b"POST /api/send HTTP/1.1\r\nHost: test\r\n\r\n{\"to\":\"user_id@im.wechat\",\"text\":\"hello\"}",
            )
            .await;
        assert!(response.starts_with("HTTP/1.1 503 Service Unavailable"));
    }

    #[tokio::test]
    async fn local_chat_works_without_accounts() {
        let factory = Arc::new(|_name: String| async { None }.boxed());
        let save_default = Arc::new(|_name: String| async { Ok(()) }.boxed());
        let handler = Handler::new(factory, save_default);
        let server = Server::new(Vec::new(), String::new()).with_handler(handler);
        let response = server
            .handle_raw(b"POST /api/chat HTTP/1.1\r\nHost: test\r\n\r\n{\"message\":\"hello\"}")
            .await;
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains(r#""reply":"[echo] hello""#));
    }

    #[test]
    fn parses_content_length_case_insensitively() {
        let head = b"POST /api/chat HTTP/1.1\r\nHost: test\r\ncontent-length: 17\r\n";
        assert_eq!(parse_content_length(head).unwrap(), 17);
    }
}
