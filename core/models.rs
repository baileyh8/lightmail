use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MailError {
    #[error("{message}")]
    Failure { message: String },
}
pub type Result<T> = std::result::Result<T, MailError>;
pub fn fail(message: impl ToString) -> MailError {
    MailError::Failure {
        message: message.to_string(),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, uniffi::Record)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub address: String,
    pub provider: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub auth_kind: String,
    pub color: String,
    pub enabled: bool,
    pub sent_mode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, uniffi::Record)]
pub struct Folder {
    pub id: String,
    pub account_id: String,
    pub path: String,
    pub name: String,
    pub role: String,
    pub unread_count: u32,
    pub total_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, uniffi::Record)]
pub struct MessageSummary {
    pub id: String,
    pub account_id: String,
    pub folder_id: String,
    pub uid: u32,
    pub uid_validity: u32,
    pub canonical_id: String,
    pub thread_id: String,
    pub message_id: String,
    pub subject: String,
    pub from_name: String,
    pub from_address: String,
    #[serde(default)]
    pub reply_to_address: String,
    pub to_addresses: String,
    pub cc_addresses: String,
    pub timestamp: i64,
    pub snippet: String,
    pub unread: bool,
    pub starred: bool,
    pub size: u64,
    pub has_attachments: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, uniffi::Record)]
pub struct MessageQuery {
    pub account_id: String,
    pub folder_id: String,
    pub scope: String,
    pub search: String,
    pub unread_only: bool,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, uniffi::Record)]
pub struct AttachmentInfo {
    pub part_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, uniffi::Record)]
pub struct MailBody {
    pub message_id: String,
    pub text: String,
    pub markdown: String,
    pub html: String,
    pub attachments: Vec<AttachmentInfo>,
    pub content_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, uniffi::Record)]
pub struct SyncResult {
    pub added: u32,
    pub folders: u32,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, uniffi::Record)]
pub struct Draft {
    pub id: String,
    pub account_id: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    pub attachment_paths: Vec<String>,
    pub reply_to_message_id: String,
    pub references: String,
    pub status: String,
    pub last_error: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub send_after: i64,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct StorageInfo {
    pub message_count: u64,
    pub body_count: u64,
    pub cache_bytes: u64,
    pub database_bytes: u64,
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn folder_id(account: &str, path: &str) -> String {
    format!("{}:{}", account, path)
}
