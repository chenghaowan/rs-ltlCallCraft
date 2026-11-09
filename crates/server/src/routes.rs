//! axum 路由:登记登录(YM Auth)、名单/心跳、会话、历史补齐、附件。
//! 语义对齐 ltl-manage `modules/{auth,user,conversation,message,file}` 的聊天相关子集。

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Multipart, OriginalUri, Path as AxPath, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use ltl_chat_core::api::TokenResponse;
use ltl_chat_core::iroh_gossip::proto::TopicId;
use ltl_chat_core::types::{
    AttachmentMeta, ConvType, Conversation, GossipEnvelope, GossipPayload, Message, MessageId, User,
    UserId,
};

use crate::config::AuthMode;
use crate::db::UserRow;
use crate::state::{AppState, OAuthSlot, ONLINE_WINDOW_MS};
use crate::now_ms;

pub fn router(st: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/v1/auth/meta", get(auth_meta))
        .route("/api/v1/auth/login", post(auth_login))
        .route("/api/v1/auth/login/local", post(auth_login_local))
        .route("/api/v1/auth/phone/login", post(auth_phone_login))
        .route("/api/v1/auth/oauth-url", get(auth_oauth_url))
        .route("/api/v1/auth/callback", get(auth_oauth_callback))
        .route("/api/v1/auth/poll", get(auth_oauth_poll))
        .route("/api/v1/auth/me", get(auth_me))
        .route("/api/v1/auth/refresh", post(auth_refresh))
        .route("/api/v1/users", get(users_list))
        .route("/api/v1/users/online", get(users_online))
        .route("/api/v1/heartbeat", post(heartbeat))
        .route("/api/v1/conversations", get(conv_list).post(conv_create))
        .route("/api/v1/conversations/{id}/members", post(conv_add_members))
        .route("/api/v1/conversations/{id}/messages", get(msg_history).post(msg_send))
        .route("/api/v1/files/upload", post(file_upload))
        .route("/api/v1/files/{id}", get(file_download))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(st)
}

// ---------------------------------------------------------------------------
// 错误与鉴权工具
// ---------------------------------------------------------------------------

pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, msg: impl Into<String>) -> Self {
        Self { status, message: msg.into() }
    }

    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, msg)
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg)
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, msg)
    }
}

impl From<ltl_chat_core::error::LtlError> for ApiError {
    fn from(e: ltl_chat_core::error::LtlError) -> Self {
        match e {
            ltl_chat_core::error::LtlError::Auth(m) => Self::unauthorized(m),
            other => Self::new(StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "detail": self.message }))).into_response()
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
}

fn current_user(st: &AppState, headers: &HeaderMap) -> Result<UserRow, ApiError> {
    let token = bearer_token(headers).ok_or_else(|| ApiError::unauthorized("缺少 Bearer token"))?;
    let claims = st.jwt.decode(token)?;
    let uid: UserId = claims
        .sub
        .parse()
        .map_err(|_| ApiError::unauthorized("token 主体非法"))?;
    let user = st
        .db
        .user(uid)?
        .ok_or_else(|| ApiError::unauthorized("用户不存在"))?;
    if !user.is_active {
        return Err(ApiError::unauthorized("用户已停用"));
    }
    let now = now_ms();
    if now - user.last_active_ms > 30_000 {
        let _ = st.db.touch(uid, now);
    }
    Ok(user)
}

// ---------------------------------------------------------------------------
// 登录 / 登记(四态: ym | casdoor | local | token)
// ---------------------------------------------------------------------------

/// 服务器认证能力(无需登录;GUI/CLI 据此渲染登录表单)
async fn auth_meta(State(st): State<Arc<AppState>>) -> ApiResult<serde_json::Value> {
    Ok(Json(serde_json::json!({
        "mode": st.auth.mode.as_str(),
        "demo": st.auth.demo_enabled(),
    })))
}

#[derive(Deserialize)]
struct LoginBody {
    ym_auth_token: String,
}

/// 演示分支(对齐旧系统):`demo_` 前缀 token 不走任何外部 IdP
fn demo_user_info(token: &str) -> ltl_chat_core::ym::YmUserInfo {
    let suffix: String = token.chars().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect();
    ltl_chat_core::ym::YmUserInfo {
        ymid: format!("demo_{token}"),
        name: format!("Demo User ({suffix})"),
        email: Some(format!("demo{suffix}@example.com")),
        avatar: None,
    }
}

/// token 模式:由 token 确定性派生 ymid(不向其他用户泄露 token 本身)
fn token_ymid(token: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in token.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    format!("tok_{h:016x}")
}

async fn issue_token(
    st: &Arc<AppState>,
    info: ltl_chat_core::ym::YmUserInfo,
    ym_token: Option<&str>,
    ym_refresh: Option<&str>,
) -> Result<TokenResponse, ApiError> {
    let now = now_ms();
    let user = st.db.upsert_user_by_ymid(
        &info.ymid,
        &info.name,
        info.email.as_deref(),
        info.avatar.as_deref(),
        None,
        ym_token,
        ym_refresh,
        now,
    )?;
    Ok(TokenResponse {
        access_token: st.jwt.issue(user.id, 24 * 3600)?,
        user: user.to_user(ONLINE_WINDOW_MS, now),
        server_endpoint_id: st.endpoint_id.to_string(),
    })
}

async fn auth_login(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> ApiResult<TokenResponse> {
    let token = body.ym_auth_token.trim();
    // 演示 token 在所有模式可用(受 demo 开关控制)
    if token.starts_with("demo_") {
        if !st.auth.demo_enabled() {
            return Err(ApiError::unauthorized("演示登录已关闭"));
        }
        let resp = issue_token(&st, demo_user_info(token), None, None).await?;
        spawn_roster_sync(&st);
        return Ok(Json(resp));
    }
    match st.auth.mode {
        AuthMode::Ym => {
            let ym = st.ym.as_ref().expect("ym client");
            let Some(info) = ym.validate_token(token).await else {
                return Err(ApiError::unauthorized("YM Auth 校验失败"));
            };
            let resp = issue_token(&st, info, Some(token), None).await?;
            spawn_roster_sync(&st);
            Ok(Json(resp))
        }
        AuthMode::Casdoor => {
            let casdoor = st.casdoor.as_ref().expect("casdoor client");
            let Some(info) = casdoor.validate_token(token).await else {
                return Err(ApiError::unauthorized("Casdoor 校验失败"));
            };
            let resp = issue_token(&st, info, None, None).await?;
            Ok(Json(resp))
        }
        AuthMode::Token => {
            let Some(display_name) = st.auth.tokens.get(token).cloned() else {
                return Err(ApiError::unauthorized("token 无效"));
            };
            let info = ltl_chat_core::ym::YmUserInfo {
                ymid: token_ymid(token),
                name: display_name,
                email: None,
                avatar: None,
            };
            let resp = issue_token(&st, info, None, None).await?;
            Ok(Json(resp))
        }
        AuthMode::Local => Err(ApiError::bad_request(
            "本服务器为内置账号模式, 请用 POST /api/v1/auth/login/local",
        )),
    }
}

#[derive(Deserialize)]
struct LocalLoginBody {
    username: String,
    password: String,
}

/// 内置账号登录(argon2 校验 config.toml [auth.users])
async fn auth_login_local(
    State(st): State<Arc<AppState>>,
    Json(body): Json<LocalLoginBody>,
) -> ApiResult<TokenResponse> {
    if st.auth.mode != AuthMode::Local {
        return Err(ApiError::bad_request("本服务器未启用内置账号模式"));
    }
    let username = body.username.trim();
    let Some(stored_hash) = st.auth.users.get(username) else {
        return Err(ApiError::unauthorized("用户名或密码错误"));
    };
    let verify = || -> bool {
        use argon2::password_hash::PasswordVerifier;
        use argon2::Argon2;
        match argon2::password_hash::PasswordHash::new(stored_hash) {
            Ok(parsed) => Argon2::default().verify_password(body.password.as_bytes(), &parsed).is_ok(),
            Err(_) => false,
        }
    };
    if !verify() {
        return Err(ApiError::unauthorized("用户名或密码错误"));
    }
    let info = ltl_chat_core::ym::YmUserInfo {
        ymid: username.to_string(),
        name: username.to_string(),
        email: None,
        avatar: None,
    };
    let resp = issue_token(&st, info, None, None).await?;
    Ok(Json(resp))
}

// ---------------------------------------------------------------------------
// OAuth 浏览器登录(casdoor 模式):授权页 → 回调换码 → 票据轮询
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct OAuthUrlQuery {
    /// 备用:强制回调 host(如 --public-url), 缺省用请求 Host
    #[serde(default)]
    base: Option<String>,
}

/// 返回 {authorization_url, ticket, redirect_uri};GUI/CLI 打开浏览器后轮询 /poll
async fn auth_oauth_url(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<OAuthUrlQuery>,
    uri: axum::extract::OriginalUri,
) -> ApiResult<ltl_chat_core::api::OAuthStartResponse> {
    if st.auth.mode != AuthMode::Casdoor {
        return Err(ApiError::bad_request("本服务器未启用 casdoor 模式"));
    }
    let (proto, host) = request_proto_host(&headers, uri);
    let redirect_uri = match q.base {
        Some(b) => format!("{}/api/v1/auth/callback", b.trim_end_matches('/')),
        None => ltl_chat_core::casdoor::CasdoorClient::callback_uri(&proto, &host),
    };
    let ticket = uuid::Uuid::new_v4().simple().to_string();
    let casdoor = st.casdoor.as_ref().expect("casdoor client");
    let authorization_url = casdoor.authorize_url(&redirect_uri, &ticket);
    st.oauth.lock().unwrap().insert(
        ticket.clone(),
        OAuthSlot::Pending { state: ticket.clone(), at: Instant::now() },
    );
    Ok(Json(ltl_chat_core::api::OAuthStartResponse {
        authorization_url,
        ticket,
        redirect_uri,
    }))
}

fn request_proto_host(headers: &HeaderMap, uri: axum::extract::OriginalUri) -> (String, String) {
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http")
        .to_string();
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or_else(|| {
            uri.host().map(|h| {
                if let Some(port) = uri.port_u16() {
                    format!("{h}:{port}")
                } else {
                    h.to_string()
                }
            })
        })
        .unwrap_or_else(|| "127.0.0.1:8901".to_string());
    (proto, host)
}

/// Casdoor 回调:换 token + userinfo → 发自家 JWT → 存进票据槽, 回一个简单成功页
async fn auth_oauth_callback(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
    uri: axum::extract::OriginalUri,
) -> Result<axum::response::Html<String>, ApiError> {
    let code = q.get("code").cloned().unwrap_or_default();
    let state = q.get("state").cloned().unwrap_or_default();
    let err = |msg: &str| {
        axum::response::Html(format!(
            "<!DOCTYPE html><meta charset='utf-8'><body style=\"font-family:sans-serif;text-align:center;padding-top:15vh\"><h2>❌ 登录失败</h2><p style='color:#888'>{msg}</p></body>"
        ))
    };
    if code.is_empty() || state.is_empty() {
        return Err(ApiError::bad_request("缺少 code/state"));
    }

    let (proto, host) = request_proto_host(&headers, uri);
    let redirect_uri = ltl_chat_core::casdoor::CasdoorClient::callback_uri(&proto, &host);

    // 取出票据(防重放: 取走即删)
    let slot = st.oauth.lock().unwrap().remove(&state);
    let Some(OAuthSlot::Pending { state: expected, at }) = slot else {
        return Ok(err("登录票据无效或已使用, 请回应用重新发起"));
    };
    if expected != state || at.elapsed() > crate::state::OAUTH_TICKET_TTL {
        return Ok(err("登录票据已过期, 请回应用重新发起"));
    }

    let casdoor = st.casdoor.as_ref().expect("casdoor client");
    let (_access_token, info) = casdoor
        .exchange_code(&code, &redirect_uri)
        .await
        .map_err(ApiError::unauthorized)?;
    let resp = issue_token(&st, info, None, None).await?;

    st.oauth
        .lock()
        .unwrap()
        .insert(state, OAuthSlot::Done { resp });
    tracing::info!("OAuth 登录成功, 票据就绪");
    Ok(axum::response::Html(
        "<!DOCTYPE html><meta charset='utf-8'><body style=\"font-family:sans-serif;text-align:center;padding-top:15vh\"><h2>✅ 登录成功</h2><p style='color:#888'>请回到 ltl-chat 应用</p></body>".into(),
    ))
}

/// GUI/CLI 每 2s 轮询:202 = 未完成,200 = TokenResponse,404 = 票据失效
async fn auth_oauth_poll(
    State(st): State<Arc<AppState>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let Some(ticket) = q.get("ticket") else {
        return ApiError::bad_request("缺少 ticket").into_response();
    };
    let slot = {
        let mut map = st.oauth.lock().unwrap();
        // 顺带清理过期票据
        map.retain(|_, s| match s {
            OAuthSlot::Pending { at, .. } => at.elapsed() <= crate::state::OAUTH_TICKET_TTL,
            OAuthSlot::Done { .. } => true,
        });
        map.get(ticket).cloned()
    };
    match slot {
        Some(OAuthSlot::Done { resp }) => Json(resp).into_response(),
        Some(OAuthSlot::Pending { .. }) => StatusCode::ACCEPTED.into_response(),
        None => ApiError::not_found("票据不存在或已过期").into_response(),
    }
}

#[derive(Deserialize)]
struct PhoneLoginBody {
    phone: String,
    #[serde(default)]
    username: Option<String>,
}

async fn auth_phone_login(
    State(st): State<Arc<AppState>>,
    Json(body): Json<PhoneLoginBody>,
) -> ApiResult<TokenResponse> {
    if st.auth.mode != AuthMode::Ym {
        return Err(ApiError::bad_request("本服务器未启用 YM Auth 手机号登录"));
    }
    let (ym_access, ym_refresh) = st
        .ym
        .as_ref()
        .expect("ym client")
        .phone_login(&body.phone, body.username.as_deref())
        .await
        .map_err(ApiError::unauthorized)?;
    let Some(info) = st.ym.as_ref().expect("ym client").validate_token(&ym_access).await else {
        return Err(ApiError::unauthorized("YM Auth 校验失败"));
    };
    let resp =
        issue_token(&st, info, Some(&ym_access), ym_refresh.as_deref()).await?;
    spawn_roster_sync(&st);
    Ok(Json(resp))
}

async fn auth_me(State(st): State<Arc<AppState>>, headers: HeaderMap) -> ApiResult<User> {
    let me = current_user(&st, &headers)?;
    let now = now_ms();
    Ok(Json(me.to_user(ONLINE_WINDOW_MS, now)))
}

#[derive(Deserialize)]
struct RefreshBody {
    access_token: String,
}

async fn auth_refresh(
    State(st): State<Arc<AppState>>,
    Json(body): Json<RefreshBody>,
) -> ApiResult<TokenResponse> {
    let claims = st.jwt.decode_refreshable(&body.access_token)?;
    let uid: UserId = claims
        .sub
        .parse()
        .map_err(|_| ApiError::unauthorized("token 主体非法"))?;
    let user = st
        .db
        .user(uid)?
        .ok_or_else(|| ApiError::unauthorized("用户不存在"))?;
    let now = now_ms();
    Ok(Json(TokenResponse {
        access_token: st.jwt.issue(user.id, 24 * 3600)?,
        user: user.to_user(ONLINE_WINDOW_MS, now),
        server_endpoint_id: st.endpoint_id.to_string(),
    }))
}

/// 节流的全员同步(60s 一次;仅 ym 模式有意义;demo 用户跳过)
fn spawn_roster_sync(st: &Arc<AppState>) {
    if st.auth.mode != AuthMode::Ym {
        return;
    }
    {
        let mut g = st.last_sync.lock().unwrap();
        if let Some(t) = *g {
            if t.elapsed() < Duration::from_secs(60) {
                return;
            }
        }
        *g = Some(Instant::now());
    }
    let st = st.clone();
    tokio::spawn(async move {
        // 取任一持有有效 YM token 的活跃用户作同步凭据
        let rows = match st.db.list_users() {
            Ok(r) => r,
            Err(_) => return,
        };
        let token = rows
            .iter()
            .find(|r| r.is_active && !r.ymid.starts_with("demo_"))
            .and_then(|r| r.ym_auth_token.clone());
        let Some(token) = token else { return };
        let list = st.ym.as_ref().expect("ym client").users_list(&token).await;
        if list.is_empty() {
            return;
        }
        if let Err(e) = st.db.sync_roster(&list, now_ms()) {
            tracing::warn!("roster 同步失败: {e}");
        } else {
            tracing::info!("roster 同步完成: {} 人", list.len());
        }
    });
}

// ---------------------------------------------------------------------------
// 名单 / 在线
// ---------------------------------------------------------------------------

async fn users_list(State(st): State<Arc<AppState>>, headers: HeaderMap) -> ApiResult<Vec<User>> {
    let _me = current_user(&st, &headers)?;
    spawn_roster_sync(&st);
    let now = now_ms();
    let rows = st.db.list_users()?;
    Ok(Json(
        rows.iter()
            .filter(|r| r.is_active)
            .map(|r| r.to_user(ONLINE_WINDOW_MS, now))
            .collect(),
    ))
}

#[derive(Serialize)]
struct OnlineUser {
    user_id: UserId,
}

async fn users_online(State(st): State<Arc<AppState>>, headers: HeaderMap) -> ApiResult<Vec<OnlineUser>> {
    current_user(&st, &headers)?;
    let now = now_ms();
    let rows = st.db.list_users()?;
    Ok(Json(
        rows.iter()
            .filter(|r| r.is_active && now - r.last_active_ms < ONLINE_WINDOW_MS)
            .map(|r| OnlineUser { user_id: r.id })
            .collect(),
    ))
}

async fn heartbeat(State(st): State<Arc<AppState>>, headers: HeaderMap) -> ApiResult<serde_json::Value> {
    let me = current_user(&st, &headers)?;
    st.db.touch(me.id, now_ms())?;
    Ok(Json(json!({ "message": "ok" })))
}

// ---------------------------------------------------------------------------
// 会话
// ---------------------------------------------------------------------------

async fn conv_list(State(st): State<Arc<AppState>>, headers: HeaderMap) -> ApiResult<Vec<Conversation>> {
    let me = current_user(&st, &headers)?;
    Ok(Json(st.db.conversations_for_user(me.id)?))
}

#[derive(Deserialize)]
struct CreateConvBody {
    conv_type: ConvType,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    member_ids: Vec<UserId>,
}

async fn conv_create(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateConvBody>,
) -> ApiResult<Conversation> {
    let me = current_user(&st, &headers)?;
    let now = now_ms();

    if body.conv_type == ConvType::Single {
        let Some(&other) = body.member_ids.first() else {
            return Err(ApiError::bad_request("单聊需要 member_ids[0]"));
        };
        if let Some(existing) = st.db.find_single_conversation(me.id, other)? {
            let conv = st
                .db
                .conversation(existing)?
                .ok_or_else(|| ApiError::not_found("会话不存在"))?;
            return Ok(Json(conv));
        }
    }

    if body.conv_type == ConvType::Group && body.title.as_deref().unwrap_or("").is_empty() {
        return Err(ApiError::bad_request("群聊需要 title"));
    }

    let topic = TopicId::from_bytes(rand::random::<[u8; 32]>());
    let topic_hex = hex::encode(topic.as_bytes());
    let conv_id = st.db.create_conversation(
        body.conv_type,
        body.title.as_deref(),
        me.id,
        &body.member_ids,
        &topic_hex,
        now,
    )?;
    st.watch_topic(topic_hex);
    let conv = st
        .db
        .conversation(conv_id)?
        .ok_or_else(|| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "会话创建失败"))?;
    Ok(Json(conv))
}

#[derive(Deserialize)]
struct AddMembersBody {
    member_ids: Vec<UserId>,
}

async fn conv_add_members(
    State(st): State<Arc<AppState>>,
    AxPath(id): AxPath<i64>,
    headers: HeaderMap,
    Json(body): Json<AddMembersBody>,
) -> ApiResult<serde_json::Value> {
    let me = current_user(&st, &headers)?;
    if !st.db.is_member(id, me.id)? {
        return Err(ApiError::not_found("会话不存在或非成员"));
    }
    st.db.add_members(id, &body.member_ids)?;
    Ok(Json(json!({ "message": "ok" })))
}

// ---------------------------------------------------------------------------
// 历史消息(离线补齐)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct HistoryQuery {
    #[serde(default)]
    after_seq: Option<i64>,
    #[serde(default)]
    before_seq: Option<i64>,
    #[serde(default = "default_history_limit")]
    limit: usize,
}

fn default_history_limit() -> usize {
    100
}

/// 历史消息(带服务器序号, 客户端据此增量补齐)
#[derive(Serialize)]
struct HistoryMessage {
    message: ltl_chat_core::types::Message,
    server_seq: i64,
}

async fn msg_history(
    State(st): State<Arc<AppState>>,
    AxPath(id): AxPath<i64>,
    headers: HeaderMap,
    Query(q): Query<HistoryQuery>,
) -> ApiResult<Vec<HistoryMessage>> {
    let me = current_user(&st, &headers)?;
    if !st.db.is_member(id, me.id)? {
        return Err(ApiError::not_found("会话不存在或非成员"));
    }
    let limit = q.limit.clamp(1, 500);
    let rows = st.db.history_page(id, q.after_seq, q.before_seq, limit)?;
    Ok(Json(
        rows.into_iter().map(|(seq, message)| HistoryMessage { message, server_seq: seq }).collect(),
    ))
}

#[derive(Deserialize)]
struct SendMessageBody {
    #[serde(default)]
    text: String,
    #[serde(default)]
    attachments: Vec<AttachmentMeta>,
    #[serde(default)]
    reply_to: Option<MessageId>,
}

/// HTTP 发消息(ltl-manage 内嵌页面等无 iroh 客户端使用):
/// 鉴权 → 成员校验 → 构造 Message → 经已订阅 topic 的 sender gossip 广播
/// (在线 GUI 对端秒收;iroh 不回显, 服务器自身只落库一次)→ 落库取 server_seq。
/// 广播失败不阻塞返回——对端还有 30s 增量 backfill 兜底。
async fn msg_send(
    State(st): State<Arc<AppState>>,
    AxPath(id): AxPath<i64>,
    headers: HeaderMap,
    Json(body): Json<SendMessageBody>,
) -> ApiResult<HistoryMessage> {
    let me = current_user(&st, &headers)?;
    if !st.db.is_member(id, me.id)? {
        return Err(ApiError::not_found("会话不存在或非成员"));
    }
    let text = body.text.trim();
    if text.is_empty() && body.attachments.is_empty() {
        return Err(ApiError::bad_request("消息内容为空"));
    }
    let conv = st
        .db
        .conversation(id)?
        .ok_or_else(|| ApiError::not_found("会话不存在"))?;
    let message = Message {
        reply_to: body.reply_to,
        attachments: body.attachments,
        ..Message::new_text(id, me.id, &me.name, text.to_string())
    };
    let envelope = GossipEnvelope::new(GossipPayload::Message(message.clone()));
    let bytes = envelope.to_bytes();
    // 先取克隆结束锁语句, 避免 MutexGuard 跨 .await (future 需 Send)
    let sender = st.senders.lock().unwrap().get(&conv.topic_id).cloned();
    if let Some(sender) = sender {
        if let Err(e) = sender.broadcast(bytes.into()).await {
            tracing::warn!("会话 {id} HTTP 消息 gossip 广播失败(对端将靠增量补齐): {e}");
        }
    } else {
        tracing::warn!("会话 {id} topic sender 未就绪, 仅落库(对端将靠增量补齐)");
    }
    let server_seq = st.db.insert_message(&message)?;
    Ok(Json(HistoryMessage { message, server_seq }))
}

// ---------------------------------------------------------------------------
// 附件
// ---------------------------------------------------------------------------

async fn file_upload(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> ApiResult<AttachmentMeta> {
    current_user(&st, &headers)?;
    let field = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("multipart 解析失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("缺少文件字段"))?;

    let file_name = field.file_name().unwrap_or("file").to_string();
    let mime = field.content_type().unwrap_or("application/octet-stream").to_string();
    let bytes = field
        .bytes()
        .await
        .map_err(|e| ApiError::bad_request(format!("读取文件失败: {e}")))?;

    let safe: String = file_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' || !c.is_ascii() { c } else { '_' })
        .collect();
    let stored_name = format!("{}_{}", uuid::Uuid::new_v4().simple(), safe);
    let path = st.uploads_dir.join(&stored_name);
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("写文件失败: {e}")))?;

    let id = st.db.insert_attachment(&file_name, &stored_name, bytes.len() as i64, &mime)?;
    Ok(Json(AttachmentMeta {
        id,
        file_name,
        file_size: bytes.len() as i64,
        file_type: mime,
        url: format!("/api/v1/files/{id}"),
    }))
}

async fn file_download(
    State(st): State<Arc<AppState>>,
    AxPath(id): AxPath<i64>,
) -> Result<Response, ApiError> {
    let row = st
        .db
        .attachment(id)?
        .ok_or_else(|| ApiError::not_found("附件不存在"))?;
    let bytes = tokio::fs::read(st.uploads_dir.join(&row.file_path))
        .await
        .map_err(|e| ApiError::not_found(format!("附件文件缺失: {e}")))?;
    let mime = if row.file_type.is_empty() { "application/octet-stream" } else { &row.file_type };
    Ok((
        [
            (header::CONTENT_TYPE, mime.to_string()),
            (
                header::CONTENT_LENGTH,
                bytes.len().to_string(),
            ),
        ],
        bytes,
    )
        .into_response())
}
