//! ltl-chat 服务器入口。
//!
//! ```text
//! ltl-chat-server serve [--config config.toml] [--host ..] [--port ..] [--data-dir ..]
//! ltl-chat-server hash <密码>          # 生成 argon2 哈希, 粘贴进 [auth.users]
//! ```

mod config;
mod db;
mod jwt;
mod routes;
mod state;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

use config::FileConfig;

#[derive(Parser, Debug)]
#[command(
    name = "ltl-chat-server",
    about = "ltl-chat 服务器(YM Auth/Casdoor/local/token 四态认证 + iroh gossip 留档)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// 启动服务器
    Serve {
        /// 配置文件(缺省读 ./config.toml, 不存在则 ym + demo)
        #[arg(long)]
        config: Option<PathBuf>,
        /// 监听地址
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// 监听端口
        #[arg(long, default_value_t = 8901)]
        port: u16,
        /// 数据目录(server.db / 密钥 / uploads)
        #[arg(long, default_value = "data")]
        data_dir: PathBuf,
        /// [deprecated] YM Auth 基址(等价 [auth.ym] endpoint + mode=ym)
        #[arg(long)]
        ym_auth_base: Option<String>,
    },
    /// 生成 argon2 密码哈希(用于 config.toml [auth.users])
    Hash { password: String },
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

    match Cli::parse().cmd {
        Cmd::Hash { password } => {
            let hash = hash_password(&password)?;
            println!("{hash}");
        }
        Cmd::Serve { config, host, port, data_dir, ym_auth_base } => {
            let cfg = FileConfig::load(config.as_deref(), ym_auth_base.as_deref())?;
            serve(cfg, host, port, data_dir).await?;
        }
    }
    Ok(())
}

async fn serve(cfg: FileConfig, host: String, port: u16, data_dir: PathBuf) -> anyhow::Result<()> {
    let st = state::AppState::init(&data_dir, &cfg).await?;

    eprintln!("ltl-chat-server");
    eprintln!("  http     : http://{}:{}", host, port);
    eprintln!("  data-dir : {}", data_dir.display());
    eprintln!("  auth     : mode={} demo={}", st.auth.mode.as_str(), st.auth.demo_enabled());
    eprintln!("  iroh id  : {}", st.endpoint_id);
    if st.auth.demo_enabled() {
        eprintln!("  ⚠ 演示登录已开启(demo_* token 可直接登录), 生产请在 config.toml 设 demo = false");
    }

    let app = routes::router(st);
    let listener = tokio::net::TcpListener::bind((host.as_str(), port)).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// argon2id 默认参数, 输出标准 PHC 字符串
pub fn hash_password(password: &str) -> anyhow::Result<String> {
    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
    use argon2::Argon2;
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("argon2 哈希失败: {e}"))?;
    Ok(hash.to_string())
}
