//! UI 常量与绘制辅助:飞书配色、表情表、头像、气泡、中文字体。

use egui::{Color32, FontData, FontDefinitions, Pos2, Rect, Response, Sense, Ui, Vec2};

/// 飞书主题蓝
pub const FEISHU_BLUE: Color32 = Color32::from_rgb(0x33, 0x70, 0xff);
pub const FEISHU_BLUE_HOVER: Color32 = Color32::from_rgb(0x2b, 0x5f, 0xe0);
pub const BG_PAGE: Color32 = Color32::from_rgb(0xf5, 0xf6, 0xf7);
pub const BG_LIST: Color32 = Color32::from_rgb(0xf7, 0xf8, 0xfa);
pub const BG_BUBBLE_OTHER: Color32 = Color32::from_rgb(0xf2, 0xf3, 0xf5);
pub const TEXT_MAIN: Color32 = Color32::from_rgb(0x1f, 0x23, 0x28);
pub const TEXT_SUB: Color32 = Color32::from_rgb(0x8f, 0x95, 0x9e);
pub const UNREAD_RED: Color32 = Color32::from_rgb(0xf5, 0x3f, 0x3f);
pub const ONLINE_GREEN: Color32 = Color32::from_rgb(0x34, 0xc7, 0x59);

/// 移植 frontR utils/constants.ts EMOJIS
pub const EMOJIS: &[&str] = &[
    "😀", "😃", "😄", "😁", "😆", "😂", "🤣", "😊", "😇", "🙂", "😍", "🤩", "😘", "😗", "😋", "😛",
    "😜", "🤪", "😝", "🤑", "🤗", "🤭", "🤫", "🤔", "😏", "😒", "😌", "😔", "😪", "😴", "😷",
    "🤒", "🤕", "🤢", "🤮", "🥴", "😵", "🤯", "🥳", "😎", "🧐", "😕", "😟", "🙁", "😮", "😯",
    "😲", "😳", "🥺", "😢", "😭", "😱", "😤", "😡", "🤬", "👍", "👎", "👊", "✊", "🤛", "🤜",
    "👏", "🙌", "👐", "🤝", "🙏", "💪", "✍️", "💋", "👄", "👅", "👀", "🧠", "🎉", "🎊", "✨",
    "🌟", "⭐", "🔥", "💯", "❤️", "💛", "💚", "💙", "💜", "🖤", "🤍", "💔", "💕", "💞", "💗",
    "💖", "✅", "❌", "☀️", "🌈", "🎵", "🎶", "🎤", "🎬", "🎨", "💎", "🎁", "🎀", "🎯", "🎲",
    "🎮",
];

/// 加载中文字体(微软雅黑)与 emoji 字体(Segoe UI Emoji), 没有则退回默认
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let candidates: [(&str, &str); 5] = [
        ("msyh", "C:\\Windows\\Fonts\\msyh.ttc"),
        ("msyhbd", "C:\\Windows\\Fonts\\msyhbd.ttc"),
        ("simhei", "C:\\Windows\\Fonts\\simhei.ttf"),
        ("dengxian", "C:\\Windows\\Fonts\\Deng.ttf"),
        ("seguiemj", "C:\\Windows\\Fonts\\seguiemj.ttf"),
    ];
    let mut cjk_installed = false;
    for (name, path) in candidates {
        if let Ok(data) = std::fs::read(path) {
            fonts
                .font_data
                .insert(name.to_string(), std::sync::Arc::new(FontData::from_owned(data)));
            if name == "seguiemj" {
                // emoji 追加到两个族尾, 避免覆盖中文字形
                fonts
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default()
                    .push(name.to_string());
                fonts
                    .families
                    .entry(egui::FontFamily::Monospace)
                    .or_default()
                    .push(name.to_string());
            } else if !cjk_installed {
                // 首个中文字体插到最前
                fonts
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default()
                    .insert(0, name.to_string());
                fonts
                    .families
                    .entry(egui::FontFamily::Monospace)
                    .or_default()
                    .insert(0, name.to_string());
                cjk_installed = true;
            }
        }
    }
    ctx.set_fonts(fonts);
}

/// 圆形首字头像(painter 版, 颜色哈希与 frontR stringToColor 一致)
pub fn paint_avatar(p: &egui::Painter, center: Pos2, size: f32, name: &str, show_online: bool) {
    let color = ltl_chat_core::util::string_to_color(name);
    let radius = size / 2.0;
    p.circle_filled(center, radius, Color32::from_rgb(color[0], color[1], color[2]));
    let ch = name
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "?".into());
    p.text(
        center,
        egui::Align2::CENTER_CENTER,
        ch,
        egui::FontId::proportional(size * 0.5),
        Color32::WHITE,
    );
    if show_online {
        let dot_r = radius * 0.28;
        let pos = Pos2::new(center.x + radius * 0.75, center.y + radius * 0.75);
        p.circle_filled(pos, dot_r + 1.5, Color32::WHITE);
        p.circle_filled(pos, dot_r, ONLINE_GREEN);
    }
}

/// 群聊头像:蓝底"群"字
pub fn paint_group_avatar(p: &egui::Painter, center: Pos2, size: f32) {
    p.rect_filled(
        Rect::from_center_size(center, Vec2::splat(size)),
        size * 0.2,
        FEISHU_BLUE,
    );
    p.text(
        center,
        egui::Align2::CENTER_CENTER,
        "群",
        egui::FontId::proportional(size * 0.5),
        Color32::WHITE,
    );
}

/// 圆角气泡 frame
pub fn bubble_frame(fill: Color32) -> egui::Frame {
    egui::Frame::default()
        .fill(fill)
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(10, 7))
}
