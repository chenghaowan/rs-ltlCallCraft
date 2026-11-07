//! ltl-chat 服务器入口。
//!
//! 启动:YM Auth 客户端 + SQLite + 常驻 iroh 节点(所有会话 topic 的引导/留档节点)+ axum。

mod db;
mod jwt;
mod routes;
mod state;

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "ltl-chat-server", about = "ltl-chat 服务器(YM Auth 登记登录 + iroh gossip 留档)")]
struct Cli {
    /// 监听地址
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// 监听端口
    #[arg(long, default_value_t = 8901)]
    port: u16,
    /// 数据目录(server.db / 密钥 / uploads)
    #[arg(long, default_value = "data")]
    data_dir: PathBuf,
    /// YM Auth 服务基址
    #[arg(long, default_value = "https://yimei2000.cn/auth")]
    ym_auth_base: String,
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,ltl_chat_server=debug".into()),
        )
        .init();

    let cli = Cli::parse();
    let st = state::AppState::init(&cli.data_dir, &cli.ym_auth_base).await?;

    eprintln!("ltl-chat-server");
    eprintln!("  http     : http://{}:{}", cli.host, cli.port);
    eprintln!("  data-dir : {}", cli.data_dir.display());
    eprintln!("  ym-auth  : {}", cli.ym_auth_base);
    eprintln!("  iroh id  : {}", st.endpoint_id);

    let app = routes::router(st);
    let listener = tokio::net::TcpListener::bind((cli.host.as_str(), cli.port)).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
