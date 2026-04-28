use super::*;
use anyhow::{Context, Result};
use base64::Engine;
use rand::RngCore;
use reqwest::{Client as HttpClient, StatusCode};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
const ILINK_APP_ID: &str = "bot";

#[derive(Clone)]
pub struct Client {
    base_url: String,
    bot_token: String,
    bot_id: String,
    user_id: String,
    http: HttpClient,
    wechat_uin: String,
    route_tag: Option<String>,
}

impl Client {
    pub fn new_with_route_tag(creds: &Credentials, route_tag: Option<String>) -> Self {
        let base_url = if creds.baseurl.is_empty() {
            DEFAULT_BASE_URL.to_string()
        } else {
            creds.baseurl.clone()
        };
        Self {
            base_url,
            bot_token: creds.bot_token.clone(),
            bot_id: creds.ilink_bot_id.clone(),
            user_id: creds.ilink_user_id.clone(),
            http: HttpClient::new(),
            wechat_uin: generate_wechat_uin(),
            route_tag,
        }
    }

    pub fn unauthenticated() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            bot_token: String::new(),
            bot_id: String::new(),
            user_id: String::new(),
            http: HttpClient::new(),
            wechat_uin: generate_wechat_uin(),
            route_tag: None,
        }
    }

    pub fn bot_id(&self) -> &str {
        &self.bot_id
    }

    pub fn normalized_bot_id(&self) -> String {
        normalize_account_id(&self.bot_id)
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn get_updates(&self, buf: &str, timeout: Duration) -> Result<GetUpdatesResponse> {
        let req = GetUpdatesRequest {
            get_updates_buf: buf.to_string(),
            base_info: BaseInfo::channel(),
        };
        self.post_with_timeout("/ilink/bot/getupdates", &req, Some(timeout))
            .await
    }

    pub async fn send_message(&self, msg: &SendMessageRequest) -> Result<SendMessageResponse> {
        self.post("/ilink/bot/sendmessage", msg).await
    }

    pub async fn get_config(
        &self,
        user_id: &str,
        context_token: &str,
    ) -> Result<GetConfigResponse> {
        let req = GetConfigRequest {
            ilink_user_id: user_id.to_string(),
            context_token: context_token.to_string(),
            base_info: BaseInfo::channel(),
        };
        self.post("/ilink/bot/getconfig", &req).await
    }

    pub async fn send_typing(&self, user_id: &str, typing_ticket: &str, status: i32) -> Result<()> {
        let req = SendTypingRequest {
            ilink_user_id: user_id.to_string(),
            typing_ticket: typing_ticket.to_string(),
            status,
            base_info: BaseInfo::channel(),
        };
        let resp: SendTypingResponse = self.post("/ilink/bot/sendtyping", &req).await?;
        if resp.ret != 0 {
            anyhow::bail!("sendtyping failed: ret={} errmsg={}", resp.ret, resp.errmsg);
        }
        Ok(())
    }

    pub async fn get_upload_url(&self, req: &GetUploadUrlRequest) -> Result<GetUploadUrlResponse> {
        self.post("/ilink/bot/getuploadurl", req).await
    }

    pub(crate) async fn get<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = self
            .http
            .get(url)
            .headers(self.common_headers())
            .send()
            .await?;
        read_json(resp).await
    }

    async fn post<T: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<R> {
        self.post_with_timeout(path, body, None).await
    }

    async fn post_with_timeout<T: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &T,
        timeout: Option<Duration>,
    ) -> Result<R> {
        let body = serde_json::to_vec(body)?;
        let mut req = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .header("Content-Type", "application/json")
            .header("AuthorizationType", "ilink_bot_token")
            .header("Authorization", format!("Bearer {}", self.bot_token))
            .header("Content-Length", body.len().to_string())
            .header("X-WECHAT-UIN", &self.wechat_uin)
            .headers(self.common_headers())
            .body(body);
        if let Some(timeout) = timeout {
            req = req.timeout(timeout);
        }
        let resp = req.send().await?;
        read_json(resp).await
    }

    fn common_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("iLink-App-Id", ILINK_APP_ID.parse().expect("header value"));
        headers.insert(
            "iLink-App-ClientVersion",
            build_client_version(env!("CARGO_PKG_VERSION"))
                .to_string()
                .parse()
                .expect("header value"),
        );
        if let Some(route_tag) = self.route_tag.as_deref().filter(|v| !v.is_empty()) {
            match route_tag.parse() {
                Ok(value) => {
                    headers.insert("SKRouteTag", value);
                }
                Err(err) => {
                    tracing::warn!(error = %err, "invalid SKRouteTag header value, ignoring");
                }
            }
        }
        headers
    }
}

async fn read_json<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
    let status = resp.status();
    let bytes = resp.bytes().await.context("read response")?;
    if status != StatusCode::OK {
        anyhow::bail!(
            "HTTP {}: {}",
            status.as_u16(),
            String::from_utf8_lossy(&bytes)
        );
    }
    serde_json::from_slice(&bytes)
        .with_context(|| format!("unmarshal response: {}", response_summary(&bytes)))
}

fn response_summary(bytes: &[u8]) -> String {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(map)) => {
            let mut keys = map.keys().take(12).cloned().collect::<Vec<_>>();
            keys.sort();
            let mut parts = vec![format!("json object keys={}", keys.join(","))];
            if let Some(ret) = map.get("ret").or_else(|| map.get("Ret")) {
                parts.push(format!("ret={ret}"));
            }
            if let Some(errcode) = map
                .get("errcode")
                .or_else(|| map.get("errCode"))
                .or_else(|| map.get("ErrCode"))
            {
                parts.push(format!("errcode={errcode}"));
            }
            if let Some(errmsg) = map
                .get("errmsg")
                .or_else(|| map.get("errMsg"))
                .or_else(|| map.get("ErrMsg"))
                .and_then(|value| value.as_str())
            {
                parts.push(format!("errmsg={}", compact_snippet(errmsg, 160)));
            }
            parts.join("; ")
        }
        Ok(Value::Array(items)) => format!("json array len={}", items.len()),
        Ok(value) => format!("json {}", value_type(&value)),
        Err(_) => format!(
            "non-json body prefix={}",
            compact_snippet(&String::from_utf8_lossy(bytes), 160)
        ),
    }
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn compact_snippet(value: &str, max_chars: usize) -> String {
    let mut out = value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>();
    if out.chars().count() > max_chars {
        out = out.chars().take(max_chars).collect::<String>();
        out.push_str("...");
    }
    out
}

fn generate_wechat_uin() -> String {
    let n = rand::thread_rng().next_u32().to_string();
    base64::engine::general_purpose::STANDARD.encode(n.as_bytes())
}

fn build_client_version(version: &str) -> u32 {
    let mut parts = version
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    let major = parts.next().unwrap_or(0) & 0xff;
    let minor = parts.next().unwrap_or(0) & 0xff;
    let patch = parts.next().unwrap_or(0) & 0xff;
    (major << 16) | (minor << 8) | patch
}
