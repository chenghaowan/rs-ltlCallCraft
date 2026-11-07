//! ltl-chat 桌面端入口(egui, 飞书风格)—— 本项目主入口。
//!
//! ```text
//! cargo run -p ltl-chat-gui --release            # 默认 profile
//! cargo run -p ltl-chat-gui -- --profile b       # 多实例联调
//! ```

mod app;
mod assets;
mod backend;
mod views;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "ltl-chat",
    about = "老头通讯 — iroh P2P 聊天桌面端(飞书风格)"
)]
struct Cli {
    /// 数据目录分组(多实例联调用, 如 --profile b)
    #[arg(long, default_value = "")]
    profile: String,
    /// 默认服务器地址(登录页可改)
    #[arg(long, default_value = "http://127.0.0.1:8901")]
    server: String,
}

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,ltl_chat_gui=info,ltl_chat_core=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let server = cli.server.clone();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("老头通讯 — ltl-chat")
            .with_inner_size([1240.0, 820.0])
            .with_min_inner_size([960.0, 620.0]),
        ..Default::default()
    };

    eframe::run_native(
        "ltl-chat",
        options,
        Box::new(move |cc| Ok(Box::new(app::LtlChatApp::new(cc, &cli.profile, server)))),
    )
}
