use super::{
    aes_key_to_base64, download_file_from_cdn_with_url, markdown_to_plain_text, upload_file_to_cdn,
};
use crate::ilink::*;
use anyhow::{Context, Result};
use base64::Engine;
use regex::Regex;
use reqwest::Client as HttpClient;
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::fs;
use uuid::Uuid;

pub fn extract_image_urls(text: &str) -> Vec<String> {
    Regex::new(r"!\[[^\]]*\]\(([^)]+)\)")
        .unwrap()
        .captures_iter(text)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str().trim().to_string()))
        .filter(|url| url.starts_with("http://") || url.starts_with("https://"))
        .collect()
}

pub async fn send_typing_state(client: &Client, user_id: &str, context_token: &str) -> Result<()> {
    let cfg = client.get_config(user_id, context_token).await?;
    if cfg.ret != 0 {
        anyhow::bail!("getconfig failed: ret={} errmsg={}", cfg.ret, cfg.errmsg);
    }
    if cfg.typing_ticket.is_empty() {
        anyhow::bail!("no typing_ticket returned from getconfig");
    }
    client
        .send_typing(user_id, &cfg.typing_ticket, TYPING_STATUS_TYPING)
        .await
}

pub async fn send_text_reply(
    client: &Client,
    to_user_id: &str,
    text: &str,
    context_token: &str,
    client_id: Option<String>,
) -> Result<()> {
    let req = SendMessageRequest {
        msg: SendMsg {
            from_user_id: client.bot_id().to_string(),
            to_user_id: to_user_id.to_string(),
            client_id: client_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            message_type: MESSAGE_TYPE_BOT,
            message_state: MESSAGE_STATE_FINISH,
            item_list: vec![MessageItem {
                kind: ITEM_TYPE_TEXT,
                text_item: Some(TextItem {
                    text: markdown_to_plain_text(text),
                }),
                image_item: None,
                voice_item: None,
                video_item: None,
                file_item: None,
            }],
            context_token: context_token.to_string(),
        },
        base_info: BaseInfo::channel(),
    };
    let resp = client.send_message(&req).await?;
    if resp.ret != 0 {
        anyhow::bail!(
            "send message failed: ret={} errmsg={}",
            resp.ret,
            resp.errmsg
        );
    }
    Ok(())
}

pub async fn send_media_from_url(
    client: &Client,
    to_user_id: &str,
    media_url: &str,
    context_token: &str,
) -> Result<()> {
    let (data, content_type) = download_file(media_url).await?;
    send_media_data(
        client,
        to_user_id,
        filename_from_url(media_url),
        media_url,
        &data,
        &content_type,
        context_token,
    )
    .await
}

#[allow(dead_code)]
pub async fn send_media_from_path(
    client: &Client,
    to_user_id: &str,
    path: &Path,
    context_token: &str,
) -> Result<()> {
    let data = tokio::fs::read(path)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();
    let content_type = infer_content_type(path.to_string_lossy().as_ref());
    send_media_data(
        client,
        to_user_id,
        name,
        path.to_string_lossy().as_ref(),
        &data,
        &content_type,
        context_token,
    )
    .await
}

pub async fn save_inbound_media_item(
    item: &MessageItem,
    save_dir: &Path,
) -> Result<Option<PathBuf>> {
    fs::create_dir_all(save_dir)
        .await
        .with_context(|| format!("create {}", save_dir.display()))?;
    match item.kind {
        ITEM_TYPE_IMAGE => {
            let Some(image) = item.image_item.as_ref() else {
                return Ok(None);
            };
            let Some(media) = image.media.as_ref() else {
                return Ok(None);
            };
            let data = if !image.aeskey.is_empty() {
                let aes_key_base64 = base64::engine::general_purpose::STANDARD
                    .encode(hex::decode(&image.aeskey).context("decode image aeskey hex")?);
                download_file_from_cdn_with_url(
                    &media.encrypt_query_param,
                    &aes_key_base64,
                    Some(&media.full_url),
                )
                .await?
            } else if !media.aes_key.is_empty() {
                download_file_from_cdn_with_url(
                    &media.encrypt_query_param,
                    &media.aes_key,
                    Some(&media.full_url),
                )
                .await?
            } else if !image.url.is_empty() {
                download_file(&image.url).await?.0
            } else if !media.full_url.is_empty() {
                download_file(&media.full_url).await?.0
            } else {
                return Ok(None);
            };
            let ext = detect_image_ext(&data).unwrap_or(".jpg");
            write_saved_media(save_dir, "image", ext, &data)
                .await
                .map(Some)
        }
        ITEM_TYPE_VOICE => {
            let Some(voice) = item.voice_item.as_ref() else {
                return Ok(None);
            };
            let Some(media) = voice.media.as_ref() else {
                return Ok(None);
            };
            if media.aes_key.is_empty() {
                return Ok(None);
            }
            let data = download_file_from_cdn_with_url(
                &media.encrypt_query_param,
                &media.aes_key,
                Some(&media.full_url),
            )
            .await?;
            write_saved_media(save_dir, "voice", ".silk", &data)
                .await
                .map(Some)
        }
        ITEM_TYPE_FILE => {
            let Some(file) = item.file_item.as_ref() else {
                return Ok(None);
            };
            let Some(media) = file.media.as_ref() else {
                return Ok(None);
            };
            if media.aes_key.is_empty() {
                return Ok(None);
            }
            let data = download_file_from_cdn_with_url(
                &media.encrypt_query_param,
                &media.aes_key,
                Some(&media.full_url),
            )
            .await?;
            let name = safe_file_name(if file.file_name.is_empty() {
                "file.bin"
            } else {
                &file.file_name
            });
            let path = save_dir.join(format!("{}-{}", timestamp_ms(), name));
            fs::write(&path, data)
                .await
                .with_context(|| format!("write {}", path.display()))?;
            Ok(Some(path))
        }
        ITEM_TYPE_VIDEO => {
            let Some(video) = item.video_item.as_ref() else {
                return Ok(None);
            };
            let Some(media) = video.media.as_ref() else {
                return Ok(None);
            };
            if media.aes_key.is_empty() {
                return Ok(None);
            }
            let data = download_file_from_cdn_with_url(
                &media.encrypt_query_param,
                &media.aes_key,
                Some(&media.full_url),
            )
            .await?;
            write_saved_media(save_dir, "video", ".mp4", &data)
                .await
                .map(Some)
        }
        _ => Ok(None),
    }
}

async fn send_media_data(
    client: &Client,
    to_user_id: &str,
    file_name: String,
    source: &str,
    data: &[u8],
    content_type: &str,
    context_token: &str,
) -> Result<()> {
    let (cdn_media_type, item_type) = classify_media(content_type, source);
    let uploaded = upload_file_to_cdn(client, data, to_user_id, cdn_media_type).await?;
    let media = MediaInfo {
        encrypt_query_param: uploaded.download_param,
        aes_key: aes_key_to_base64(&uploaded.aes_key_hex),
        encrypt_type: 1,
        full_url: String::new(),
    };
    let item = match item_type {
        ITEM_TYPE_IMAGE => MessageItem {
            kind: ITEM_TYPE_IMAGE,
            image_item: Some(ImageItem {
                url: String::new(),
                media: Some(media),
                thumb_media: None,
                aeskey: String::new(),
                mid_size: uploaded.cipher_size as i32,
                thumb_size: 0,
                thumb_height: 0,
                thumb_width: 0,
                hd_size: 0,
            }),
            text_item: None,
            voice_item: None,
            video_item: None,
            file_item: None,
        },
        ITEM_TYPE_VIDEO => MessageItem {
            kind: ITEM_TYPE_VIDEO,
            video_item: Some(VideoItem {
                media: Some(media),
                video_size: uploaded.cipher_size as i32,
                play_length: 0,
                video_md5: String::new(),
                thumb_media: None,
                thumb_size: 0,
                thumb_height: 0,
                thumb_width: 0,
            }),
            text_item: None,
            image_item: None,
            voice_item: None,
            file_item: None,
        },
        _ => MessageItem {
            kind: ITEM_TYPE_FILE,
            file_item: Some(FileItem {
                media: Some(media),
                file_name,
                md5: String::new(),
                len: uploaded.file_size.to_string(),
            }),
            text_item: None,
            image_item: None,
            voice_item: None,
            video_item: None,
        },
    };
    let req = SendMessageRequest {
        msg: SendMsg {
            from_user_id: client.bot_id().to_string(),
            to_user_id: to_user_id.to_string(),
            client_id: Uuid::new_v4().to_string(),
            message_type: MESSAGE_TYPE_BOT,
            message_state: MESSAGE_STATE_FINISH,
            item_list: vec![item],
            context_token: context_token.to_string(),
        },
        base_info: BaseInfo::channel(),
    };
    let resp = client.send_message(&req).await?;
    if resp.ret != 0 {
        anyhow::bail!("send media failed: ret={} errmsg={}", resp.ret, resp.errmsg);
    }
    Ok(())
}

async fn download_file(url: &str) -> Result<(Vec<u8>, String)> {
    let resp = HttpClient::builder()
        .timeout(Duration::from_secs(60))
        .build()?
        .get(url)
        .send()
        .await?;
    let status = resp.status();
    let content_type = resp
        .headers()
        .get("Content-Type")
        .and_then(|v| v.to_str().ok())
        .map(ToString::to_string)
        .unwrap_or_else(|| infer_content_type(url));
    let bytes = resp.bytes().await?;
    if !status.is_success() {
        anyhow::bail!("HTTP {}", status.as_u16());
    }
    Ok((bytes.to_vec(), content_type))
}

fn classify_media(content_type: &str, url: &str) -> (i32, i32) {
    let ct = content_type.to_ascii_lowercase();
    if ct.starts_with("image/") || has_ext(url, &["png", "jpg", "jpeg", "gif", "webp", "bmp"]) {
        return (CDN_MEDIA_TYPE_IMAGE, ITEM_TYPE_IMAGE);
    }
    if ct.starts_with("video/") || has_ext(url, &["mp4", "mov", "webm", "mkv", "avi"]) {
        return (CDN_MEDIA_TYPE_VIDEO, ITEM_TYPE_VIDEO);
    }
    (CDN_MEDIA_TYPE_FILE, ITEM_TYPE_FILE)
}

fn infer_content_type(path: &str) -> String {
    mime_guess::from_path(strip_query(path))
        .first_or_octet_stream()
        .to_string()
}

fn filename_from_url(url: &str) -> String {
    Path::new(strip_query(url))
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("file")
        .to_string()
}

fn has_ext(path: &str, exts: &[&str]) -> bool {
    let ext = Path::new(strip_query(path))
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    exts.contains(&ext.as_str())
}

fn strip_query(raw: &str) -> &str {
    raw.split_once('?').map(|(left, _)| left).unwrap_or(raw)
}

async fn write_saved_media(
    save_dir: &Path,
    prefix: &str,
    ext: &str,
    data: &[u8],
) -> Result<PathBuf> {
    let path = save_dir.join(format!("{}-{}{}", timestamp_ms(), prefix, ext));
    fs::write(&path, data)
        .await
        .with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

fn timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn detect_image_ext(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(".png")
    } else if data.starts_with(b"\xff\xd8\xff") {
        Some(".jpg")
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some(".gif")
    } else if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some(".webp")
    } else {
        None
    }
}

fn safe_file_name(name: &str) -> String {
    let cleaned = name
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => ch,
        })
        .collect::<String>();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "file.bin".to_string()
    } else {
        trimmed.chars().take(200).collect()
    }
}
