//! iroh + iroh-gossip API 冒烟:起节点、订阅主题、广播、收到自己的回显即退出。
//! 仅用于在 CI/本机第一次构建时校验 iroh 1.4 / iroh-gossip 0.102 的 API 形态。

use anyhow::Result;
use futures_lite::StreamExt;
use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::Endpoint;
use iroh_gossip::net::Gossip;

#[tokio::main]
async fn main() -> Result<()> {
    let secret_key = iroh::SecretKey::generate();
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret_key)
        .bind()
        .await?;
    println!("> endpoint id: {}", endpoint.id());

    let gossip = Gossip::builder().spawn(endpoint.clone());
    let _router = Router::builder(endpoint.clone())
        .accept(iroh_gossip::ALPN, gossip.clone())
        .spawn();

    let topic = iroh_gossip::proto::TopicId::from_bytes([7u8; 32]);
    let (sender, mut receiver) = gossip.subscribe_and_join(topic, vec![]).await?.split();

    sender.broadcast(b"hello".to_vec().into()).await?;
    println!("> broadcast ok");

    let mut got = false;
    for _ in 0..20 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), receiver.try_next()).await {
            Ok(Ok(Some(event))) => {
                println!("> event: {event:?}");
                if matches!(event, iroh_gossip::api::Event::Received(_)) {
                    got = true;
                    break;
                }
            }
            Ok(Ok(None)) => break,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => break,
        }
    }
    println!("> smoke {}", if got { "PASSED" } else { "no-echo" });
    Ok(())
}
