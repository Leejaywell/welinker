use crate::ilink::*;
use aes::cipher::{generic_array::GenericArray, BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes128;
use anyhow::{Context, Result};
use base64::Engine;
use md5::{Digest, Md5};
use rand::RngCore;
use reqwest::Client as HttpClient;
use std::time::Duration;
use url::form_urlencoded::byte_serialize;

const CDN_BASE_URL: &str = "https://novac2c.cdn.weixin.qq.com/c2c";

#[derive(Debug, Clone)]
pub struct UploadedFile {
    pub download_param: String,
    pub aes_key_hex: String,
    pub file_size: usize,
    pub cipher_size: usize,
}

pub async fn upload_file_to_cdn(
    client: &Client,
    data: &[u8],
    to_user_id: &str,
    media_type: i32,
) -> Result<UploadedFile> {
    let mut filekey = [0_u8; 16];
    let mut aeskey = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut filekey);
    rand::thread_rng().fill_bytes(&mut aeskey);
    let filekey_hex = hex::encode(filekey);
    let aeskey_hex = hex::encode(aeskey);

    let mut hasher = Md5::new();
    hasher.update(data);
    let raw_md5 = hex::encode(hasher.finalize());
    let cipher_size = aes_ecb_padded_size(data.len());

    let upload = client
        .get_upload_url(&GetUploadUrlRequest {
            filekey: filekey_hex.clone(),
            media_type,
            to_user_id: to_user_id.to_string(),
            rawsize: data.len(),
            rawfilemd5: raw_md5,
            filesize: cipher_size,
            thumb_rawsize: None,
            thumb_rawfilemd5: None,
            thumb_filesize: None,
            no_need_thumb: true,
            aeskey: aeskey_hex.clone(),
            base_info: BaseInfo::channel(),
        })
        .await?;
    if upload.ret != 0 {
        anyhow::bail!(
            "get upload URL failed: ret={} errmsg={}",
            upload.ret,
            upload.errmsg
        );
    }

    let encrypted = encrypt_aes_ecb(data, &aeskey)?;
    let cdn_url = if upload.upload_full_url.trim().is_empty() {
        if upload.upload_param.is_empty() {
            anyhow::bail!("getuploadurl returned no upload URL");
        }
        format!(
            "{CDN_BASE_URL}/upload?encrypted_query_param={}&filekey={}",
            byte_serialize(upload.upload_param.as_bytes()).collect::<String>(),
            byte_serialize(filekey_hex.as_bytes()).collect::<String>()
        )
    } else {
        upload.upload_full_url
    };
    let download_param = upload_to_cdn(&cdn_url, encrypted).await?;
    Ok(UploadedFile {
        download_param,
        aes_key_hex: aeskey_hex,
        file_size: data.len(),
        cipher_size,
    })
}

pub fn aes_key_to_base64(hex_key: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(hex_key.as_bytes())
}

#[allow(dead_code)]
pub async fn download_file_from_cdn(
    encrypt_query_param: &str,
    aes_key_base64: &str,
) -> Result<Vec<u8>> {
    download_file_from_cdn_with_url(encrypt_query_param, aes_key_base64, None).await
}

#[allow(dead_code)]
pub async fn download_file_from_cdn_with_url(
    encrypt_query_param: &str,
    aes_key_base64: &str,
    full_url: Option<&str>,
) -> Result<Vec<u8>> {
    let aes_key = parse_aes_key(aes_key_base64)?;
    let url = full_url
        .filter(|url| !url.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            format!("{CDN_BASE_URL}/download?encrypted_query_param=")
                + &byte_serialize(encrypt_query_param.as_bytes()).collect::<String>()
        });
    let resp = HttpClient::builder()
        .timeout(Duration::from_secs(60))
        .build()?
        .get(url)
        .send()
        .await?;
    let status = resp.status();
    let bytes = resp.bytes().await?;
    if !status.is_success() {
        anyhow::bail!(
            "CDN download HTTP {}: {}",
            status.as_u16(),
            String::from_utf8_lossy(&bytes)
        );
    }
    decrypt_aes_ecb(&bytes, &aes_key)
}

fn parse_aes_key(aes_key_base64: &str) -> Result<Vec<u8>> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(aes_key_base64)
        .context("decode AES key base64")?;
    if decoded.len() == 16 {
        return Ok(decoded);
    }
    if decoded.len() == 32 && decoded.iter().all(|b| b.is_ascii_hexdigit()) {
        return hex::decode(String::from_utf8_lossy(&decoded).as_ref())
            .context("decode AES key hex");
    }
    anyhow::bail!(
        "aes_key must decode to 16 raw bytes or 32-char hex string, got {} bytes",
        decoded.len()
    )
}

async fn upload_to_cdn(cdn_url: &str, encrypted: Vec<u8>) -> Result<String> {
    let resp = HttpClient::builder()
        .timeout(Duration::from_secs(60))
        .build()?
        .post(cdn_url)
        .header("Content-Type", "application/octet-stream")
        .body(encrypted)
        .send()
        .await?;
    let status = resp.status();
    let header = resp
        .headers()
        .get("X-Encrypted-Param")
        .and_then(|v| v.to_str().ok())
        .map(ToString::to_string);
    let body = resp.bytes().await?;
    if !status.is_success() {
        anyhow::bail!(
            "CDN upload HTTP {}: {}",
            status.as_u16(),
            String::from_utf8_lossy(&body)
        );
    }
    header.context("CDN upload missing X-Encrypted-Param header")
}

fn encrypt_aes_ecb(plaintext: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes128::new_from_slice(key)?;
    let pad_len = 16 - (plaintext.len() % 16);
    let mut padded = Vec::with_capacity(plaintext.len() + pad_len);
    padded.extend_from_slice(plaintext);
    padded.extend(std::iter::repeat_n(pad_len as u8, pad_len));
    for chunk in padded.chunks_mut(16) {
        cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
    }
    Ok(padded)
}

#[allow(dead_code)]
fn decrypt_aes_ecb(ciphertext: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    if !ciphertext.len().is_multiple_of(16) {
        anyhow::bail!("ciphertext is not a multiple of block size");
    }
    let cipher = Aes128::new_from_slice(key)?;
    let mut out = ciphertext.to_vec();
    for chunk in out.chunks_mut(16) {
        cipher.decrypt_block(GenericArray::from_mut_slice(chunk));
    }
    let pad_len = *out.last().unwrap_or(&0) as usize;
    if pad_len == 0 || pad_len > 16 || pad_len > out.len() {
        anyhow::bail!("invalid PKCS7 padding");
    }
    out.truncate(out.len() - pad_len);
    Ok(out)
}

fn aes_ecb_padded_size(size: usize) -> usize {
    (size / 16 + 1) * 16
}
