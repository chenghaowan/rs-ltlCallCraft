//! GUI 后台线程:tokio runtime 中的 BackendActor。
//!
//! 数据流:REST/gossip → 写 ClientStore(SQLite)→ 发 `Evt::DataChanged`
//! → GUI 下帧直接读 store 渲染。发送走 gossip 主题;iroh 不回显自己的
//! broadcast,所以发送方本地立即落库。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_lite::StreamExt;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use uuid::Uuid;

use ltl_chat_core::api::{ChatApiClient, TokenResponse};
use ltl_chat_core::node::client_key_file;
use ltl_chat_core::iroh::EndpointId;
use ltl_chat_core::iroh_gossip::api::{Event, GossipSender};
use ltl_chat_core::node::ChatNode;
use ltl_chat_core::store::ClientStore;
use ltl_chat_core::types::{
    ConvType, Conversation, GossipEnvelope, GossipPayload, Message, MsgType, User, UserId,
};

// ---------------------------------------------------------------------------
// 命令 / 事件
// ---------------------------------------------------------------------------

#[derive(Debug)]
#[allow(dead_code)] // TryResume 由 backend 启动时直接调用; RefreshUsers 备用
pub enum Cmd {
    LoginByToken { server: String, token: String },
    LoginByPhone { server: String, phone: String, username: Option<String> },
    TryResume { server: String },
    Logout,
    RefreshConversations,
    RefreshUsers,
    /// 打开会话:加入 topic + 增量历史补齐 + 标记已读
    OpenConversation { conv_id: i64 },
    /// 向上翻更早的服务器历史
    LoadOlder { conv_id: i64 },
    SendText { conv_id: i64, text: String },
    SendFile { conv_id: i64, path: PathBuf, as_image: bool },
    Recall { conv_id: i64, message_id: Uuid },
    CreateSingle { user_id: UserId },
    CreateGroup { title: String, member_ids: Vec<UserId> },
    AddMembers { conv_id: i64, member_ids: Vec<UserId> },
    SetPinned { conv_id: i64, pinned: bool },
    DownloadAttachment { url: String, save_path: PathBuf },
    FetchImageData { url: String },
}

pub enum Evt {
    LoginOk { user: User },
    LoginFailed(String),
    LoggedOut(String),
    /// 本地库有更新(会话/消息/名单), GUI 下帧重读
    DataChanged,
    /// 非致命错误提示
    Error(String),
    /// 图片下载并解码完成
    ImageReady { url: String, width: u32, height: u32, rgba: Vec<u8> },
    FileSaved { path: PathBuf },
}

// ---------------------------------------------------------------------------
// topic 注册表(发送方句柄 + 已加入集合, actor 与轮询任务共享)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TopicRegistry {
    joined: HashSet<i64>,
    senders: HashMap<i64, GossipSender>,
}

type SharedRegistry = Arc<Mutex<TopicRegistry>>;

// ---------------------------------------------------------------------------
// Actor
// ---------------------------------------------------------------------------

struct Session {
    user: User,
    api: ChatApiClient,
    node: Arc<ChatNode>,
    server_endpoint: Option<EndpointId>,
}

pub struct Backend {
    store: Arc<ClientStore>,
    data_dir: PathBuf,
    evt: UnboundedSender<Evt>,
    cmd_rx: UnboundedReceiver<Cmd>,
    session: Option<Session>,
    registry: SharedRegistry,
    generation: Arc<AtomicU64>,
}

enum LoginKind {
    Token(String),
    Phone { phone: String, username: Option<String> },
}

pub fn spawn_backend(
    store: Arc<ClientStore>,
    data_dir: PathBuf,
) -> (UnboundedSender<Cmd>, UnboundedReceiver<Evt>) {
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (evt_tx, evt_rx) = unbounded_channel();
    std::thread::Builder::new()
        .name("ltl-chat-backend".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                let mut actor = Backend {
                    store,
                    data_dir,
                    evt: evt_tx,
                    cmd_rx,
                    session: None,
                    registry: Arc::new(Mutex::new(TopicRegistry::default())),
                    generation: Arc::new(AtomicU64::new(0)),
                };
                actor.try_resume().await;
                while let Some(cmd) = actor.cmd_rx.recv().await {
                    actor.handle(cmd).await;
                }
            });
        })
        .expect("spawn backend thread");
    (cmd_tx, evt_rx)
}

impl Backend {
    fn send(&self, evt: Evt) {
        let _ = self.evt.send(evt);
    }

    async fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::LoginByToken { server, token } => {
                self.login(&server, LoginKind::Token(token)).await
            }
            Cmd::LoginByPhone { server, phone, username } => {
                self.login(&server, LoginKind::Phone { phone, username }).await
            }
            Cmd::TryResume { server } => self.try_resume_at(&server).await,
            Cmd::Logout => self.logout("已退出登录"),
            Cmd::RefreshConversations => self.refresh_conversations().await,
            Cmd::RefreshUsers => self.refresh_users().await,
            Cmd::OpenConversation { conv_id } => self.open_conversation(conv_id).await,
            Cmd::LoadOlder { conv_id } => self.load_older(conv_id).await,
            Cmd::SendText { conv_id, text } => self.send_text(conv_id, text).await,
            Cmd::SendFile { conv_id, path, as_image } => {
                self.send_file(conv_id, path, as_image).await
            }
            Cmd::Recall { conv_id, message_id } => self.recall(conv_id, message_id).await,
            Cmd::CreateSingle { user_id } => self.create_single(user_id).await,
            Cmd::CreateGroup { title, member_ids } => self.create_group(title, member_ids).await,
            Cmd::AddMembers { conv_id, member_ids } => self.add_members(conv_id, member_ids).await,
            Cmd::SetPinned { conv_id, pinned } => {
                let _ = self.store.set_pinned(conv_id, pinned);
                self.send(Evt::DataChanged);
            }
            Cmd::DownloadAttachment { url, save_path } => self.download(url, save_path).await,
            Cmd::FetchImageData { url } => self.fetch_image(url).await,
        }
    }

    // -----------------------------------------------------------------
    // 登录
    // -----------------------------------------------------------------

    async fn login(&mut self, server: &str, kind: LoginKind) {
        let api = ChatApiClient::new(server);
        let resp: Result<TokenResponse, ltl_chat_core::error::LtlError> = match kind {
            LoginKind::Token(t) => api.login_by_token(&t).await,
            LoginKind::Phone { phone, username } => {
                api.login_by_phone(&phone, username.as_deref()).await
            }
        };
        match resp {
            Err(e) => self.send(Evt::LoginFailed(format!("{e}"))),
            Ok(resp) => self.establish(server, resp).await,
        }
    }

    async fn try_resume(&mut self) {
        let Ok(Some(server)) = self.store.kv_get("server") else { return };
        self.try_resume_at(&server).await;
    }

    async fn try_resume_at(&mut self, server: &str) {
        let token = match self.store.kv_get("token") {
            Ok(Some(t)) if !t.is_empty() => t,
            _ => return,
        };
        let api = ChatApiClient::with_token(server, &token);
        // 优先续签, 续不上直接试旧 token
        let resp = match api.refresh(&token).await {
            Ok(r) => Some(r),
            Err(_) => api.me().await.ok().map(|user| TokenResponse {
                access_token: token.clone(),
                user,
                server_endpoint_id: self
                    .store
                    .kv_get("server_endpoint_id")
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
            }),
        };
        match resp {
            Some(resp) => self.establish(server, resp).await,
            None => {
                let _ = self.store.kv_set("token", "");
            }
        }
    }

    async fn establish(&mut self, server: &str, resp: TokenResponse) {
        let _ = self.store.kv_set("server", server);
        let _ = self.store.kv_set("token", &resp.access_token);
        let _ = self.store.kv_set("server_endpoint_id", &resp.server_endpoint_id);
        let _ = self.store.upsert_user(&resp.user);
        let _ = self.store.kv_set("user_id", &resp.user.id.to_string());

        let node = match ChatNode::spawn(&client_key_file(&self.data_dir)).await {
            Ok(n) => Arc::new(n),
            Err(e) => {
                self.send(Evt::LoginFailed(format!("iroh 节点启动失败: {e}")));
                return;
            }
        };

        let server_endpoint = resp.server_endpoint_id.parse::<EndpointId>().ok();
        if server_endpoint.is_none() && !resp.server_endpoint_id.is_empty() {
            tracing::warn!("服务器 endpoint id 解析失败: {}", resp.server_endpoint_id);
        }

        self.registry.lock().unwrap().joined.clear();
        self.registry.lock().unwrap().senders.clear();
        self.session = Some(Session {
            user: resp.user.clone(),
            api: ChatApiClient::with_token(server, &resp.access_token),
            node,
            server_endpoint,
        });

        self.send(Evt::LoginOk { user: resp.user });
        self.refresh_conversations().await;
        self.refresh_users().await;

        // 周期任务:心跳 + 名单/在线 + 会话轮询(30s, 对齐旧系统节奏)
        let gen = self.generation.load(Ordering::SeqCst);
        let session = self.session.as_ref().expect("session just set");
        let api = session.api.clone();
        let node = session.node.clone();
        let server_ep = session.server_endpoint;
        let store = self.store.clone();
        let evt = self.evt.clone();
        let registry = self.registry.clone();
        let generation = self.generation.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                if gen != generation.load(Ordering::SeqCst) {
                    return;
                }
                if let Err(e) = api.heartbeat().await {
                    tracing::debug!("心跳失败: {e}");
                }
                if let Ok(users) = api.users().await {
                    for u in &users {
                        let _ = store.upsert_user(u);
                    }
                    let _ = evt.send(Evt::DataChanged);
                }
                if let Ok(convs) = api.conversations().await {
                    for c in &convs {
                        join_and_listen(&node, &store, &evt, c, server_ep, &registry).await;
                    }
                    let _ = store.replace_conversations(&convs);
                    let _ = evt.send(Evt::DataChanged);
                }
            }
        });
    }

    fn logout(&mut self, reason: &str) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.registry.lock().unwrap().joined.clear();
        self.registry.lock().unwrap().senders.clear();
        self.session = None;
        let _ = self.store.kv_set("token", "");
        self.send(Evt::LoggedOut(reason.to_string()));
    }

    // -----------------------------------------------------------------
    // 数据刷新
    // -----------------------------------------------------------------

    async fn refresh_conversations(&mut self) {
        let Some(session) = &self.session else { return };
        match session.api.conversations().await {
            Ok(convs) => {
                for c in &convs {
                    join_and_listen(
                        &session.node,
                        &self.store,
                        &self.evt,
                        c,
                        session.server_endpoint,
                        &self.registry,
                    )
                    .await;
                }
                let _ = self.store.replace_conversations(&convs);
                self.send(Evt::DataChanged);
            }
            Err(e) => self.send(Evt::Error(format!("刷新会话失败: {e}"))),
        }
    }

    async fn refresh_users(&mut self) {
        let Some(session) = &self.session else { return };
        match session.api.users().await {
            Ok(users) => {
                for u in &users {
                    let _ = self.store.upsert_user(u);
                }
                self.send(Evt::DataChanged);
            }
            Err(e) => self.send(Evt::Error(format!("刷新名单失败: {e}"))),
        }
    }

    async fn open_conversation(&mut self, conv_id: i64) {
        let Some(session) = &mut self.session else { return };
        if let Ok(Some(conv)) = self.store.get_conversation(conv_id) {
            join_and_listen(
                &session.node,
                &self.store,
                &self.evt,
                &conv,
                session.server_endpoint,
                &self.registry,
            )
            .await;
        }
        // 增量历史补齐
        let after = self.store.max_server_seq(conv_id).unwrap_or(0);
        match session.api.history(conv_id, after, 200).await {
            Ok(items) => {
                for hm in items {
                    match self.store.insert_message(&hm.message, Some(hm.server_seq)) {
                        Ok(true) => self.send(Evt::DataChanged),
                        Ok(false) => {
                            let _ = self.store.fill_server_seq(&hm.message.id.to_string(), hm.server_seq);
                        }
                        Err(e) => tracing::warn!("历史消息入库失败: {e}"),
                    }
                }
            }
            Err(e) => self.send(Evt::Error(format!("历史同步失败: {e}"))),
        }
        let _ = self.store.mark_read(conv_id);
        self.send(Evt::DataChanged);
    }

    async fn load_older(&mut self, conv_id: i64) {
        let Some(session) = &self.session else { return };
        let before = self
            .store
            .list_messages(conv_id, 1, None)
            .ok()
            .and_then(|v| v.first().and_then(|m| m.server_seq))
            .unwrap_or(i64::MAX);
        if before == i64::MAX {
            return;
        }
        match session.api.history_before(conv_id, before, 100).await {
            Ok(items) => {
                for hm in items {
                    match self.store.insert_message(&hm.message, Some(hm.server_seq)) {
                        Ok(true) => self.send(Evt::DataChanged),
                        Ok(false) => {
                            let _ = self.store.fill_server_seq(&hm.message.id.to_string(), hm.server_seq);
                        }
                        Err(e) => tracing::warn!("历史消息入库失败: {e}"),
                    }
                }
            }
            Err(e) => self.send(Evt::Error(format!("历史翻页失败: {e}"))),
        }
    }

    // -----------------------------------------------------------------
    // 发送
    // -----------------------------------------------------------------

    async fn send_text(&mut self, conv_id: i64, text: String) {
        if text.trim().is_empty() {
            return;
        }
        let Some(session) = &self.session else { return };
        let msg = Message::new_text(conv_id, session.user.id, &session.user.name, text);
        self.deliver(conv_id, GossipPayload::Message(msg)).await;
    }

    async fn send_file(&mut self, conv_id: i64, path: PathBuf, as_image: bool) {
        let Some(session) = &self.session else { return };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.send(Evt::Error(format!("读取文件失败: {e}")));
                return;
            }
        };
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into());

        let (payload_bytes, mime, msg_type) = if as_image {
            let compressed = compress_image_png(&bytes, 256);
            let b = compressed.unwrap_or(bytes);
            (b, "image/png", MsgType::Image)
        } else {
            (bytes, guess_mime(&file_name), MsgType::File)
        };

        match session
            .api
            .upload_attachment(payload_bytes, &file_name, mime)
            .await
        {
            Ok(meta) => {
                let msg = Message::new_attachment(
                    conv_id,
                    session.user.id,
                    &session.user.name,
                    msg_type,
                    String::new(),
                    vec![meta],
                );
                self.deliver(conv_id, GossipPayload::Message(msg)).await;
            }
            Err(e) => self.send(Evt::Error(format!("附件上传失败: {e}"))),
        }
    }

    async fn recall(&mut self, conv_id: i64, message_id: Uuid) {
        let Some(session) = &self.session else { return };
        let mine = self
            .store
            .get_message(&message_id.to_string())
            .ok()
            .flatten()
            .map(|lm| lm.message.sender_id == session.user.id)
            .unwrap_or(false);
        if !mine {
            self.send(Evt::Error("只能撤回自己的消息".into()));
            return;
        }
        let payload = GossipPayload::Recall {
            conversation_id: conv_id,
            message_id,
            sender_id: session.user.id,
        };
        self.deliver(conv_id, payload).await;
        let _ = self.store.mark_recalled(conv_id, &message_id.to_string());
        self.send(Evt::DataChanged);
    }

    /// 确保 topic 已加入并广播;iroh 不回显, 发送方消息本地落库
    async fn deliver(&mut self, conv_id: i64, payload: GossipPayload) {
        let Some(session) = &mut self.session else { return };
        ensure_joined(
            &session.node,
            &self.store,
            &self.evt,
            conv_id,
            session.server_endpoint,
            &self.registry,
        )
        .await;
        let sender = self.registry.lock().unwrap().senders.get(&conv_id).cloned();
        match sender {
            Some(s) => {
                let envelope = GossipEnvelope::new(payload.clone());
                let bytes = envelope.to_bytes();
                if let Err(e) = s.broadcast(bytes.into()).await {
                    self.send(Evt::Error(format!("发送失败: {e}")));
                    return;
                }
                if let GossipPayload::Message(m) = &payload {
                    let _ = self.store.insert_message(m, None);
                }
                self.send(Evt::DataChanged);
            }
            None => self.send(Evt::Error("会话未连接(等待 gossip 加入), 请稍后重试".into())),
        }
    }

    // -----------------------------------------------------------------
    // 会话创建
    // -----------------------------------------------------------------

    async fn create_single(&mut self, user_id: UserId) {
        let Some(session) = &self.session else { return };
        match session.api.create_conversation(ConvType::Single, None, vec![user_id]).await {
            Ok(conv) => {
                let _ = self.store.replace_conversations(std::slice::from_ref(&conv));
                join_and_listen(
                    &session.node,
                    &self.store,
                    &self.evt,
                    &conv,
                    session.server_endpoint,
                    &self.registry,
                )
                .await;
                self.send(Evt::DataChanged);
            }
            Err(e) => self.send(Evt::Error(format!("发起会话失败: {e}"))),
        }
    }

    async fn create_group(&mut self, title: String, member_ids: Vec<UserId>) {
        let Some(session) = &self.session else { return };
        match session
            .api
            .create_conversation(ConvType::Group, Some(&title), member_ids)
            .await
        {
            Ok(conv) => {
                let _ = self.store.replace_conversations(std::slice::from_ref(&conv));
                join_and_listen(
                    &session.node,
                    &self.store,
                    &self.evt,
                    &conv,
                    session.server_endpoint,
                    &self.registry,
                )
                .await;
                self.send(Evt::DataChanged);
            }
            Err(e) => self.send(Evt::Error(format!("建群失败: {e}"))),
        }
    }

    async fn add_members(&mut self, conv_id: i64, member_ids: Vec<UserId>) {
        let Some(session) = &self.session else { return };
        match session.api.add_members(conv_id, member_ids).await {
            Ok(()) => {
                self.refresh_conversations().await;
            }
            Err(e) => self.send(Evt::Error(format!("拉人失败: {e}"))),
        }
    }

    // -----------------------------------------------------------------
    // 附件下载 / 图片
    // -----------------------------------------------------------------

    async fn download(&mut self, url: String, save_path: PathBuf) {
        let Some(session) = &self.session else { return };
        let abs = session.api.absolute_url(&url);
        match session.api.download(&abs).await {
            Ok(bytes) => match std::fs::write(&save_path, &bytes) {
                Ok(()) => self.send(Evt::FileSaved { path: save_path }),
                Err(e) => self.send(Evt::Error(format!("保存失败: {e}"))),
            },
            Err(e) => self.send(Evt::Error(format!("下载失败: {e}"))),
        }
    }

    async fn fetch_image(&mut self, url: String) {
        let Some(session) = &self.session else { return };
        let abs = session.api.absolute_url(&url);
        match session.api.download(&abs).await {
            Ok(bytes) => match image::load_from_memory(&bytes) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    self.send(Evt::ImageReady { url, width: w, height: h, rgba: rgba.into_raw() });
                }
                Err(e) => self.send(Evt::Error(format!("图片解码失败: {e}"))),
            },
            Err(e) => self.send(Evt::Error(format!("图片下载失败: {e}"))),
        }
    }
}

// ---------------------------------------------------------------------------
// topic 加入与收包循环
// ---------------------------------------------------------------------------

/// 未加入则加入并 spawn 收包循环(幂等)
async fn ensure_joined(
    node: &Arc<ChatNode>,
    store: &Arc<ClientStore>,
    evt: &UnboundedSender<Evt>,
    conv_id: i64,
    server_ep: Option<EndpointId>,
    registry: &SharedRegistry,
) {
    if registry.lock().unwrap().joined.contains(&conv_id) {
        return;
    }
    if let Ok(Some(conv)) = store.get_conversation(conv_id) {
        join_and_listen(node, store, evt, &conv, server_ep, registry).await;
    }
}

async fn join_and_listen(
    node: &Arc<ChatNode>,
    store: &Arc<ClientStore>,
    evt: &UnboundedSender<Evt>,
    conv: &Conversation,
    server_ep: Option<EndpointId>,
    registry: &SharedRegistry,
) {
    {
        let mut reg = registry.lock().unwrap();
        if reg.joined.contains(&conv.id) {
            return;
        }
        reg.joined.insert(conv.id);
    }
    let topic = match node.join_topic(&conv.topic_id, server_ep).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("加入会话 {} 的 topic 失败: {e}", conv.id);
            registry.lock().unwrap().joined.remove(&conv.id);
            return;
        }
    };
    let (sender, mut receiver) = topic.split();
    registry.lock().unwrap().senders.insert(conv.id, sender);

    let conv_id = conv.id;
    let store = store.clone();
    let evt = evt.clone();
    let registry = registry.clone();
    tokio::spawn(async move {
        while let Some(item) = receiver.next().await {
            match item {
                Ok(Event::Received(msg)) => {
                    let Ok(envelope) = GossipEnvelope::from_bytes(&msg.content) else { continue };
                    match envelope.payload {
                        GossipPayload::Message(m) => {
                            if m.conversation_id != conv_id {
                                continue;
                            }
                            if store.insert_message(&m, None).unwrap_or(false) {
                                let _ = evt.send(Evt::DataChanged);
                            }
                        }
                        GossipPayload::Recall { conversation_id, message_id, .. } => {
                            let _ = store.mark_recalled(conversation_id, &message_id.to_string());
                            let _ = evt.send(Evt::DataChanged);
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::debug!("topic {conv_id} 收包错误: {e}");
                    break;
                }
            }
        }
        let mut reg = registry.lock().unwrap();
        reg.joined.remove(&conv_id);
        reg.senders.remove(&conv_id);
    });
}

// ---------------------------------------------------------------------------
// 图片压缩(移植 frontR compressImage:最长边 ≤ maxSize 的 PNG)
// ---------------------------------------------------------------------------

fn compress_image_png(bytes: &[u8], max_size: u32) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    let (w, h) = (img.width(), img.height());
    let img = if w > max_size || h > max_size {
        let scale = f64::min(max_size as f64 / w as f64, max_size as f64 / h as f64);
        let nw = ((w as f64) * scale).round().max(1.0) as u32;
        let nh = ((h as f64) * scale).round().max(1.0) as u32;
        img.resize_exact(nw, nh, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

fn guess_mime(file_name: &str) -> &'static str {
    let lower = file_name.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".pdf") {
        "application/pdf"
    } else if lower.ends_with(".txt") {
        "text/plain"
    } else if lower.ends_with(".zip") {
        "application/zip"
    } else if lower.ends_with(".doc") || lower.ends_with(".docx") {
        "application/msword"
    } else if lower.ends_with(".xls") || lower.ends_with(".xlsx") {
        "application/vnd.ms-excel"
    } else {
        "application/octet-stream"
    }
}
