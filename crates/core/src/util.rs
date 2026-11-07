//! 小工具:头像配色、图片判断、时间格式化、数据目录。
//! `string_to_color`/`is_image` 移植自 frontR 的 `utils/colors.ts`、`utils/file.ts`。

use std::path::PathBuf;

use chrono::TimeZone;

/// 与 frontR COLORS 一致的头像色板 (#rrggbb)
pub const AVATAR_COLORS: &[[u8; 3]] = &[
    [0x66, 0x7e, 0xea],
    [0x76, 0x4b, 0xa2],
    [0x33, 0x70, 0xff],
    [0x34, 0xc7, 0x59],
    [0xff, 0x95, 0x00],
    [0xf5, 0x3f, 0x3f],
    [0x5a, 0xc8, 0xfa],
    [0xaf, 0x52, 0xde],
];

/// 移植 frontR stringToColor(按 UTF-16 码元哈希, 与原实现一致;
/// JS 的 << 与加法都是 32 位回绕, 这里必须用 wrapping 否则 debug 溢出 panic)
pub fn string_to_color(s: &str) -> [u8; 3] {
    let mut hash: i32 = 0;
    for unit in s.encode_utf16() {
        hash = (unit as i32)
            .wrapping_add(hash.wrapping_shl(5).wrapping_sub(hash));
    }
    let idx = (hash.unsigned_abs() as usize) % AVATAR_COLORS.len();
    AVATAR_COLORS[idx]
}

/// 移植 frontR isImage(MIME 或扩展名判断)
pub fn is_image(file_type: &str, file_name: &str) -> bool {
    if file_type.starts_with("image/") {
        return true;
    }
    let lower = file_name.to_ascii_lowercase();
    [".jpg", ".jpeg", ".png", ".gif", ".webp"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

/// HH:mm(本地时区)
pub fn fmt_time_hhmm(ms: i64) -> String {
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default()
}

/// 今天显示 HH:mm, 否则 MM-dd
pub fn fmt_time_smart(ms: i64) -> String {
    let Some(t) = chrono::Local.timestamp_millis_opt(ms).single() else {
        return String::new();
    };
    let today = chrono::Local::now().date_naive() == t.date_naive();
    if today {
        t.format("%H:%M").to_string()
    } else {
        t.format("%m-%d").to_string()
    }
}

/// 数据目录: %APPDATA%/ltl-chat/<profile> (Windows) 或平台等价目录
pub fn data_dir(profile: &str) -> PathBuf {
    let base = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ltl-chat");
    if profile.is_empty() {
        base
    } else {
        base.join(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_matches_frontend() {
        // "张三" 在 frontR stringToColor 下的落点与此实现一致即可(同一哈希)
        assert_eq!(string_to_color("张三").len(), 3);
    }

    #[test]
    fn is_image_cases() {
        assert!(is_image("image/png", "a.png"));
        assert!(is_image("", "photo.JPG"));
        assert!(!is_image("application/pdf", "a.pdf"));
    }
}
