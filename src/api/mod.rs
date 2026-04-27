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

mod web_assets {
    include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));
}

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
                let _ = stream.write_all(&response).await;
            });
        }
    }

    async fn handle_raw(&self, data: &[u8]) -> Vec<u8> {
        let request = String::from_utf8_lossy(data);
        let Some((head, body)) = request.split_once("\r\n\r\n") else {
            return http_response(400, "text/plain", "invalid HTTP request");
        };
        let first = head.lines().next().unwrap_or_default();
        let Some((method, path)) = request_line_parts(first) else {
            return http_response(400, "text/plain", "invalid HTTP request");
        };
        if method == "GET" && (path == "/" || path == "/index.html") {
            return http_response_bytes(
                200,
                "text/html; charset=utf-8",
                web_assets::WEB_INDEX.as_bytes(),
            );
        }
        if method == "GET" {
            let asset_path = path
                .trim_start_matches('/')
                .split_once('?')
                .map(|(path, _)| path)
                .unwrap_or_else(|| path.trim_start_matches('/'));
            if let Some(asset) = web_assets::web_asset(asset_path) {
                return http_response_bytes(200, asset.content_type, asset.bytes);
            }
        }
        if method == "GET" && path == "/health" {
            return http_response(200, "text/plain", "ok\n");
        }
        if method == "GET" && path == "/api/status" {
            return json_response(
                200,
                &StatusResponse {
                    status: "ok",
                    account_count: self.clients.len(),
                },
            );
        }
        if method == "GET" && path == "/api/accounts" {
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
        if method == "GET" && path == "/api/config" {
            return self.handle_get_config();
        }
        if method == "PUT" && path == "/api/config" {
            return self.handle_put_config(body);
        }
        if method == "POST" && path == "/api/chat" {
            return self.handle_chat(body).await;
        }
        if method == "POST" && path == "/api/send" {
            return self.handle_send(body).await;
        }
        http_response(404, "text/plain", "not found")
    }

    fn handle_get_config(&self) -> Vec<u8> {
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

    fn handle_put_config(&self, body: &str) -> Vec<u8> {
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

    async fn handle_chat(&self, body: &str) -> Vec<u8> {
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

    async fn handle_send(&self, body: &str) -> Vec<u8> {
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

async fn read_http_request(stream: &mut TcpStream) -> std::result::Result<Vec<u8>, Vec<u8>> {
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

fn request_line_parts(first: &str) -> Option<(&str, &str)> {
    let mut parts = first.split_whitespace();
    let method = parts.next()?;
    let raw_path = parts.next()?;
    let path = raw_path
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(raw_path);
    Some((method, path))
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

fn http_response(status: u16, content_type: &str, body: &str) -> Vec<u8> {
    http_response_bytes(status, content_type, body.as_bytes())
}

fn http_response_bytes(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
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
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut response = header.into_bytes();
    response.extend_from_slice(body);
    response
}

fn json_response<T: Serialize>(status: u16, value: &T) -> Vec<u8> {
    match serde_json::to_string(value) {
        Ok(body) => http_response(status, "application/json", &body),
        Err(err) => http_response(
            500,
            "text/plain",
            &format!("failed to serialize response: {err}"),
        ),
    }
}

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
        assert!(response.starts_with(b"HTTP/1.1 200 OK"));
        let response = String::from_utf8(response).unwrap();
        assert!(response.contains("<title>Welinker</title>"));
    }

    #[tokio::test]
    async fn serves_web_asset() {
        let server = Server::new(Vec::new(), String::new());
        let asset_path = web_assets::WEB_ASSET_PATHS
            .first()
            .expect("Vite build should emit at least one asset");
        let request = format!("GET /{asset_path} HTTP/1.1\r\nHost: test\r\n\r\n");
        let response = server.handle_raw(request.as_bytes()).await;
        assert!(response.starts_with(b"HTTP/1.1 200 OK"));
    }

    #[tokio::test]
    async fn serves_status_json() {
        let server = Server::new(Vec::new(), String::new());
        let response = server
            .handle_raw(b"GET /api/status HTTP/1.1\r\nHost: test\r\n\r\n")
            .await;
        assert!(response.starts_with(b"HTTP/1.1 200 OK"));
        let response = String::from_utf8(response).unwrap();
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
        assert!(response.starts_with(b"HTTP/1.1 503 Service Unavailable"));
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
        assert!(response.starts_with(b"HTTP/1.1 200 OK"));
        let response = String::from_utf8(response).unwrap();
        assert!(response.contains(r#""reply":"[echo] hello""#));
    }

    #[test]
    fn parses_content_length_case_insensitively() {
        let head = b"POST /api/chat HTTP/1.1\r\nHost: test\r\ncontent-length: 17\r\n";
        assert_eq!(parse_content_length(head).unwrap(), 17);
    }
}
