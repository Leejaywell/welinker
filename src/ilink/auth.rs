use super::*;
use crate::config::app_dir;
use anyhow::{Context, Result};
use std::{fs, path::PathBuf, time::Duration};
use tokio::time::sleep;
use url::form_urlencoded::byte_serialize;

const QR_CODE_URL: &str = "https://ilinkai.weixin.qq.com/ilink/bot/get_bot_qrcode?bot_type=3";
const QR_STATUS_PATH: &str = "/ilink/bot/get_qrcode_status?qrcode=";
const FIXED_BASE_URL: &str = "https://ilinkai.weixin.qq.com";

pub async fn fetch_qrcode() -> Result<QrCodeResponse> {
    Client::unauthenticated()
        .get(QR_CODE_URL)
        .await
        .context("fetch QR code")
}

pub async fn poll_qr_status<F>(qrcode: &str, mut on_status: F) -> Result<Credentials>
where
    F: FnMut(&str),
{
    let client = Client::unauthenticated();
    let encoded_qrcode = byte_serialize(qrcode.as_bytes()).collect::<String>();
    let mut base_url = FIXED_BASE_URL.to_string();
    loop {
        let url = format!("{base_url}{QR_STATUS_PATH}{encoded_qrcode}");
        let resp: QrStatusResponse = match client.get(&url).await {
            Ok(resp) => resp,
            Err(err) => {
                tracing::debug!(error = %err, "QR status poll failed, retrying");
                sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        on_status(&resp.status);
        match resp.status.as_str() {
            "scaned_but_redirect" => {
                if !resp.redirect_host.is_empty() {
                    base_url = format!("https://{}", resp.redirect_host);
                    tracing::info!(redirect_host = %resp.redirect_host, "QR polling redirected");
                }
            }
            "confirmed" => {
                return Ok(Credentials {
                    bot_token: resp.bot_token,
                    ilink_bot_id: resp.ilink_bot_id,
                    baseurl: resp.baseurl,
                    ilink_user_id: resp.ilink_user_id,
                });
            }
            "expired" => anyhow::bail!("QR code expired"),
            _ => {}
        }
    }
}

pub fn accounts_dir() -> PathBuf {
    app_dir().join("accounts")
}

pub fn credentials_path() -> PathBuf {
    accounts_dir()
}

pub fn normalize_account_id(raw: &str) -> String {
    raw.replace(['@', '.', ':'], "-")
}

pub fn save_credentials(creds: &Credentials) -> Result<()> {
    let dir = accounts_dir();
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(format!(
        "{}.json",
        normalize_account_id(&creds.ilink_bot_id)
    ));
    fs::write(&path, serde_json::to_vec_pretty(creds)?)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn load_all_credentials() -> Result<Vec<Credentials>> {
    let dir = accounts_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(data) = fs::read(entry.path()) else {
            continue;
        };
        if let Ok(creds) = serde_json::from_slice::<Credentials>(&data) {
            if !creds.bot_token.is_empty() {
                out.push(creds);
            }
        }
    }
    Ok(out)
}
