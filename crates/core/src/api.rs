//! 聊天服务器 REST 客户端(GUI 后台线程与 CLI 共用)。
//!
//! 所有路径挂 `/api/v1`;鉴权 `Authorization: Bearer <jwt>`;
//! 401 统一映射为 `LtlError::Auth`(上层据此回登录页)。

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::error::{LtlError, Result};
use crate::types::{AttachmentMeta, ConvType, Conversation, Message, User, UserId};

#[derive(Clone)]
pub struct ChatApiClient {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
}

// ---------------------------------------------------------------------------
// REST DTO
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub user: User,
    /// 服务器 iroh 节点 id(客户端 gossip bootstrap)
    #[serde(default)]
    pub server_endpoint_id: String,
}

#[derive(Serialize)]
struct LoginByTokenBody {
    ym_auth_token: String,
}

#[derive(Serialize)]
struct LoginByPhoneBody {
    phone: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
}

#[derive(Serialize)]
struct RefreshBody {
    access_token: String,
}

#[derive(Serialize)]
pub struct CreateConversationBody {
    pub conv_type: ConvType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub member_ids: Vec<UserId>,
}

/// 服务器历史消息(带服务器序号, 用于增量补齐)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HistoryMessage {
    pub message: Message,
    pub server_seq: i64,
}

#[derive(Serialize)]
struct AddMembersBody {
    member_ids: Vec<UserId>,
}

#[derive(Deserialize)]
struct OnlineUser {
    user_id: UserId,
}

// ---------------------------------------------------------------------------
// 实现
// ---------------------------------------------------------------------------

impl ChatApiClient {
    pub fn new(base: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: base.trim_end_matches('/').to_string(),
            token: None,
        }
    }

    pub fn with_token(base: &str, token: &str) -> Self {
        let mut c = Self::new(base);
        c.token = Some(token.to_string());
        c
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    // -- 登录 ----------------------------------------------------------------

    pub async fn login_by_token(&self, ym_auth_token: &str) -> Result<TokenResponse> {
        self.post("/api/v1/auth/login", &LoginByTokenBody { ym_auth_token: ym_auth_token.into() })
            .await
    }

    pub async fn login_by_phone(&self, phone: &str, username: Option<&str>) -> Result<TokenResponse> {
        self.post(
            "/api/v1/auth/phone/login",
            &LoginByPhoneBody { phone: phone.into(), username: username.map(|s| s.to_string()) },
        )
        .await
    }

    /// 旧 token 续签(签发 30 天内有效)
    pub async fn refresh(&self, old_token: &str) -> Result<TokenResponse> {
        self.post("/api/v1/auth/refresh", &RefreshBody { access_token: old_token.into() })
            .await
    }

    pub async fn me(&self) -> Result<User> {
        self.get("/api/v1/auth/me").await
    }

    // -- 名单 ----------------------------------------------------------------

    pub async fn users(&self) -> Result<Vec<User>> {
        self.get("/api/v1/users").await
    }

    pub async fn online_user_ids(&self) -> Result<Vec<UserId>> {
        let list: Vec<OnlineUser> = self.get("/api/v1/users/online").await?;
        Ok(list.into_iter().map(|u| u.user_id).collect())
    }

    pub async fn heartbeat(&self) -> Result<()> {
        let _: serde_json::Value = self.post("/api/v1/heartbeat", &serde_json::json!({})).await?;
        Ok(())
    }

    // -- 会话 ----------------------------------------------------------------

    pub async fn conversations(&self) -> Result<Vec<Conversation>> {
        self.get("/api/v1/conversations").await
    }

    pub async fn create_conversation(
        &self,
        conv_type: ConvType,
        title: Option<&str>,
        member_ids: Vec<UserId>,
    ) -> Result<Conversation> {
        self.post(
            "/api/v1/conversations",
            &CreateConversationBody {
                conv_type,
                title: title.map(|s| s.to_string()),
                member_ids,
            },
        )
        .await
    }

    pub async fn add_members(&self, conv_id: i64, member_ids: Vec<UserId>) -> Result<()> {
        let _: serde_json::Value = self
            .post(
                &format!("/api/v1/conversations/{conv_id}/members"),
                &AddMembersBody { member_ids },
            )
            .await?;
        Ok(())
    }

    /// 增量历史:`after_seq` 为本地已知的最大服务器序号(取更新的)
    pub async fn history(&self, conv_id: i64, after_seq: i64, limit: usize) -> Result<Vec<HistoryMessage>> {
        self.get(&format!(
            "/api/v1/conversations/{conv_id}/messages?after_seq={after_seq}&limit={limit}"
        ))
        .await
    }

    /// 更早的历史:`before_seq` 为本地最小服务器序号(取更旧的)
    pub async fn history_before(&self, conv_id: i64, before_seq: i64, limit: usize) -> Result<Vec<HistoryMessage>> {
        self.get(&format!(
            "/api/v1/conversations/{conv_id}/messages?before_seq={before_seq}&limit={limit}"
        ))
        .await
    }

    // -- 附件 ----------------------------------------------------------------

    pub async fn upload_attachment(&self, bytes: Vec<u8>, file_name: &str, mime: &str) -> Result<AttachmentMeta> {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(file_name.to_string())
            .mime_str(mime)
            .map_err(|e| LtlError::Other(e.to_string()))?;
        let form = reqwest::multipart::Form::new().part("file", part);
        let url = format!("{}{}", self.base, "/api/v1/files/upload");
        let mut req = self.http.post(&url).multipart(form);
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await?;
        self.parse(resp).await
    }

    /// 相对 url 拼成绝对地址
    pub fn absolute_url(&self, url: &str) -> String {
        if url.starts_with("http://") || url.starts_with("https://") {
            url.to_string()
        } else {
            format!("{}{}", self.base, url)
        }
    }

    pub async fn download(&self, url: &str) -> Result<Vec<u8>> {
        let mut req = self.http.get(url);
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(LtlError::Other(format!("下载失败: HTTP {}", resp.status())));
        }
        Ok(resp.bytes().await?.to_vec())
    }

    // -- 底层 ----------------------------------------------------------------

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base, path);
        let mut req = self.http.get(&url);
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await?;
        self.parse(resp).await
    }

    async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let url = format!("{}{}", self.base, path);
        let mut req = self.http.post(&url).json(body);
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await?;
        self.parse(resp).await
    }

    async fn parse<T: DeserializeOwned>(&self, resp: reqwest::Response) -> Result<T> {
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(LtlError::Auth("登录已过期".into()));
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(LtlError::Other(format!("HTTP {status}: {}", truncate(&text, 300))));
        }
        Ok(resp.json().await?)
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}
