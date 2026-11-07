//! iroh 节点封装:Endpoint(密钥持久化)+ Gossip + Router。
//!
//! 服务端与客户端共用:双方都以同一形态加入 gossip 主题,
//! 服务器只是"永远在线的那个 peer"(引导节点 + 历史留档)。

use std::path::{Path, PathBuf};

use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointId, SecretKey};
use iroh_gossip::api::GossipTopic;
use iroh_gossip::net::Gossip;

use crate::error::{LtlError, Result};
use crate::types::topic_id_from_hex;

/// gossip 单条消息上限(文本 + 附件元数据 JSON 足够;字节本体走 HTTP)
const MAX_GOSSIP_MESSAGE: usize = 128 * 1024;

pub struct ChatNode {
    endpoint: Endpoint,
    gossip: Gossip,
    // Router 需要持有到进程结束(负责把入站连接路由给 gossip)
    _router: Router,
}

impl ChatNode {
    /// 启动节点。`key_path` 指向密钥文件(不存在则生成并写入)。
    pub async fn spawn(key_path: &Path) -> Result<Self> {
        let secret_key = load_or_create_secret(key_path)?;
        let endpoint = Endpoint::builder(presets::N0)
            .secret_key(secret_key)
            .bind()
            .await
            .map_err(|e| LtlError::Other(format!("iroh Endpoint 绑定失败: {e}")))?;

        // 局域网 mDNS 发现:同机/同网段互发现不依赖 n0 公网 DNS
        match iroh_mdns_address_lookup::MdnsAddressLookup::builder().build(endpoint.id()) {
            Ok(mdns) => match endpoint.address_lookup() {
                Ok(lookup) => lookup.add(mdns),
                Err(e) => tracing::warn!("address_lookup 不可用: {e}"),
            },
            Err(e) => tracing::warn!("mDNS 发现不可用: {e}"),
        }
        let gossip = Gossip::builder()
            .max_message_size(MAX_GOSSIP_MESSAGE)
            .spawn(endpoint.clone());
        let router = Router::builder(endpoint.clone())
            .accept(iroh_gossip::ALPN, gossip.clone())
            .spawn();
        Ok(Self { endpoint, gossip, _router: router })
    }

    /// 本机节点身份(gossip 对端用它拨号)
    pub fn endpoint_id(&self) -> EndpointId {
        self.endpoint.id()
    }

    /// 加入一个主题(subscribe 不等待;首条广播前请先 `topic.joined()`)。
    /// `bootstrap` 一般是服务器节点;服务器自己 bootstrap 传 None。
    pub async fn join_topic(
        &self,
        topic_hex: &str,
        bootstrap: Option<EndpointId>,
    ) -> Result<GossipTopic> {
        let topic = topic_id_from_hex(topic_hex)?;
        let boot: Vec<EndpointId> = bootstrap.into_iter().collect();
        let joined = self
            .gossip
            .subscribe(topic, boot)
            .await
            .map_err(|e| LtlError::Iroh(format!("订阅 topic 失败: {e}")))?;
        Ok(joined)
    }
}

fn load_or_create_secret(key_path: &Path) -> Result<SecretKey> {
    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if key_path.exists() {
        let bytes = std::fs::read(key_path)?;
        if bytes.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            return Ok(SecretKey::from_bytes(&arr));
        }
        tracing::warn!("密钥文件长度异常, 重新生成: {}", key_path.display());
    }
    let secret = SecretKey::generate();
    std::fs::write(key_path, secret.to_bytes())?;
    Ok(secret)
}

/// 服务器节点密钥文件名
pub fn server_key_file(data_dir: &Path) -> PathBuf {
    data_dir.join("server_node.key")
}

/// 客户端节点密钥文件名
pub fn client_key_file(data_dir: &Path) -> PathBuf {
    data_dir.join("node.key")
}
