use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_lite::StreamExt;

use ltl_chat_core::error::Result;
use ltl_chat_core::iroh::EndpointId;
use ltl_chat_core::node::{server_key_file, ChatNode};
use ltl_chat_core::types::{GossipEnvelope, GossipPayload};
use ltl_chat_core::ym::YmAuthClient;

use crate::db::Db;
use crate::jwt::JwtKeys;

/// 在线判定窗口(旧系统 600s, 这里心跳 30s 收紧到 120s)
pub const ONLINE_WINDOW_MS: i64 = 120_000;

pub struct AppState {
    pub db: Db,
    pub node: ChatNode,
    pub endpoint_id: EndpointId,
    pub ym: YmAuthClient,
    pub jwt: JwtKeys,
    pub uploads_dir: PathBuf,
    /// roster 同步节流(至少间隔 60s)
    pub last_sync: Mutex<Option<Instant>>,
}

impl AppState {
    pub async fn init(data_dir: &Path, ym_auth_base: &str) -> Result<Arc<Self>> {
        std::fs::create_dir_all(data_dir)?;
        let uploads_dir = data_dir.join("uploads");
        std::fs::create_dir_all(&uploads_dir)?;

        let db = Db::open(&data_dir.join("server.db"))?;
        let node = ChatNode::spawn(&server_key_file(data_dir)).await?;
        let endpoint_id = node.endpoint_id();
        let jwt = JwtKeys::load_or_create(&data_dir.join("server_jwt.key"))?;
        let ym = YmAuthClient::new(ym_auth_base);

        let st = Arc::new(Self {
            db,
            node,
            endpoint_id,
            ym,
            jwt,
            uploads_dir,
            last_sync: Mutex::new(None),
        });

        // 启动时对每个已存在会话 topic 建立持久化订阅
        for topic in st.db.all_topic_ids()? {
            spawn_persist_loop(st.clone(), topic);
        }
        Ok(st)
    }

    /// 新会话建好后由路由层调用
    pub fn watch_topic(self: &Arc<Self>, topic_hex: String) {
        spawn_persist_loop(self.clone(), topic_hex);
    }
}

/// 常驻订阅循环:收 gossip → 落库(历史留档)。断流后 5s 重试。
fn spawn_persist_loop(st: Arc<AppState>, topic_hex: String) {
    tokio::spawn(async move {
        loop {
            match st.node.join_topic(&topic_hex, None).await {
                Ok(topic) => {
                    let (_sender, mut receiver) = topic.split();
                    while let Some(item) = receiver.next().await {
                        match item {
                            Ok(ltl_chat_core::iroh_gossip::api::Event::Received(msg)) => {
                                handle_envelope(&st, &msg.content);
                            }
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!("topic {topic_hex} 事件流错误: {e}");
                                break;
                            }
                        }
                    }
                    tracing::warn!("topic {topic_hex} 事件流结束, 5s 后重新订阅");
                }
                Err(e) => {
                    tracing::error!("加入 topic {topic_hex} 失败: {e}");
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

fn handle_envelope(st: &AppState, content: &[u8]) {
    let Ok(envelope) = GossipEnvelope::from_bytes(content) else {
        tracing::debug!("忽略无法解析的 gossip 载荷");
        return;
    };
    match envelope.payload {
        GossipPayload::Message(m) => {
            match st.db.insert_message(&m) {
                Ok(_seq) => tracing::debug!(
                    "留档消息 conv={} sender={} type={}",
                    m.conversation_id,
                    m.sender_id,
                    m.msg_type.as_str()
                ),
                Err(e) => tracing::error!("消息落库失败: {e}"),
            }
        }
        GossipPayload::Recall { conversation_id, message_id, .. } => {
            if let Err(e) = st.db.mark_recalled(conversation_id, &message_id.to_string()) {
                tracing::error!("撤回落库失败: {e}");
            }
        }
    }
}
