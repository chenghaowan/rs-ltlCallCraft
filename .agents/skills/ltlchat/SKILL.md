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
cargo run -p ltl-chat-server -- serve --host 127.0.0.1 --port 8901 --data-dir data [--config config.toml]
# 生成内置账号哈希: ltl-chat-server hash 你的密码   → 粘贴进 config.toml [auth.users]
# 环境变量 RUST_LOG=debug 可看 gossip 收包;数据在 data/{server.db,server_jwt.key,server_node.key,uploads/}
```

**认证四态**(`config.toml [auth] mode`,模板见 `config.example.toml`):

| mode | 登录方式 | GUI 登录页 |
|---|---|---|
| `ym`(默认) | YM Auth 手机号 / token;缺省 endpoint = https://yimei2000.cn/auth | 手机号表单 + token 粘贴 |
| `casdoor` | 浏览器打开 Casdoor 授权页(密码/邮箱验证码/短信/**飞书扫码**)+ token 兜底 | "打开浏览器登录" + 轮询,2s×150 次 |
| `local` | config.toml 内置账号(用户名 + argon2) | 用户名密码表单 |
| `token` | 静态令牌白名单 → 显示名 | token 粘贴 |

- `demo = true`(缺省)时任何模式下 `demo_` 前缀 token 可直接登录(每人一个独立演示账号);生产置 false。
- `GET /api/v1/auth/meta` 无需登录返回 `{mode, demo}`,GUI/CLI 启动即拉取。
- **casdoor 模式流程**:`GET /auth/oauth-url` 返回授权页地址 + 票据 → 用户浏览器完成登录 → Casdoor 回调 `{proto}://{host}/api/v1/auth/callback`(Host 自动识别,反代需传 X-Forwarded-Proto/Host)→ 服务器用 client_secret 换 token + userinfo → 发自家 JWT → GUI 每 2s `GET /auth/poll?ticket=` 取走。
- **Casdoor 部署清单**(阿里云,单二进制 + SQLite):① 二级域名如 `auth.yimei2000.cn` nginx 反代,透传 Host/X-Forwarded-Proto,acme 证书;② `conf/app.conf` 的 `origin` 必须等于对外域名,`driverName=sqlite3` 首次启动自动建表(默认管理员 admin/123);③ 管理台建 1 个 Application(client_id/secret + 回调白名单)和 Providers(Aliyun SMS / Email SMTP 走 465 端口 / Lark 飞书);④ 登录方式在应用上勾选;RBAC 完全不用管。

## CLI

```bash
cargo run -p ltl-chat-cli -- login --token demo_alice
cargo run -p ltl-chat-cli -- login --user alice --password xxx    # 服务器 local 模式
cargo run -p ltl-chat-cli -- login --oauth                        # 服务器 casdoor 模式(开浏览器+轮询)
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
