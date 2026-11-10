//! GUI 应用状态机:事件泵、本地库重读、布局编排。
//! 渲染细节在 views.rs(同一类型的另一个 impl 块)。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use uuid::Uuid;

use ltl_chat_core::store::{ClientStore, ConversationView};
use ltl_chat_core::types::{LocalMessage, User, UserId};

use crate::backend::{spawn_backend, Cmd, Evt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Chats,
    Contacts,
}

struct Toast {
    text: String,
    at: Instant,
}

pub struct LtlChatApp {
    pub cmd: UnboundedSender<Cmd>,
    pub evt_rx: UnboundedReceiver<Evt>,
    pub store: Arc<ClientStore>,
    pub data_dir: PathBuf,
    pub server_default: String,

    // 登录页
    pub login_server: String,
    /// 服务器认证能力 (mode, demo)
    pub auth_meta: Option<(String, bool)>,
    /// 浏览器 OAuth/SSO 登录等待中(ym / casdoor 模式)
    pub oauth_waiting: bool,
    pub login_username: String,
    pub login_password: String,
    pub login_token: String,
    pub login_error: Option<String>,
    pub login_busy: bool,

    // 会话态
    pub me: Option<User>,
    pub view: View,
    pub dirty: bool,
    pub convs: Vec<ConversationView>,
    pub active_conv: Option<i64>,
    pub messages: Vec<LocalMessage>,
    pub messages_dirty: bool,
    /// 创建单聊后等待自动打开的成员 id
    pub pending_open_user: Option<UserId>,

    // 输入与交互
    pub input: String,
    pub reply_to: Option<Uuid>,
    pub show_emoji: bool,
    pub search: String,
    pub contacts_search: String,
    pub show_group_modal: bool,
    pub group_title: String,
    pub group_picked: HashSet<i64>,
    pub members_modal: Option<i64>,
    pub members_picked: HashSet<i64>,

    // 渲染辅助
    toasts: Vec<Toast>,
    pub textures: HashMap<String, egui::TextureHandle>,
    pub image_pending: HashSet<String>,
    /// 待建纹理队列(需 ctx, update 开头物化)
    pending_textures: Vec<(String, egui::ColorImage)>,
    pub last_msg_count: usize,
    pub scroll_bottom: bool,
}

impl LtlChatApp {
    pub fn new(cc: &eframe::CreationContext<'_>, profile: &str, server_default: String) -> Self {
        crate::assets::install_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        let data_dir = ltl_chat_core::util::data_dir(profile);
        let store = Arc::new(ClientStore::open(&data_dir.join("client.db")).expect("打开本地库失败"));
        let (cmd, evt_rx) = spawn_backend(store.clone(), data_dir.clone());
        let _ = cmd.send(Cmd::LoadAuthMeta { server: server_default.clone() });
        Self {
            cmd,
            evt_rx,
            store,
            data_dir,
            login_server: server_default.clone(),
            auth_meta: None,
            oauth_waiting: false,
            server_default: server_default.clone(),
            login_username: String::new(),
            login_password: String::new(),
            login_token: String::new(),
            login_error: None,
            login_busy: false,
            me: None,
            view: View::Chats,
            dirty: true,
            convs: Vec::new(),
            active_conv: None,
            messages: Vec::new(),
            messages_dirty: false,
            pending_open_user: None,
            input: String::new(),
            reply_to: None,
            show_emoji: false,
            search: String::new(),
            contacts_search: String::new(),
            show_group_modal: false,
            group_title: String::new(),
            group_picked: HashSet::new(),
            members_modal: None,
            members_picked: HashSet::new(),
            toasts: Vec::new(),
            textures: HashMap::new(),
            image_pending: HashSet::new(),
            pending_textures: Vec::new(),
            last_msg_count: 0,
            scroll_bottom: true,
        }
    }

    // -----------------------------------------------------------------
    // 布局编排
    // -----------------------------------------------------------------

    fn update(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.drain_events();
        self.materialize_textures(&ctx);
        self.draw_toasts(&ctx);
        if self.dirty {
            self.reload_convs();
            self.dirty = false;
        }
        if self.messages_dirty {
            self.reload_messages();
            self.messages_dirty = false;
        }
        // 持续重绘:gossip 收包通过事件驱动, 这里保底低频刷新(在线点/时钟)
        ctx.request_repaint_after(Duration::from_millis(500));

        if self.me.is_none() {
            self.show_login(ui);
            return;
        }

        egui::Panel::left("rail")
            .exact_size(56.0)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(crate::assets::BG_LIST))
            .show(ui, |ui| self.show_rail(ui));

        egui::Panel::left("list")
            .exact_size(280.0)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(crate::assets::BG_LIST))
            .show(ui, |ui| match self.view {
                View::Chats => self.show_chats_list(ui),
                View::Contacts => self.show_contacts_list(ui),
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::WHITE))
            .show(ui, |ui| self.show_chat_panel(ui));

        if self.show_group_modal {
            self.show_group_modal_ui(&ctx);
        }
        if self.members_modal.is_some() {
            self.show_members_modal_ui(&ctx);
        }
    }
}

impl eframe::App for LtlChatApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        LtlChatApp::update(self, ui);
    }
}

// -----------------------------------------------------------------
// 事件泵与数据重读
// -----------------------------------------------------------------

impl LtlChatApp {
    fn drain_events(&mut self) {
        while let Ok(evt) = self.evt_rx.try_recv() {
            match evt {
                Evt::LoginOk { user } => {
                    self.me = Some(user);
                    self.login_busy = false;
                    self.oauth_waiting = false;
                    self.login_error = None;
                    self.active_conv = None;
                    self.messages.clear();
                    self.dirty = true;
                }
                Evt::LoginFailed(e) => {
                    self.login_busy = false;
                    self.oauth_waiting = false;
                    self.login_error = Some(e);
                }
                Evt::LoggedOut(reason) => {
                    self.me = None;
                    self.active_conv = None;
                    self.messages.clear();
                    self.convs.clear();
                    self.login_error = Some(reason);
                    self.login_busy = false;
                    self.oauth_waiting = false;
                    let _ = self.cmd.send(Cmd::LoadAuthMeta { server: self.login_server.trim().to_string() });
                }
                Evt::AuthMeta { mode, demo } => {
                    self.auth_meta = Some((mode, demo));
                }
                Evt::OAuthWaiting => {
                    self.oauth_waiting = true;
                }
                Evt::DataChanged => {
                    self.dirty = true;
                    if self.active_conv.is_some() {
                        self.messages_dirty = true;
                    }
                }
                Evt::Error(e) => self.push_toast(e),
                Evt::ImageReady { url, width, height, rgba } => {
                    self.image_pending.remove(&url);
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [width.max(1) as usize, height.max(1) as usize],
                        &rgba,
                    );
                    self.pending_textures.push((url, image));
                    self.dirty = true;
                    if self.active_conv.is_some() {
                        self.messages_dirty = true;
                    }
                }
                Evt::FileSaved { path } => {
                    self.push_toast(format!("已保存: {}", path.display()));
                }
            }
        }
    }

    /// update 开头调用:用 ctx 把待建纹理落进缓存
    fn materialize_textures(&mut self, ctx: &egui::Context) {
        for (url, image) in self.pending_textures.drain(..) {
            let handle = ctx.load_texture(format!("img:{url}"), image, egui::TextureOptions::LINEAR);
            self.textures.insert(url, handle);
        }
    }

    fn reload_convs(&mut self) {
        let Some(me) = &self.me else { return };
        self.convs = self.store.list_conversations(me.id).unwrap_or_default();
        // 建单聊后自动打开
        if let Some(uid) = self.pending_open_user {
            if let Some(conv) = self
                .convs
                .iter()
                .find(|cv| {
                    cv.conversation.conv_type == ltl_chat_core::types::ConvType::Single
                        && cv.conversation.members.iter().any(|m| m.user_id == uid)
                })
                .map(|cv| cv.conversation.id)
            {
                self.open_conv(conv);
            }
            self.pending_open_user = None;
        }
    }

    fn reload_messages(&mut self) {
        let Some(conv_id) = self.active_conv else { return };
        let before = self.messages.len();
        self.messages = self
            .store
            .list_messages(conv_id, 300, None)
            .unwrap_or_default();
        if self.messages.len() != before || self.scroll_bottom {
            self.scroll_bottom = true;
        }
    }

    pub fn open_conv(&mut self, conv_id: i64) {
        if self.active_conv == Some(conv_id) {
            return;
        }
        self.active_conv = Some(conv_id);
        self.input.clear();
        self.reply_to = None;
        self.show_emoji = false;
        self.scroll_bottom = true;
        // 本地立即标记已读(未读徽标归零), 后台再补历史
        let _ = self.store.mark_read(conv_id);
        self.dirty = true;
        let _ = self.cmd.send(Cmd::OpenConversation { conv_id });
    }

    pub fn push_toast(&mut self, text: String) {
        self.toasts.push(Toast { text, at: Instant::now() });
    }

    pub fn request_image(&mut self, url: &str) {
        if self.textures.contains_key(url) || self.image_pending.contains(url) {
            return;
        }
        self.image_pending.insert(url.to_string());
        let _ = self.cmd.send(Cmd::FetchImageData { url: url.to_string() });
    }

    fn draw_toasts(&mut self, ctx: &egui::Context) {
        self.toasts.retain(|t| t.at.elapsed() < Duration::from_secs(4));
        for (i, toast) in self.toasts.iter().enumerate() {
            let screen = ctx.input(|i| i.raw.screen_rect.unwrap_or(egui::Rect::EVERYTHING));
            let pos = egui::pos2(screen.center().x - 140.0, screen.top() + 24.0 + (i as f32 * 44.0));
            egui::Area::new(egui::Id::new(("toast", i)))
                .fixed_pos(pos)
                .show(ctx, |ui| {
                    egui::Frame::default()
                        .fill(egui::Color32::from_rgba_unmultiplied(30, 34, 40, 235))
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(&toast.text)
                                    .color(egui::Color32::WHITE)
                                    .size(13.0),
                            );
                        });
                });
        }
    }
}
