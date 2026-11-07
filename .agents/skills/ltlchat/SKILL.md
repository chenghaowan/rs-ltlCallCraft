---
name: ltlchat
description: 操作/开发 ltl-chat(rs-ltlCallCraft,iroh+iroh-gossip P2P 通讯):构建运行、服务器启停、CLI 全量命令、egui 桌面端、双实例联调、协议说明。当用户要求操作 ltl-chat / 老头通讯 / rs-ltlCallCraft(起服务器、发消息、联调、改聊天功能)时使用。
---

# ltl-chat(rs-ltlCallCraft)操作手册

Rust workspace,4 个 crate:`crates/core`(类型/协议/存储/iroh 封装/YM 客户端)、`crates/server`(登记登录+gossip 留档)、`crates/gui`(egui 飞书风桌面端,主入口)、`crates/cli`。

## 构建

```bash
# 仓库根(D:\claw.forvia\rs-ltlCallCraft)
cargo build --workspace          # .cargo/config.toml 已配 rsproxy 镜像
cargo test -p ltl-chat-core
```

## 服务器

```bash
cargo run -p ltl-chat-server -- --host 127.0.0.1 --port 8901 --data-dir data
# 环境变量 RUST_LOG=debug 可看 gossip 收包;数据在 data/{server.db,server_jwt.key,server_node.key,uploads/}
```

- 登记登录对接 YM Auth(`--ym-auth-base https://yimei2000.cn/auth`,默认)。
- **本地演示**:`demo_` 前缀 token 不走外网 —— `demo_local_dev`、`demo_user_a` 等任意后缀,每人一个独立演示账号。
- 服务器是每个会话 gossip 主题的常驻节点(引导 + 历史留档 + 附件存储)。

## CLI

```bash
cargo run -p ltl-chat-cli -- login --token demo_alice
cargo run -p ltl-chat-cli -- --profile bob login --token demo_bob
cargo run -p ltl-chat-cli -- whoami | users | conversations
cargo run -p ltl-chat-cli -- create-dm --user bob
cargo run -p ltl-chat-cli -- create-group --title 项目组 --members bob,charlie
cargo run -p ltl-chat-cli -- send --conv 1 --text "hello" [--wait-secs 2]
cargo run -p ltl-chat-cli -- history --conv 1
cargo run -p ltl-chat-cli -- listen --conv 1        # 长驻收听,联调用
```

- `--profile X` 决定数据目录 `%APPDATA%\ltl-chat\X`,同机多实例联调必加。
- 配置在 `<数据目录>/config.toml`(server/token/server_endpoint_id)。

## GUI(主入口)

```bash
cargo run -p ltl-chat-gui --release             # 默认 profile
cargo run -p ltl-chat-gui -- --profile b        # 第二实例
```

登录页:服务器地址 + 手机号/用户名(YM Auth)或 Token 粘贴或"本地演示登录"(token=demo_local_dev)。
界面:左导航(消息/通讯录)→ 中栏会话列表(置顶/单人消息/群聊分组、未读徽标、右键置顶)→ 右栏聊天(气泡 #3370ff、引用、撤回右键、表情、图片≤256px 压缩、文件卡下载、建群/拉人弹窗)。

## 双实例联调(标准流程)

```bash
# 终端1
cargo run -p ltl-chat-server -- --data-dir data
# 终端2
cargo run -p ltl-chat-gui -- --profile a    # 演示登录 demo_local_dev
# 终端3
cargo run -p ltl-chat-gui -- --profile b    # 演示登录 demo_user_b
# a 在通讯录点 b 发消息 → b 实时收到;关 b, a 发消息, 重启 b 打开会话 → HTTP 历史补齐
```

## 协议要点(改代码前必读)

- **gossip wire**:`GossipEnvelope { payload, nonce: [u8;16] }`,serde_json;payload 为 `Message(..)` 或 `Recall{..}`。nonce 防 iroh 内容哈希去重(90s 窗口);单条上限 128 KiB(core/node.rs)。
- **iroh 不回显自己的 broadcast**:发送方本地直接落库;服务器节点收包落库作为历史。
- **离线补齐**:消息带 `server_seq`(服务器递增);客户端 `GET /api/v1/conversations/{id}/messages?after_seq=` 增量、`before_seq=` 向上翻页;gossip 先到的消息 uuid 幂等入库、seq 后填。
- **未读**:纯客户端计算(rowid 游标,core/store.rs read_cursors)。
- **在线**:客户端 30s 心跳,服务器窗口 120s。
- **身份**:登录响应的 `server_endpoint_id` 是客户端 gossip bootstrap;TopicId 由服务器在建会话时随机生成(hex 存储)。
- 改 wire 结构要同步 core/types.rs(唯一权威),server 与 gui/cli 都经 core。

## 数据位置

| 端 | 位置 |
|---|---|
| 服务器 | `--data-dir`(默认 ./data):server.db / uploads/ / 两个密钥 |
| 客户端 | `%APPDATA%\ltl-chat\<profile>`:client.db / node.key / config.toml |
