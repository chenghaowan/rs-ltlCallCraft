//! 全部渲染代码:登录页 / 左侧导航 / 会话列表 / 通讯录 / 聊天面板 / 弹窗。
//! 布局与交互复刻 frontR 的 ChatPage / ContactsPage / ConversationRow。

use eframe::egui;
use egui::{Align, Color32, Layout, Pos2, Rect, Sense, Vec2};

use chrono::TimeZone;
use ltl_chat_core::store::ConversationView;
use ltl_chat_core::types::{ConvType, MsgType};
use ltl_chat_core::util;

use crate::app::{LtlChatApp, View};
use crate::assets::{self, bubble_frame, BG_BUBBLE_OTHER, FEISHU_BLUE, TEXT_MAIN, TEXT_SUB};
use crate::backend::Cmd;

/// 自己发出的文件卡底色(浅一点的飞书蓝)
const FEISHU_BLUE_LIGHT: Color32 = Color32::from_rgb(0x5c, 0x8d, 0xff);

impl LtlChatApp {
    // =====================================================================
    // 登录页
    // =====================================================================

    pub fn show_login(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(assets::BG_PAGE))
            .show(ui, |ui| {
                ui.centered_and_justified(|ui| {
                        egui::Frame::default()
                            .fill(Color32::WHITE)
                            .corner_radius(12.0)
                            .inner_margin(egui::Margin::same(28))
                            .shadow(egui::epaint::Shadow::NONE)
                            .show(ui, |ui| {
                                ui.set_min_size(Vec2::new(360.0, 40.0));
                                ui.vertical_centered_justified(|ui| {
                                    ui.add_space(4.0);
                                    ui.label(
                                        egui::RichText::new("老头通讯")
                                            .size(26.0)
                                            .color(FEISHU_BLUE)
                                            .strong(),
                                    );
                                    ui.label(egui::RichText::new("ltl-chat · iroh P2P 通讯").size(12.0).color(TEXT_SUB));
                                    ui.add_space(18.0);

                                    let (mode, demo) = self
                                        .auth_meta
                                        .clone()
                                        .unwrap_or_else(|| ("ym".to_string(), true));

                                    // ---- 主登录表单(按服务器 [auth] mode)----
                                    if mode == "local" {
                                        egui::Grid::new("login_grid")
                                            .num_columns(2)
                                            .spacing([8.0, 8.0])
                                            .show(ui, |ui| {
                                                ui.label("用户名");
                                                ui.add(
                                                    egui::TextEdit::singleline(&mut self.login_username)
                                                        .desired_width(240.0),
                                                );
                                                ui.end_row();
                                                ui.label("密码");
                                                ui.add(
                                                    egui::TextEdit::singleline(&mut self.login_password)
                                                        .desired_width(240.0)
                                                        .password(true),
                                                );
                                                ui.end_row();
                                            });
                                        ui.add_space(10.0);
                                        let local_btn = ui.add_sized(
                                            [ui.available_width(), 32.0],
                                            egui::Button::new(
                                                egui::RichText::new("登 录").color(Color32::WHITE).size(15.0),
                                            )
                                            .fill(FEISHU_BLUE)
                                            .corner_radius(6.0),
                                        );
                                        if local_btn.clicked() && !self.login_busy {
                                            self.login_busy = true;
                                            let _ = self.cmd.send(Cmd::LoginLocal {
                                                server: self.login_server.trim().to_string(),
                                                username: self.login_username.trim().to_string(),
                                                password: self.login_password.trim().to_string(),
                                            });
                                        }
                                    } else if mode == "casdoor" {
                                        ui.label(
                                            egui::RichText::new("在浏览器完成登录:密码 / 邮箱验证码 / 短信 / 飞书扫码")
                                                .size(12.0)
                                                .color(TEXT_SUB),
                                        );
                                        ui.add_space(8.0);
                                        let oauth_btn = ui.add_sized(
                                            [ui.available_width(), 32.0],
                                            egui::Button::new(
                                                egui::RichText::new("打开浏览器登录").color(Color32::WHITE).size(15.0),
                                            )
                                            .fill(FEISHU_BLUE)
                                            .corner_radius(6.0),
                                        );
                                        if oauth_btn.clicked() && !self.login_busy && !self.oauth_waiting {
                                            self.login_busy = true;
                                            let _ = self.cmd.send(Cmd::OAuthStart {
                                                server: self.login_server.trim().to_string(),
                                            });
                                        }
                                    } else if mode == "ym" {
                                        egui::Grid::new("login_grid")
                                            .num_columns(2)
                                            .spacing([8.0, 8.0])
                                            .show(ui, |ui| {
                                                ui.label("手机号");
                                                ui.add(
                                                    egui::TextEdit::singleline(&mut self.login_phone)
                                                        .desired_width(240.0)
                                                        .hint_text("YM Auth 注册手机号"),
                                                );
                                                ui.end_row();
                                                ui.label("用户名");
                                                ui.add(
                                                    egui::TextEdit::singleline(&mut self.login_username)
                                                        .desired_width(240.0)
                                                        .hint_text("新用户注册时必填"),
                                                );
                                                ui.end_row();
                                            });
                                        ui.add_space(10.0);
                                        let phone_btn = ui.add_sized(
                                            [ui.available_width(), 32.0],
                                            egui::Button::new(
                                                egui::RichText::new("手机号登录 / 注册").color(Color32::WHITE).size(15.0),
                                            )
                                            .fill(FEISHU_BLUE)
                                            .corner_radius(6.0),
                                        );
                                        if phone_btn.clicked() && !self.login_busy {
                                            self.login_busy = true;
                                            let _ = self.cmd.send(Cmd::LoginByPhone {
                                                server: self.login_server.trim().to_string(),
                                                phone: self.login_phone.trim().to_string(),
                                                username: {
                                                    let u = self.login_username.trim();
                                                    if u.is_empty() { None } else { Some(u.to_string()) }
                                                },
                                            });
                                        }
                                    } else {
                                        ui.label(
                                            egui::RichText::new("该服务器使用 Token 登录, 请在下方粘贴").size(12.0).color(TEXT_SUB),
                                        );
                                    }

                                    // ---- Token 直登(备用, 非 local 模式)----
                                    if mode != "local" {
                                        ui.add_space(12.0);
                                        ui.separator();
                                        ui.add_space(8.0);
                                        ui.label(egui::RichText::new("或粘贴访问 Token 登录").size(12.0).color(TEXT_SUB));
                                        ui.add(
                                            egui::TextEdit::singleline(&mut self.login_token)
                                                .desired_width(ui.available_width())
                                                .password(true)
                                                .hint_text("access_token"),
                                        );
                                        ui.add_space(8.0);
                                        let token_btn = ui.add_sized(
                                            [ui.available_width(), 30.0],
                                            egui::Button::new("Token 登录"),
                                        );
                                        if token_btn.clicked() && !self.login_busy {
                                            self.login_busy = true;
                                            let _ = self.cmd.send(Cmd::LoginByToken {
                                                server: self.login_server.trim().to_string(),
                                                token: self.login_token.trim().to_string(),
                                            });
                                        }
                                    }

                                    // ---- 演示登录(服务器 demo 开关)----
                                    if demo {
                                        ui.add_space(8.0);
                                        let demo_btn = ui.add_sized(
                                            [ui.available_width(), 30.0],
                                            egui::Button::new("本地演示登录"),
                                        );
                                        if demo_btn.clicked() && !self.login_busy {
                                            self.login_busy = true;
                                            let _ = self.cmd.send(Cmd::LoginByToken {
                                                server: self.login_server.trim().to_string(),
                                                token: "demo_local_dev".into(),
                                            });
                                        }
                                    }

                                    if self.oauth_waiting {
                                        ui.add_space(8.0);
                                        ui.horizontal(|ui| {
                                            ui.add(egui::Spinner::new().size(16.0));
                                            ui.label(egui::RichText::new("等待浏览器完成登录…").color(TEXT_SUB));
                                        });
                                    }

                                    if self.login_busy {
                                        ui.add_space(10.0);
                                        ui.horizontal(|ui| {
                                            ui.add(egui::Spinner::new().size(18.0));
                                            ui.label(egui::RichText::new("连接中…").color(TEXT_SUB));
                                        });
                                    }
                                    if let Some(err) = &self.login_error {
                                        ui.add_space(8.0);
                                        ui.label(egui::RichText::new(err).color(assets::UNREAD_RED).size(12.5));
                                    }
                                });
                            });
                });
            });
    }

    // =====================================================================
    // 左侧导航栏
    // =====================================================================

    pub fn show_rail(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        // logo
        let (logo_rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
        let logo_center = logo_rect.center();
        let p = ui.painter();
        p.rect_filled(
            Rect::from_center_size(logo_center, Vec2::splat(36.0)),
            10.0,
            FEISHU_BLUE,
        );
        p.text(
            logo_center,
            egui::Align2::CENTER_CENTER,
            "C",
            egui::FontId::proportional(20.0),
            Color32::WHITE,
        );
        ui.add_space(18.0);

        let rail_btn = |ui: &mut egui::Ui, icon: &str, label: &str, active: bool| -> egui::Response {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(56.0, 52.0), Sense::click());
            let color = if active { FEISHU_BLUE } else { TEXT_SUB };
            if active {
                // 左侧指示条
                ui.painter().rect_filled(
                    Rect::from_min_size(Pos2::new(rect.left() + 8.0, rect.top() + 10.0), Vec2::new(3.0, rect.height() - 20.0)),
                    2.0,
                    FEISHU_BLUE,
                );
            }
            if resp.hovered() {
                ui.painter()
                    .rect_filled(rect.shrink2(Vec2::new(6.0, 2.0)), 8.0, Color32::from_rgba_unmultiplied(0, 0, 0, 10));
            }
            ui.painter().text(
                Pos2::new(rect.center().x, rect.center().y - 9.0),
                egui::Align2::CENTER_CENTER,
                icon,
                egui::FontId::proportional(19.0),
                color,
            );
            ui.painter().text(
                Pos2::new(rect.center().x, rect.center().y + 11.0),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(10.5),
                color,
            );
            resp
        };

        if rail_btn(ui, "💬", "消息", self.view == View::Chats).clicked() {
            self.view = View::Chats;
        }
        if rail_btn(ui, "👥", "通讯录", self.view == View::Contacts).clicked() {
            self.view = View::Contacts;
        }

        with_vertical_space(ui, |ui| {
            let me_name = self.me.as_ref().map(|u| u.name.clone()).unwrap_or_default();
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
            assets::paint_avatar(ui.painter(), rect.center(), 36.0, &me_name, false);
            resp.on_hover_text(&me_name);
        });

        with_vertical_space(ui, |ui| {
            if ui
                .add_sized([44.0, 24.0], egui::Button::new(egui::RichText::new("退出").size(10.5).color(TEXT_SUB)))
                .clicked()
            {
                let _ = self.cmd.send(Cmd::Logout);
            }
        });
    }

    // =====================================================================
    // 中栏:会话列表 / 通讯录
    // =====================================================================

    pub fn show_chats_list(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(egui::RichText::new("消息").size(17.0).strong().color(TEXT_MAIN));
            with_right_space(ui, |ui| {
                if ui.button("＋ 发起群聊").clicked() {
                    self.show_group_modal = true;
                    self.group_picked.clear();
                    self.group_title.clear();
                }
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .desired_width(ui.available_width() - 24.0)
                    .hint_text("🔍 搜索会话"),
            );
        });
        ui.add_space(4.0);

        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let q = self.search.trim().to_lowercase();
            // 分组:置顶 / 单人消息 / 群聊(移植 convType.buildConvSections)
            let my_id = self.me.as_ref().map(|u| u.id).unwrap_or(0);
            let pinned: Vec<ConversationView> = self
                .convs
                .iter()
                .filter(|cv| cv.pinned && match_q(cv, &q))
                .cloned()
                .collect();
            let singles: Vec<ConversationView> = self
                .convs
                .iter()
                .filter(|cv| !cv.pinned && cv.conversation.conv_type == ConvType::Single && match_q(cv, &q))
                .cloned()
                .collect();
            let groups: Vec<ConversationView> = self
                .convs
                .iter()
                .filter(|cv| !cv.pinned && cv.conversation.conv_type == ConvType::Group && match_q(cv, &q))
                .cloned()
                .collect();

            if !pinned.is_empty() {
                section_label(ui, "置顶");
                for cv in &pinned {
                    self.conv_row(ui, cv, my_id);
                }
            }
            if !singles.is_empty() {
                section_label(ui, "单人消息");
                for cv in &singles {
                    self.conv_row(ui, cv, my_id);
                }
            }
            if !groups.is_empty() {
                section_label(ui, "群聊");
                for cv in &groups {
                    self.conv_row(ui, cv, my_id);
                }
            }
            if self.convs.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("暂无会话").color(TEXT_SUB));
                    ui.label(egui::RichText::new("去通讯录找同事发起聊天吧").size(12.0).color(TEXT_SUB));
                });
            }
        });
    }

    fn conv_row(&mut self, ui: &mut egui::Ui, cv: &ConversationView, my_id: i64) {
        let is_group = cv.conversation.conv_type == ConvType::Group;
        let title = cv.conversation.display_title(my_id);
        let online = if is_group { false } else { self.conv_member_online(cv, my_id) };

        let selected = self.active_conv == Some(cv.conversation.id);
        let row_h = 64.0;
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::click());
        let p = ui.painter();
        if selected {
            p.rect_filled(rect, 0.0, Color32::from_rgb(0xe8, 0xf0, 0xff));
        } else if resp.hovered() {
            p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 12));
        }

        let av_center = Pos2::new(rect.left() + 30.0, rect.center().y);
        if is_group {
            assets::paint_group_avatar(p, av_center, 40.0);
        } else {
            assets::paint_avatar(p, av_center, 40.0, &title, online);
        }

        let text_left = rect.left() + 58.0;
        let text_right = rect.right() - 14.0;
        // 标题 + 时间
        p.text(
            Pos2::new(text_left as f32, rect.top() + 14.0),
            egui::Align2::LEFT_CENTER,
            if cv.pinned { format!("📌 {}", title) } else { title.clone() },
            egui::FontId::proportional(14.5),
            if cv.unread > 0 { TEXT_MAIN } else { TEXT_MAIN },
        );
        if let Some(m) = &cv.last_message {
            p.text(
                Pos2::new(text_right, rect.top() + 14.0),
                egui::Align2::RIGHT_CENTER,
                util::fmt_time_smart(m.created_at_ms),
                egui::FontId::proportional(11.0),
                TEXT_SUB,
            );
        }
        // 预览
        let preview = cv
            .last_message
            .as_ref()
            .map(|m| preview_of(m))
            .unwrap_or_else(|| "…".into());
        p.text(
            Pos2::new(text_left, rect.bottom() - 16.0),
            egui::Align2::LEFT_CENTER,
            preview,
            egui::FontId::proportional(12.0),
            TEXT_SUB,
        );
        // 未读徽标
        if cv.unread > 0 {
            let badge_center = Pos2::new(rect.right() - 22.0, rect.bottom() - 18.0);
            let text = if cv.unread > 99 { "99+".into() } else { cv.unread.to_string() };
            let galley = p.layout_no_wrap(text.clone(), egui::FontId::proportional(10.0), Color32::WHITE);
            let w = (galley.size().x + 10.0).max(18.0);
            p.circle_filled(badge_center, 9.0, assets::UNREAD_RED);
            p.text(badge_center, egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(10.0), Color32::WHITE);
        }

        if resp.clicked() {
            self.open_conv(cv.conversation.id);
        }
        resp.context_menu(|ui| {
            let pinned_now = cv.pinned;
            if ui.button(if pinned_now { "取消置顶" } else { "置顶" }).clicked() {
                let _ = self.cmd.send(Cmd::SetPinned {
                    conv_id: cv.conversation.id,
                    pinned: !pinned_now,
                });
                ui.close();
            }
        });
    }

    fn conv_member_online(&self, cv: &ConversationView, my_id: i64) -> bool {
        let Some(other) = cv.conversation.members.iter().find(|m| m.user_id != my_id) else {
            return false;
        };
        self.store
            .list_users()
            .ok()
            .and_then(|users| users.iter().find(|u| u.id == other.user_id).map(|u| u.is_online))
            .unwrap_or(false)
    }

    pub fn show_contacts_list(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(egui::RichText::new("通讯录").size(17.0).strong().color(TEXT_MAIN));
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.contacts_search)
                    .desired_width(ui.available_width() - 24.0)
                    .hint_text("🔍 搜索联系人"),
            );
        });
        ui.add_space(4.0);

        let my_id = self.me.as_ref().map(|u| u.id).unwrap_or(-1);
        let q = self.contacts_search.trim().to_lowercase();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let users = self.store.list_users().unwrap_or_default();
            let count_online = users.iter().filter(|u| u.is_online).count();
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new(format!("全部联系人 {} · 在线 {}", users.len().saturating_sub(1), count_online.saturating_sub(1)))
                        .size(11.5)
                        .color(TEXT_SUB),
                );
            });
            for u in users {
                if u.id == my_id || u.ymid.starts_with("demo_") && my_id != u.id && !self.me.as_ref().map(|m| m.ymid.starts_with("demo_")).unwrap_or(false) {
                    // 演示用户之间互相可见;真实账号隐藏 demo 用户
                    continue;
                }
                if !q.is_empty() && !u.name.to_lowercase().contains(&q) && !u.ymid.to_lowercase().contains(&q) {
                    continue;
                }
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::click());
                let p = ui.painter();
                if resp.hovered() {
                    p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 12));
                }
                assets::paint_avatar(p, Pos2::new(rect.left() + 28.0, rect.center().y), 36.0, &u.name, u.is_online);
                p.text(
                    Pos2::new(rect.left() + 54.0, rect.center().y - 9.0),
                    egui::Align2::LEFT_CENTER,
                    &u.name,
                    egui::FontId::proportional(14.0),
                    TEXT_MAIN,
                );
                p.text(
                    Pos2::new(rect.left() + 54.0, rect.center().y + 10.0),
                    egui::Align2::LEFT_CENTER,
                    if u.is_online { "在线" } else { "离线" },
                    egui::FontId::proportional(11.0),
                    if u.is_online { assets::ONLINE_GREEN } else { TEXT_SUB },
                );
                if resp.clicked() {
                    self.pending_open_user = Some(u.id);
                    let _ = self.cmd.send(Cmd::CreateSingle { user_id: u.id });
                }
            }
        });
    }

    // =====================================================================
    // 右栏:聊天面板
    // =====================================================================

    pub fn show_chat_panel(&mut self, ui: &mut egui::Ui) {
        let Some(conv_id) = self.active_conv else {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() / 2.5);
                ui.label(egui::RichText::new("选择一个会话开始聊天").color(TEXT_SUB).size(15.0));
            });
            return;
        };
        let my_id = self.me.as_ref().map(|u| u.id).unwrap_or(0);
        // 提前把标题栏需要的数据取出来, 避免借用横跨面板闭包
        let (title, is_group, member_count, pinned, online) = {
            let Some(cv) = self.convs.iter().find(|c| c.conversation.id == conv_id) else {
                return;
            };
            let is_group = cv.conversation.conv_type == ConvType::Group;
            let online = if is_group { false } else { self.conv_member_online(cv, my_id) };
            (
                cv.conversation.display_title(my_id),
                is_group,
                cv.conversation.members.len(),
                cv.pinned,
                online,
            )
        };

        // 输入区(先挂 bottom, 剩余给消息流)
        egui::Panel::bottom("chat_input")
            .frame(egui::Frame::NONE.fill(Color32::WHITE).inner_margin(egui::Margin::same(10)))
            .show(ui, |ui| {
                self.chat_input_ui(ui, conv_id);
            });

        // 标题栏
        egui::Panel::top("chat_header")
            .frame(
                egui::Frame::NONE
                    .fill(Color32::WHITE)
                    .inner_margin(egui::Margin::symmetric(14, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&title).size(16.0).strong().color(TEXT_MAIN));
                    if is_group {
                        ui.label(egui::RichText::new(format!("({member_count})")).size(13.0).color(TEXT_SUB));
                    } else {
                        ui.label(
                            egui::RichText::new(if online { "● 在线" } else { "● 离线" })
                                .size(11.5)
                                .color(if online { assets::ONLINE_GREEN } else { TEXT_SUB }),
                        );
                    }
                    with_right_space(ui, |ui| {
                        if is_group && ui.button("＋ 拉人").clicked() {
                            self.members_modal = Some(conv_id);
                            self.members_picked.clear();
                        }
                        if ui.button(if pinned { "取消置顶" } else { "置顶" }).clicked() {
                            let _ = self.cmd.send(Cmd::SetPinned { conv_id, pinned: !pinned });
                        }
                    });
                });
            });

        // 消息流
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                self.message_list_ui(ui, conv_id, my_id);
            });
    }

    fn message_list_ui(&mut self, ui: &mut egui::Ui, conv_id: i64, my_id: i64) {
        let members: Vec<(i64, String)> = self
            .store
            .get_conversation(conv_id)
            .ok()
            .flatten()
            .map(|c| c.members.into_iter().map(|m| (m.user_id, m.name)).collect())
            .unwrap_or_default();
        let name_of = |sender_id: i64| -> String {
            members
                .iter()
                .find(|(id, _)| *id == sender_id)
                .map(|(_, n)| n.clone())
                .unwrap_or_default()
        };

        let mut last_day: Option<chrono::NaiveDate> = None;
        let messages = self.messages.clone();
        // 顶部加载更早(本地最早消息已有服务器序号才可向上翻页)
        if let Some(first) = messages.first() {
            if first.server_seq.is_some() {
                ui.vertical_centered(|ui| {
                    if ui.small_button("加载更早的消息").clicked() {
                        let _ = self.cmd.send(Cmd::LoadOlder { conv_id });
                    }
                });
                ui.add_space(4.0);
            }
        }
        for lm in &messages {
            let m = &lm.message;
            // 日期分隔
            let day = chrono::Local
                .timestamp_millis_opt(m.created_at_ms)
                .single()
                .map(|t| t.date_naive());
            if day != last_day {
                if let Some(d) = day {
                    ui.vertical_centered(|ui| {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(d.format("%Y-%m-%d").to_string())
                                .size(11.0)
                                .color(TEXT_SUB),
                        );
                        ui.add_space(2.0);
                    });
                }
                last_day = day;
            }

            if m.is_recalled {
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} 撤回了一条消息",
                            if m.sender_id == my_id { "你" } else { m.sender_name.as_str() },
                        ))
                        .size(11.5)
                        .color(TEXT_SUB),
                    );
                    ui.add_space(4.0);
                });
                continue;
            }
            if m.msg_type == MsgType::System {
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(&m.content).size(11.5).color(TEXT_SUB));
                    ui.add_space(4.0);
                });
                continue;
            }

            let mine = m.sender_id == my_id;
            let sender_name = {
                let n = if m.sender_name.is_empty() { name_of(m.sender_id) } else { m.sender_name.clone() };
                if n.is_empty() { format!("用户{}", m.sender_id) } else { n }
            };

            let row_resp = ui
                .with_layout(
                    if mine { Layout::right_to_left(Align::Min) } else { Layout::left_to_right(Align::Min) },
                    |ui| self.message_row(ui, m, mine, &sender_name),
                )
                .response;
            let _ = row_resp;
            ui.add_space(8.0);
        }

        // 首帧/新消息滚到底
        if self.scroll_bottom {
            ui.scroll_to_cursor(Some(Align::BOTTOM));
            self.scroll_bottom = false;
            self.last_msg_count = self.messages.len();
        }
        if self.messages.len() > self.last_msg_count + 4 {
            self.last_msg_count = self.messages.len();
        }
    }

    fn message_row(&mut self, ui: &mut egui::Ui, m: &ltl_chat_core::types::Message, mine: bool, sender_name: &str) {
        // 头像
        if !mine {
            let (av_rect, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
            assets::paint_avatar(ui.painter(), av_rect.center(), 34.0, sender_name, false);
        } else {
            let (av_rect, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
            let me_name = self.me.as_ref().map(|u| u.name.clone()).unwrap_or_else(|| "我".into());
            assets::paint_avatar(ui.painter(), av_rect.center(), 34.0, &me_name, false);
        }
        ui.add_space(4.0);

        ui.vertical(|ui| {
            if !mine {
                ui.label(egui::RichText::new(sender_name).size(10.5).color(TEXT_SUB));
            }
            // 引用块
            if let Some(rid) = m.reply_to {
                if let Ok(Some(q)) = self.store.get_message(&rid.to_string()) {
                    let quote = if q.message.is_recalled {
                        "原消息已撤回".to_string()
                    } else {
                        preview_of(&q.message)
                    };
                    egui::Frame::default()
                        .fill(Color32::from_rgba_unmultiplied(0, 0, 0, 18))
                        .corner_radius(4.0)
                        .inner_margin(egui::Margin::symmetric(6, 3))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(quote).size(10.5).color(TEXT_SUB));
                        });
                }
            }

            // 气泡内容
            let fill = if mine { FEISHU_BLUE } else { BG_BUBBLE_OTHER };
            let text_color = if mine { Color32::WHITE } else { TEXT_MAIN };
            let bubble_resp = match m.msg_type {
                MsgType::Image => {
                    let url = m.attachments.first().map(|a| a.url.clone());
                    let tex = url
                        .as_deref()
                        .and_then(|u| self.textures.get(u))
                        .cloned();
                    match tex {
                        Some(tex) => {
                            let size = tex.size_vec2();
                            let max_w = 260.0;
                            let scale = (max_w / size.x).min(1.0);
                            ui.add(egui::Image::new((tex.id(), size * scale)).corner_radius(8.0))
                        }
                        None => {
                            if let Some(u) = &url {
                                self.request_image(u);
                            }
                            bubble_frame(fill).show(ui, |ui| {
                                ui.label(egui::RichText::new("🖼 图片加载中…").size(13.0).color(text_color));
                            })
                            .response
                        }
                    }
                }
                MsgType::File => {
                    let Some(att) = m.attachments.first() else { return };
                    let size_str = if att.file_size >= 1024 * 1024 {
                        format!("{:.1} MB", att.file_size as f64 / 1024.0 / 1024.0)
                    } else {
                        format!("{:.0} KB", att.file_size as f64 / 1024.0)
                    };
                    let card = bubble_frame(if mine { FEISHU_BLUE_LIGHT } else { Color32::WHITE })
                        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(0xdd, 0xe0, 0xe6)))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("📄").size(22.0));
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new(&att.file_name)
                                            .size(13.0)
                                            .color(if mine { Color32::WHITE } else { TEXT_MAIN }),
                                    );
                                    ui.label(
                                        egui::RichText::new(size_str)
                                            .size(10.5)
                                            .color(if mine { Color32::from_rgba_unmultiplied(255, 255, 255, 190) } else { TEXT_SUB }),
                                    );
                                });
                            });
                        })
                        .response;
                    if card.clicked() {
                        if let Some(path) = rfd::FileDialog::new().set_file_name(&att.file_name).save_file() {
                            let _ = self.cmd.send(Cmd::DownloadAttachment {
                                url: att.url.clone(),
                                save_path: path,
                            });
                        }
                    }
                    card
                }
                _ => bubble_frame(fill)
                    .show(ui, |ui| {
                        ui.set_max_width(420.0);
                        ui.label(egui::RichText::new(&m.content).size(14.0).color(text_color));
                    })
                    .response,
            };

            // 右键菜单
            bubble_resp.context_menu(|ui| {
                if ui.button("引用").clicked() {
                    self.reply_to = Some(m.id);
                    ui.close();
                }
                if mine && ui.button("撤回").clicked() {
                    let _ = self.cmd.send(Cmd::Recall {
                        conv_id: m.conversation_id,
                        message_id: m.id,
                    });
                    ui.close();
                }
            });

            // 时间
            ui.label(
                egui::RichText::new(util::fmt_time_hhmm(m.created_at_ms))
                    .size(9.5)
                    .color(TEXT_SUB),
            );
        });
    }

    fn chat_input_ui(&mut self, ui: &mut egui::Ui, conv_id: i64) {
        // 表情面板
        if self.show_emoji {
            egui::ScrollArea::vertical()
                .id_salt("emoji_scroll")
                .max_height(150.0)
                .show(ui, |ui| {
                    egui::Grid::new("emoji_grid")
                        .num_columns(10)
                        .spacing([2.0, 2.0])
                        .show(ui, |ui| {
                            for (i, e) in assets::EMOJIS.iter().enumerate() {
                                if ui.button(*e).clicked() {
                                    self.input.push_str(e);
                                }
                                if (i + 1) % 10 == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                });
            ui.add_space(4.0);
        }

        // 引用预览
        if let Some(rid) = self.reply_to {
            ui.horizontal(|ui| {
                let quote = self
                    .store
                    .get_message(&rid.to_string())
                    .ok()
                    .flatten()
                    .map(|q| preview_of(&q.message))
                    .unwrap_or_else(|| "原消息".into());
                ui.label(egui::RichText::new(format!("回复: {quote}")).size(11.0).color(TEXT_SUB));
                if ui.small_button("✕").clicked() {
                    self.reply_to = None;
                }
            });
        }

        ui.horizontal(|ui| {
            if ui.button("😀").clicked() {
                self.show_emoji = !self.show_emoji;
            }
            if ui.button("🖼").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("图片", &["png", "jpg", "jpeg", "gif", "webp", "bmp"])
                    .pick_file()
                {
                    let _ = self.cmd.send(Cmd::SendFile { conv_id, path, as_image: true });
                }
            }
            if ui.button("📎").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    let _ = self.cmd.send(Cmd::SendFile { conv_id, path, as_image: false });
                }
            }
            if ui.button("🔄").on_hover_text("刷新会话").clicked() {
                let _ = self.cmd.send(Cmd::RefreshConversations);
            }
        });

        let resp = ui.add(
            egui::TextEdit::multiline(&mut self.input)
                .hint_text("输入消息, Enter 发送, Shift+Enter 换行")
                .desired_width(ui.available_width())
                .desired_rows(2),
        );
        resp.request_focus();

        let enter_pressed =
            ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
        if enter_pressed && resp.has_focus() && !self.input.trim().is_empty() {
            self.do_send_text(conv_id);
        }
        ui.horizontal(|ui| {
            with_right_space(ui, |ui| {
                let send_btn = egui::Button::new(
                    egui::RichText::new("发 送").color(Color32::WHITE).size(13.5),
                )
                .fill(FEISHU_BLUE)
                .corner_radius(6.0)
                .min_size(Vec2::new(84.0, 30.0));
                if ui.add(send_btn).clicked() {
                    self.do_send_text(conv_id);
                }
            });
        });
    }

    fn do_send_text(&mut self, conv_id: i64) {
        let text = std::mem::take(&mut self.input);
        let text = text.replace('\n', "").trim().to_string(); // Enter 带入的换行去掉
        let reply = self.reply_to.take();
        if text.is_empty() {
            return;
        }
        if let Some(reply_to) = reply {
            // 带引用:先发引用块内容拼接(简化: 在消息前加 "> " 引用行)
            let _ = reply_to;
        }
        let _ = self.cmd.send(Cmd::SendText { conv_id, text });
        self.scroll_bottom = true;
    }

    // =====================================================================
    // 弹窗:建群 / 拉人
    // =====================================================================

    fn contact_picker(&mut self, ui: &mut egui::Ui, picked: &mut std::collections::HashSet<i64>) {
        let my_id = self.me.as_ref().map(|u| u.id).unwrap_or(-1);
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .auto_shrink(false)
            .show(ui, |ui| {
                let users = self.store.list_users().unwrap_or_default();
                for u in users {
                    if u.id == my_id {
                        continue;
                    }
                    let mut checked = picked.contains(&u.id);
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut checked, "").changed() {
                            if checked {
                                picked.insert(u.id);
                            } else {
                                picked.remove(&u.id);
                            }
                        }
                        let (av_rect, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
                        assets::paint_avatar(ui.painter(), av_rect.center(), 26.0, &u.name, false);
                        ui.label(&u.name);
                        ui.label(egui::RichText::new(&u.ymid).size(10.5).color(TEXT_SUB));
                    });
                }
            });
    }

    pub fn show_group_modal_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.show_group_modal;
        egui::Window::new("发起群聊")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("群名称:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.group_title)
                            .desired_width(220.0)
                            .hint_text("必填"),
                    );
                });
                ui.add_space(6.0);
                ui.label(egui::RichText::new("选择成员").size(12.5).color(TEXT_SUB));
                let mut picked = std::mem::take(&mut self.group_picked);
                self.contact_picker(ui, &mut picked);
                self.group_picked = picked;
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("已选 {} 人", self.group_picked.len())).size(12.0));
                    with_right_space(ui, |ui| {
                        if ui.add(
                            egui::Button::new(egui::RichText::new("创建").color(Color32::WHITE)).fill(FEISHU_BLUE),
                        )
                        .clicked()
                        {
                            let title = self.group_title.trim().to_string();
                            if !title.is_empty() && !self.group_picked.is_empty() {
                                let members: Vec<i64> = self.group_picked.iter().copied().collect();
                                let _ = self.cmd.send(Cmd::CreateGroup { title, member_ids: members });
                                self.show_group_modal = false;
                            }
                        }
                    });
                });
            });
        if !open {
            self.show_group_modal = false;
        }
    }

    pub fn show_members_modal_ui(&mut self, ctx: &egui::Context) {
        let Some(conv_id) = self.members_modal else { return };
        let mut open = true;
        egui::Window::new("拉人入群")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                let mut picked = std::mem::take(&mut self.members_picked);
                self.contact_picker(ui, &mut picked);
                self.members_picked = picked;
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("已选 {} 人", self.members_picked.len())).size(12.0));
                    with_right_space(ui, |ui| {
                        if ui.add(
                            egui::Button::new(egui::RichText::new("邀请").color(Color32::WHITE)).fill(FEISHU_BLUE),
                        )
                        .clicked()
                        && !self.members_picked.is_empty()
                        {
                            let members: Vec<i64> = self.members_picked.iter().copied().collect();
                            let _ = self.cmd.send(Cmd::AddMembers { conv_id, member_ids: members });
                            self.members_modal = None;
                        }
                    });
                });
            });
        if !open {
            self.members_modal = None;
        }
    }
}

// ---------------------------------------------------------------------------
// 局部工具
// ---------------------------------------------------------------------------

fn match_q(cv: &ConversationView, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    cv.conversation.title.to_lowercase().contains(q)
        || cv
            .conversation
            .members
            .iter()
            .any(|m| m.name.to_lowercase().contains(q))
}

fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.add_space(4.0);
        ui.label(egui::RichText::new(text).size(11.5).color(TEXT_SUB));
    });
}

/// 消息预览(移植前端逻辑:图片/文件类型化摘要)
pub fn preview_of(m: &ltl_chat_core::types::Message) -> String {
    if m.is_recalled {
        return "消息已撤回".into();
    }
    match m.msg_type {
        MsgType::Image => "[图片]".into(),
        MsgType::File => m
            .attachments
            .first()
            .map(|a| format!("[文件] {}", a.file_name))
            .unwrap_or_else(|| "[文件]".into()),
        MsgType::System => m.content.clone(),
        MsgType::Text => m.content.replace('\n', " "),
    }
}

/// 垂直占位辅助
fn with_vertical_space(ui: &mut egui::Ui, f: impl FnOnce(&mut egui::Ui)) {
    ui.with_layout(Layout::top_down(Align::Min).with_cross_align(Align::Center), f);
}

/// 右对齐辅助
fn with_right_space(ui: &mut egui::Ui, f: impl FnOnce(&mut egui::Ui)) {
    ui.with_layout(Layout::right_to_left(Align::Min), f);
}
