use super::*;
use anyhow::Result;
use std::{fs, path::PathBuf, sync::Arc, time::Duration};
use tokio::{sync::oneshot, time::sleep};

const MAX_CONSECUTIVE_FAILURES: usize = 5;
const ERR_CODE_SESSION_EXPIRED: i32 = -14;
const DEFAULT_LONG_POLL_TIMEOUT: Duration = Duration::from_secs(35);
const LONG_POLL_GRACE: Duration = Duration::from_secs(5);

pub type MessageHandler =
    Arc<dyn Fn(Client, WeixinMessage) -> futures::future::BoxFuture<'static, ()> + Send + Sync>;

pub struct Monitor {
    client: Client,
    handler: MessageHandler,
    get_updates_buf: String,
    buf_path: PathBuf,
    failures: usize,
    next_timeout: Duration,
}

impl Monitor {
    pub fn new(client: Client, handler: MessageHandler) -> Result<Self> {
        let account_id = normalize_account_id(client.bot_id());
        let buf_path = accounts_dir().join(format!("{account_id}.sync.json"));
        let get_updates_buf = load_buf(&buf_path);
        Ok(Self {
            client,
            handler,
            get_updates_buf,
            buf_path,
            failures: 0,
            next_timeout: DEFAULT_LONG_POLL_TIMEOUT,
        })
    }

    pub async fn run(mut self, mut shutdown: oneshot::Receiver<()>) -> Result<()> {
        loop {
            tokio::select! {
                _ = &mut shutdown => return Ok(()),
                result = self.client.get_updates(&self.get_updates_buf, self.next_timeout + LONG_POLL_GRACE) => {
                    match result {
                        Ok(resp) => self.handle_response(resp).await,
                        Err(err) => {
                            self.failures += 1;
                            let backoff = self.backoff();
                            tracing::warn!(failures = self.failures, error = %err, ?backoff, "getupdates failed");
                            if self.failures == MAX_CONSECUTIVE_FAILURES {
                                tracing::warn!("multiple failures; run `welinker login` if authentication expired");
                            }
                            sleep(backoff).await;
                        }
                    }
                }
            }
        }
    }

    async fn handle_response(&mut self, resp: GetUpdatesResponse) {
        self.failures = 0;
        if resp.longpolling_timeout_ms > 0 {
            self.next_timeout = Duration::from_millis(resp.longpolling_timeout_ms as u64);
        }
        if resp.errcode == ERR_CODE_SESSION_EXPIRED {
            if !self.get_updates_buf.is_empty() {
                self.get_updates_buf.clear();
                save_buf(&self.buf_path, "");
            } else {
                tracing::warn!("WeChat session expired; run `welinker login`");
            }
            sleep(Duration::from_secs(5)).await;
            return;
        }
        if resp.ret != 0 && resp.errcode != 0 {
            tracing::warn!(ret = resp.ret, errcode = resp.errcode, errmsg = %resp.errmsg, "server error");
            return;
        }
        if !resp.get_updates_buf.is_empty() {
            self.get_updates_buf = resp.get_updates_buf;
            save_buf(&self.buf_path, &self.get_updates_buf);
        }
        for msg in resp.msgs {
            let handler = Arc::clone(&self.handler);
            let client = self.client.clone();
            tokio::spawn(async move {
                handler(client, msg).await;
            });
        }
    }

    fn backoff(&self) -> Duration {
        let secs =
            (3_u64).saturating_mul(2_u64.saturating_pow(self.failures.saturating_sub(1) as u32));
        Duration::from_secs(secs.min(60))
    }
}

fn load_buf(path: &PathBuf) -> String {
    let Ok(data) = fs::read(path) else {
        return String::new();
    };
    serde_json::from_slice::<serde_json::Value>(&data)
        .ok()
        .and_then(|v| v["get_updates_buf"].as_str().map(ToString::to_string))
        .unwrap_or_default()
}

fn save_buf(path: &PathBuf, buf: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(
        path,
        serde_json::json!({"get_updates_buf": buf}).to_string(),
    );
}
