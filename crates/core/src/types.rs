//! 领域类型与 gossip wire 协议。
//!
//! 命名对齐 ltl-manage 旧语义(修正旧系统 msg_type 全落 text 的 bug):
//! - `MsgType::{Text,Image,File,System}` 统一走 `msg_type`
//! - 单聊 `ConvType::Single` 幂等复用, 群聊 `ConvType::Group`
//! - 附件元数据随消息走 gossip, 字节本体走服务器 HTTP

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::LtlError;

pub type UserId = i64;
pub type MessageId = Uuid;

// ---------------------------------------------------------------------------
// 基础枚举
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConvType {
    /// 单聊(幂等: 两人只存在一个会话)
    Single,
    /// 群聊
    Group,
}

impl ConvType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConvType::Single => "single",
            ConvType::Group => "group",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "single" | "direct" | "private" => ConvType::Single,
            _ => ConvType::Group,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MsgType {
    Text,
    Image,
    File,
    System,
}

impl MsgType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MsgType::Text => "text",
            MsgType::Image => "image",
            MsgType::File => "file",
            MsgType::System => "system",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "image" => MsgType::Image,
            "file" => MsgType::File,
            "system" => MsgType::System,
            _ => MsgType::Text,
        }
    }
}

// ---------------------------------------------------------------------------
// 实体
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct User {
    pub id: UserId,
    pub ymid: String,
    pub name: String,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub is_online: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemberInfo {
    pub user_id: UserId,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Conversation {
    pub id: i64,
    #[serde(default)]
    pub title: String,
    pub conv_type: ConvType,
    /// 32 字节 TopicId 的 hex 表示(64 字符), JSON/DB 都用它
    pub topic_id: String,
    pub created_by: UserId,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub members: Vec<MemberInfo>,
}

impl Conversation {
    /// 单聊会话的展示名 = 对方成员名(群聊 = 群名)
    pub fn display_title(&self, my_id: UserId) -> String {
        if self.conv_type == ConvType::Group {
            return if self.title.is_empty() {
                "未命名群聊".to_string()
            } else {
                self.title.clone()
            };
        }
        self.members
            .iter()
            .find(|m| m.user_id != my_id)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "单聊".to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttachmentMeta {
    pub id: i64,
    pub file_name: String,
    pub file_size: i64,
    /// MIME 类型
    pub file_type: String,
    /// 服务器相对路径(如 /api/v1/files/12), 客户端拼 base 成绝对地址
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    /// 全局唯一, 发送方生成; 去重与撤回都以此为准
    pub id: MessageId,
    pub conversation_id: i64,
    pub sender_id: UserId,
    #[serde(default)]
    pub sender_name: String,
    pub msg_type: MsgType,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub reply_to: Option<MessageId>,
    #[serde(default)]
    pub attachments: Vec<AttachmentMeta>,
    #[serde(default)]
    pub is_recalled: bool,
    /// 毫秒时间戳(发送方时钟)
    pub created_at_ms: i64,
}

impl Message {
    pub fn new_text(conversation_id: i64, sender_id: UserId, sender_name: &str, content: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            conversation_id,
            sender_id,
            sender_name: sender_name.to_string(),
            msg_type: MsgType::Text,
            content,
            reply_to: None,
            attachments: Vec::new(),
            is_recalled: false,
            created_at_ms: chrono::Utc::now().timestamp_millis(),
        }
    }

    pub fn new_attachment(
        conversation_id: i64,
        sender_id: UserId,
        sender_name: &str,
        msg_type: MsgType,
        content: String,
        attachments: Vec<AttachmentMeta>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            conversation_id,
            sender_id,
            sender_name: sender_name.to_string(),
            msg_type,
            content,
            reply_to: None,
            attachments,
            is_recalled: false,
            created_at_ms: chrono::Utc::now().timestamp_millis(),
        }
    }
}

/// 本地存储里的消息(带客户端行号, 用于翻页/已读游标)
#[derive(Debug, Clone)]
pub struct LocalMessage {
    pub rowid: i64,
    /// 服务器历史序号, gossip 先到的消息为 None, 历史补齐时回填
    pub server_seq: Option<i64>,
    pub message: Message,
}

// ---------------------------------------------------------------------------
// gossip wire 协议
// ---------------------------------------------------------------------------

/// gossip 主题上的载荷。iroh 按内容哈希去重(90 秒窗口), 所有需要重发的载荷
/// 都经 `GossipEnvelope` 包一层随机 nonce。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipPayload {
    /// 新消息
    Message(Message),
    /// 撤回(仅发送者本人发起)
    Recall {
        conversation_id: i64,
        message_id: MessageId,
        sender_id: UserId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GossipEnvelope {
    pub payload: GossipPayload,
    pub nonce: [u8; 16],
}

impl GossipEnvelope {
    pub fn new(payload: GossipPayload) -> Self {
        Self { payload, nonce: rand::random() }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("serde_json::to_vec infallible")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

// ---------------------------------------------------------------------------
// TopicId 编解码
// ---------------------------------------------------------------------------

pub fn topic_id_to_hex(topic: &iroh_gossip::proto::TopicId) -> String {
    hex::encode(topic.as_bytes())
}

pub fn topic_id_from_hex(s: &str) -> Result<iroh_gossip::proto::TopicId, LtlError> {
    let bytes = hex::decode(s.trim())
        .map_err(|e| LtlError::Other(format!("topic_id 非法 hex: {e}")))?;
    if bytes.len() != 32 {
        return Err(LtlError::Other(format!(
            "topic_id 长度应为 32 字节, 实际 {len}",
            len = bytes.len()
        )));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(iroh_gossip::proto::TopicId::from_bytes(arr))
}
