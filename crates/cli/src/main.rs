//! ltl-chat 命令行。
//!
//! ```text
//! ltl-chat-cli login --oauth        # ym/casdoor 模式: 浏览器 SSO 登录
//! ltl-chat-cli login --token demo_local_dev
//! ltl-chat-cli users / conversations / whoami
//! ltl-chat-cli create-dm --user 张三
//! ltl-chat-cli create-group --title 项目组 --members 张三,李四
//! ltl-chat-cli send --conv 3 --text "你好"
//! ltl-chat-cli history --conv 3
//! ltl-chat-cli listen --conv 3
//! ```

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context as _};
use clap::{Parser, Subcommand};
use ltl_chat_core::futures_lite::StreamExt;
use chrono::TimeZone;
use serde::{Deserialize, Serialize};

use ltl_chat_core::api::ChatApiClient;
use ltl_chat_core::node::{client_key_file, ChatNode};
use ltl_chat_core::store::ClientStore;
use ltl_chat_core::types::{ConvType, GossipEnvelope, GossipPayload, Message};

#[derive(Parser)]
#[command(name = "ltl-chat-cli", about = "ltl-chat 命令行(经 iroh gossip 收发)")]
struct Cli {
    /// 数据目录分组(与 GUI 多实例一致)
    #[arg(long, global = true, default_value = "")]
    profile: String,

    /// 聊天服务器地址(login 之外的命令也可用此覆盖配置)
    #[arg(long, global = true, default_value = "http://127.0.0.1:8901")]
    server: String,

    #[command(subcommand)]
    cmd: Cmds,
}

#[derive(Subcommand)]
enum Cmds {
    /// 登录并保存会话(按服务器模式:--user/--password | --token | --oauth)
    Login {
        #[arg(long)]
        token: Option<String>,
        /// 内置账号模式:用户名
        #[arg(long)]
        user: Option<String>,
        /// 内置账号模式:密码
        #[arg(long)]
        password: Option<String>,
        /// ym / casdoor 模式:打开浏览器授权页并轮询结果
        #[arg(long, default_value_t = false)]
        oauth: bool,
    },
    /// 当前登录用户
    Whoami,
    /// 全员名单(含在线状态)
    Users,
    /// 我的会话列表
    Conversations,
    /// 发起单聊(按 ymid 或名字匹配)
    CreateDm {
        #[arg(long)]
        user: String,
    },
    /// 建群
    CreateGroup {
        #[arg(long)]
        title: String,
        #[arg(long, value_delimiter = ',')]
        members: Vec<String>,
    },
    /// 发消息(临时起 iroh 节点走 gossip)
    Send {
        #[arg(long)]
        conv: i64,
        #[arg(long)]
        text: String,
        /// 广播后等待转发秒数
        #[arg(long, default_value_t = 2.0)]
        wait_secs: f64,
    },
    /// 拉最近历史
    History {
        #[arg(long)]
        conv: i64,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// 长驻收听一个会话(联调用,Ctrl+C 退出)
    Listen { #[arg(long)] conv: i64 },
}

#[derive(Serialize, Deserialize, Default)]
struct Config {
    server: String,
    token: String,
    server_endpoint_id: String,
    user_id: Option<i64>,
    user_name: Option<String>,
}

fn data_dir(profile: &str) -> PathBuf {
    ltl_chat_core::util::data_dir(profile)
}

fn load_config(dir: &PathBuf) -> Config {
    let path = dir.join("config.toml");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_config(dir: &PathBuf, cfg: &Config) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("config.toml"), toml::to_string_pretty(cfg)?)?;
    Ok(())
}

fn api_for(cfg: &Config, server_override: &str) -> anyhow::Result<(ChatApiClient, String)> {
    let server = if server_override != "http://127.0.0.1:8901" {
        server_override.to_string()
    } else if !cfg.server.is_empty() {
        cfg.server.clone()
    } else {
        server_override.to_string()
    };
    if cfg.token.is_empty() {
        bail!("未登录, 先执行 ltl-chat-cli login");
    }
    Ok((ChatApiClient::with_token(&server, &cfg.token), server))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let dir = data_dir(&cli.profile);
    let store = ClientStore::open(&dir.join("client.db"))?;
    let mut cfg = load_config(&dir);

    match cli.cmd {
        Cmds::Login { token, user, password, oauth } => {
            let api = ChatApiClient::new(&cli.server);
            let resp = if oauth {
                // 浏览器授权 + 轮询(ym / casdoor 模式)
                let start = api.oauth_start().await.context("发起浏览器登录失败")?;
                println!("请在浏览器中完成登录:");
                println!("  {}", start.authorization_url);
                open::that(&start.authorization_url).ok();
                print!("等待登录结果");
                let mut result: anyhow::Result<ltl_chat_core::api::TokenResponse> =
                    Err(anyhow::anyhow!("超时"));
                for _ in 0..150 {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    print!(".");
                    use std::io::Write as _;
                    let _ = std::io::stdout().flush();
                    match api.oauth_poll(&start.ticket).await {
                        Ok(Some(r)) => {
                            result = Ok(r);
                            break;
                        }
                        Ok(None) => continue,
                        Err(e) => {
                            result = Err(anyhow::anyhow!("{e}"));
                            break;
                        }
                    }
                }
                println!();
                result
            } else if let (Some(u), Some(pw)) = (user.as_deref(), password.as_deref()) {
                api.login_local(u, pw).await.map_err(anyhow::Error::from)
            } else if let Some(t) = token {
                api.login_by_token(&t).await.map_err(anyhow::Error::from)
            } else {
                Err(anyhow::anyhow!("需要 --user/--password、--token 或 --oauth"))
            }
            .context("登录失败")?;
            cfg.server = cli.server.clone();
            cfg.token = resp.access_token.clone();
            cfg.server_endpoint_id = resp.server_endpoint_id.clone();
            cfg.user_id = Some(resp.user.id);
            cfg.user_name = Some(resp.user.name.clone());
            save_config(&dir, &cfg)?;
            let _ = store.upsert_user(&resp.user);
            println!(
                "登录成功: {} (id={}, ymid={})",
                resp.user.name, resp.user.id, resp.user.ymid
            );
            println!("服务器 iroh 节点: {}", resp.server_endpoint_id);
        }
        Cmds::Whoami => {
            let (api, _) = api_for(&cfg, &cli.server)?;
            let me = api.me().await.context("获取当前用户失败(token 可能过期)")?;
            println!("{} (id={}, ymid={})", me.name, me.id, me.ymid);
        }
        Cmds::Users => {
            let (api, _) = api_for(&cfg, &cli.server)?;
            let users = api.users().await?;
            println!("{:<4} {:<20} {:<12} {}", "ID", "名字", "ymid", "在线");
            for u in users {
                println!(
                    "{:<4} {:<20} {:<12} {}",
                    u.id,
                    u.name,
                    u.ymid,
                    if u.is_online { "●" } else { "" }
                );
            }
        }
        Cmds::Conversations => {
            let (api, _) = api_for(&cfg, &cli.server)?;
            let me_id = cfg.user_id.unwrap_or(0);
            for c in api.conversations().await? {
                println!(
                    "#{:<4} [{:<6}] {}  成员:{}  topic:{}",
                    c.id,
                    match c.conv_type {
                        ConvType::Single => "单聊",
                        ConvType::Group => "群聊",
                    },
                    c.display_title(me_id),
                    c.members.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(","),
                    &c.topic_id[..12.min(c.topic_id.len())],
                );
            }
        }
        Cmds::CreateDm { user } => {
            let (api, _) = api_for(&cfg, &cli.server)?;
            let uid = resolve_user(&api, &user).await?;
            let conv = api.create_conversation(ConvType::Single, None, vec![uid]).await?;
            let _ = store.replace_conversations(std::slice::from_ref(&conv));
            println!("单聊就绪: #{} topic={}", conv.id, conv.topic_id);
        }
        Cmds::CreateGroup { title, members } => {
            let (api, _) = api_for(&cfg, &cli.server)?;
            let mut ids = Vec::new();
            for m in &members {
                ids.push(resolve_user(&api, m).await?);
            }
            let conv = api.create_conversation(ConvType::Group, Some(&title), ids).await?;
            let _ = store.replace_conversations(std::slice::from_ref(&conv));
            println!("群已建立: #{} \"{}\" topic={}", conv.id, conv.title, conv.topic_id);
        }
        Cmds::Send { conv, text, wait_secs } => {
            let (api, _) = api_for(&cfg, &cli.server)?;
            let all = api.conversations().await?;
            let c = all
                .iter()
                .find(|c| c.id == conv)
                .context("会话不存在或非成员")?
                .clone();
            let user_id = cfg.user_id.context("配置缺 user_id, 请重新登录")?;
            let user_name = cfg.user_name.clone().unwrap_or_default();
            let msg = Message::new_text(conv, user_id, &user_name, text);
            let envelope = GossipEnvelope::new(GossipPayload::Message(msg.clone()));

            let node = ChatNode::spawn(&client_key_file(&dir)).await?;
            let server_ep = cfg.server_endpoint_id.parse().ok();
            let topic = node
                .join_topic(&c.topic_id, server_ep)
                .await
                .context("加入 topic 失败")?;
            let mut topic = topic;
            // 等待至少一个邻居(服务器)
            let _ = tokio::time::timeout(Duration::from_secs(10), topic.joined()).await;
            let (sender, _rx) = topic.split();
            sender
                .broadcast(envelope.to_bytes().into())
                .await
                .context("broadcast 失败")?;
            let _ = store.insert_message(&msg, None);
            tokio::time::sleep(Duration::from_secs_f64(wait_secs.max(0.2))).await;
            println!("已发送: {}", preview(&msg));
        }
        Cmds::History { conv, limit } => {
            let (api, _) = api_for(&cfg, &cli.server)?;
            let items = api.history(conv, 0, limit).await.context("拉取历史失败")?;
            for hm in items {
                let m = &hm.message;
                println!(
                    "[seq={:<5} {}] {}: {}",
                    hm.server_seq,
                    chrono::Local
                        .timestamp_millis_opt(m.created_at_ms)
                        .single()
                        .map(|t| t.format("%m-%d %H:%M").to_string())
                        .unwrap_or_default(),
                    m.sender_name,
                    preview(m)
                );
            }
        }
        Cmds::Listen { conv } => {
            let (api, _) = api_for(&cfg, &cli.server)?;
            let all = api.conversations().await?;
            let c = all
                .iter()
                .find(|c| c.id == conv)
                .context("会话不存在或非成员")?
                .clone();
            println!("监听会话 #{} (Ctrl+C 退出)…", conv);
            let node = ChatNode::spawn(&client_key_file(&dir)).await?;
            let server_ep = cfg.server_endpoint_id.parse().ok();
            let topic = node
                .join_topic(&c.topic_id, server_ep)
                .await
                .context("加入 topic 失败")?;
            let mut topic = topic;
            let _ = tokio::time::timeout(Duration::from_secs(10), topic.joined()).await;
            let (sender, mut receiver) = topic.split();
            println!("已加入 gossip 主题");
            while let Some(item) = receiver.next().await {
                if let Ok(ltl_chat_core::iroh_gossip::api::Event::Received(m)) = item {
                    if let Ok(env) = GossipEnvelope::from_bytes(&m.content) {
                        if let GossipPayload::Message(msg) = env.payload {
                            println!(
                                "[{}] {}: {}",
                                chrono::Local
                                    .timestamp_millis_opt(msg.created_at_ms)
                                    .single()
                                    .map(|t| t.format("%H:%M:%S").to_string())
                                    .unwrap_or_default(),
                                msg.sender_name,
                                preview(&msg)
                            );
                            let _ = store.insert_message(&msg, None);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

async fn resolve_user(api: &ChatApiClient, needle: &str) -> anyhow::Result<i64> {
    let users = api.users().await?;
    users
        .iter()
        .find(|u| u.ymid == needle || u.name == needle)
        .or_else(|| {
            users
                .iter()
                .find(|u| u.name.contains(needle) || u.ymid.contains(needle))
        })
        .map(|u| u.id)
        .with_context(|| format!("找不到用户: {needle}"))
}

fn preview(m: &Message) -> String {
    use ltl_chat_core::types::MsgType;
    if m.is_recalled {
        return "(已撤回)".into();
    }
    match m.msg_type {
        MsgType::Image => "[图片]".into(),
        MsgType::File => m
            .attachments
            .first()
            .map(|a| format!("[文件] {}", a.file_name))
            .unwrap_or_else(|| "[文件]".into()),
        MsgType::System => format!("[系统] {}", m.content),
        MsgType::Text => m.content.clone(),
    }
}
