use serde::{Deserialize, Serialize};

pub const MESSAGE_TYPE_USER: i32 = 1;
pub const MESSAGE_TYPE_BOT: i32 = 2;
pub const MESSAGE_STATE_FINISH: i32 = 2;

pub const ITEM_TYPE_TEXT: i32 = 1;
pub const ITEM_TYPE_IMAGE: i32 = 2;
pub const ITEM_TYPE_VOICE: i32 = 3;
pub const ITEM_TYPE_FILE: i32 = 4;
pub const ITEM_TYPE_VIDEO: i32 = 5;

pub const CDN_MEDIA_TYPE_IMAGE: i32 = 1;
pub const CDN_MEDIA_TYPE_VIDEO: i32 = 2;
pub const CDN_MEDIA_TYPE_FILE: i32 = 3;

pub const TYPING_STATUS_TYPING: i32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QrCodeResponse {
    pub qrcode: String,
    pub qrcode_img_content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QrStatusResponse {
    pub status: String,
    #[serde(default)]
    pub bot_token: String,
    #[serde(default)]
    pub ilink_bot_id: String,
    #[serde(default)]
    pub baseurl: String,
    #[serde(default)]
    pub ilink_user_id: String,
    #[serde(default)]
    pub redirect_host: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub bot_token: String,
    pub ilink_bot_id: String,
    #[serde(default)]
    pub baseurl: String,
    #[serde(default)]
    pub ilink_user_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BaseInfo {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub channel_version: String,
}

impl BaseInfo {
    pub fn channel() -> Self {
        Self {
            channel_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GetUpdatesRequest {
    pub get_updates_buf: String,
    pub base_info: BaseInfo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GetUpdatesResponse {
    #[serde(default, alias = "Ret")]
    pub ret: i32,
    #[serde(default, alias = "ErrCode", alias = "errCode")]
    pub errcode: i32,
    #[serde(default, alias = "ErrMsg", alias = "errMsg")]
    pub errmsg: String,
    #[serde(default, alias = "Msgs", alias = "msgList")]
    pub msgs: Vec<WeixinMessage>,
    #[serde(default)]
    #[serde(alias = "getUpdatesBuf", alias = "GetUpdatesBuf")]
    pub get_updates_buf: String,
    #[serde(default)]
    #[serde(
        alias = "longPollingTimeoutMs",
        alias = "longpollingTimeoutMs",
        alias = "LongPollingTimeoutMs"
    )]
    #[allow(dead_code)]
    pub longpolling_timeout_ms: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeixinMessage {
    #[serde(default)]
    pub seq: i32,
    #[serde(default)]
    pub message_id: i64,
    #[serde(default)]
    pub from_user_id: String,
    #[serde(default)]
    pub to_user_id: String,
    #[serde(default)]
    pub message_type: i32,
    #[serde(default)]
    pub message_state: i32,
    #[serde(default)]
    pub item_list: Vec<MessageItem>,
    #[serde(default)]
    pub context_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageItem {
    #[serde(default)]
    #[serde(rename = "type")]
    pub kind: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_item: Option<TextItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_item: Option<ImageItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_item: Option<VoiceItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_item: Option<VideoItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_item: Option<FileItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextItem {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaInfo {
    pub encrypt_query_param: String,
    pub aes_key: String,
    pub encrypt_type: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub full_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceItem {
    #[serde(default)]
    pub media: Option<MediaInfo>,
    #[serde(default)]
    pub voice_size: i32,
    #[serde(default)]
    pub encode_type: i32,
    #[serde(default)]
    pub bits_per_sample: i32,
    #[serde(default)]
    pub sample_rate: i32,
    #[serde(default)]
    pub playtime: i32,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageItem {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub media: Option<MediaInfo>,
    #[serde(default)]
    pub thumb_media: Option<MediaInfo>,
    #[serde(default)]
    pub aeskey: String,
    #[serde(default)]
    pub mid_size: i32,
    #[serde(default)]
    pub thumb_size: i32,
    #[serde(default)]
    pub thumb_height: i32,
    #[serde(default)]
    pub thumb_width: i32,
    #[serde(default)]
    pub hd_size: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoItem {
    #[serde(default)]
    pub media: Option<MediaInfo>,
    #[serde(default)]
    pub video_size: i32,
    #[serde(default)]
    pub play_length: i32,
    #[serde(default)]
    pub video_md5: String,
    #[serde(default)]
    pub thumb_media: Option<MediaInfo>,
    #[serde(default)]
    pub thumb_size: i32,
    #[serde(default)]
    pub thumb_height: i32,
    #[serde(default)]
    pub thumb_width: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileItem {
    #[serde(default)]
    pub media: Option<MediaInfo>,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub md5: String,
    #[serde(default)]
    pub len: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GetUploadUrlRequest {
    pub filekey: String,
    pub media_type: i32,
    pub to_user_id: String,
    pub rawsize: usize,
    pub rawfilemd5: String,
    pub filesize: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb_rawsize: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb_rawfilemd5: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb_filesize: Option<usize>,
    pub no_need_thumb: bool,
    pub aeskey: String,
    pub base_info: BaseInfo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GetUploadUrlResponse {
    #[serde(default, alias = "Ret")]
    pub ret: i32,
    #[serde(default, alias = "ErrMsg", alias = "errMsg")]
    pub errmsg: String,
    #[serde(default)]
    pub upload_param: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub thumb_upload_param: String,
    #[serde(default)]
    pub upload_full_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SendMessageRequest {
    pub msg: SendMsg,
    pub base_info: BaseInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct SendMsg {
    pub from_user_id: String,
    pub to_user_id: String,
    pub client_id: String,
    pub message_type: i32,
    pub message_state: i32,
    pub item_list: Vec<MessageItem>,
    pub context_token: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendMessageResponse {
    #[serde(default, alias = "Ret")]
    pub ret: i32,
    #[serde(default, alias = "ErrMsg", alias = "errMsg")]
    pub errmsg: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GetConfigRequest {
    pub ilink_user_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub context_token: String,
    pub base_info: BaseInfo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GetConfigResponse {
    #[serde(default, alias = "Ret")]
    pub ret: i32,
    #[serde(default, alias = "ErrMsg", alias = "errMsg")]
    pub errmsg: String,
    #[serde(default)]
    pub typing_ticket: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SendTypingRequest {
    pub ilink_user_id: String,
    pub typing_ticket: String,
    pub status: i32,
    pub base_info: BaseInfo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendTypingResponse {
    #[serde(default, alias = "Ret")]
    pub ret: i32,
    #[serde(default, alias = "ErrMsg", alias = "errMsg")]
    pub errmsg: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn getupdates_accepts_capitalized_error_fields() {
        let resp: GetUpdatesResponse =
            serde_json::from_str(r#"{"Ret":0,"ErrCode":-14,"ErrMsg":"expired","Msgs":[]}"#)
                .unwrap();
        assert_eq!(resp.ret, 0);
        assert_eq!(resp.errcode, -14);
        assert_eq!(resp.errmsg, "expired");
        assert!(resp.msgs.is_empty());
    }
}
