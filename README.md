# rs-ltlCallCraft · ltl-chat(老头通讯)

[English](README.en.md) | 简体中文

从 `ltl-manage` 拆出的独立通讯模块:服务器端**可配置认证**(YM Auth / 自部署 Casdoor / 内置账号 / 静态令牌 四态切换),聊天与群聊全部走 **iroh + iroh-gossip** P2P 网络,egui 复刻飞书界面。**ltl-manage 本体不改动。**

![Rust](https://img.shields.io/badge/Rust-1.96+-deA587?logo=rust&logoColor=white)
![iroh](https://img.shields.io/badge/iroh-1.4-blue)
![egui](https://img.shields.io/badge/egui-0.37-orange)
![平台](https://img.shields.io/badge/平台-Windows%20x64-0078d4?logo=windows)

## 架构

```
rs-ltlCallCraft/
├── crates/core     ltl-chat-core     领域类型 / gossip wire 协议 / 本地存储(rusqlite) / iroh 节点封装(N0+mDNS) / YM Auth & Casdoor 客户端
├── crates/server   ltl-chat-server   认证四态 + 常驻 iroh 节点(引导节点 + 历史留档)+ 附件存储
├── crates/gui      ltl-chat-gui      eframe/egui 0.37 桌面端(飞书风)—— 本项目主入口
└── crates/cli      ltl-chat-cli      clap 子命令(登录/名单/会话/收发/监听)
```

```
┌─────────┐   gossip 主题(每会话一个 TopicId)   ┌─────────┐
│  GUI A  │◄──────────── P2P / relay ───────────►│  GUI B  │
└────┬────┘         ▲                  ▲            └────┬────┘
     │ REST(登录/历史/附件)                 │ gossip 收包留档
     ▼                                     │
┌──────────────────────────────────────────────────────────┐
│ ltl-chat-server:认证 + JWT + SQLite 历史 + 附件            │
│ 常驻 iroh 节点 = 所有主题的 bootstrap(登录响应下发其 id)   │
└──────────────────────────────────────────────────────────┘
```

## 认证(可配置)

服务器 `config.toml [auth] mode` 四选一(模板见 [`config.example.toml`](config.example.toml)):

| mode | 登录方式 | 说明 |
|---|---|---|
| `ym`(默认) | YM Auth 手机号 / token | 兼容旧体系,缺省 endpoint `https://yimei2000.cn/auth` |
| `casdoor` | 浏览器完成登录 | 自部署 [Casdoor](https://casdoor.ai)(单二进制 + SQLite):密码 / **邮箱验证码** / 短信 / **飞书扫码**,登录页自带;chat-server 只调 OIDC userinfo,GUI"打开浏览器登录"后自动轮询取票 |
| `local` | 用户名 + 密码 | 内置账号(argon2,`ltl-chat-server hash` 生成哈希),断网内网可用 |
| `token` | 静态令牌白名单 | `token → 显示名` 映射,最简部署 |

四态最终都发本服务自己的 JWT(24h,支持 30 天内续签);`demo = false` 关闭演示后门。以后加微信扫码、PASSKEY 等,只动 Casdoor 管理台,chat-server/GUI 代码零改动。

## 快速开始

```bash
# 1. 起服务器(默认 127.0.0.1:8901;不带 config = ym 模式 + demo 开)
cargo run -p ltl-chat-server -- serve --data-dir data

# 2. 起 GUI(主入口),登录页点「本地演示登录」即可无外网联调
cargo run -p ltl-chat-gui --release

# 3. 第二个身份(多实例,--profile 隔离数据目录)
cargo run -p ltl-chat-gui --release -- --profile b
```

内置账号模式示例:

```bash
ltl-chat-server hash 你的密码            # 生成 $argon2id$…
# 写进 config.toml [auth.users],然后:
ltl-chat-server serve --config config.toml --data-dir data
```

## CLI

```bash
ltl-chat-cli login --token demo_local_dev          # 或 --user/--password(local)、--oauth(casdoor)
ltl-chat-cli users | conversations | whoami
ltl-chat-cli create-dm --user bob
ltl-chat-cli create-group --title 项目组 --members bob,charlie
ltl-chat-cli send --conv 1 --text "你好"
ltl-chat-cli history --conv 1
ltl-chat-cli listen --conv 1                       # 长驻收听,联调用
```

## 关键设计

- **实时消息**:iroh-gossip 广播(每会话一个 TopicId),服务器常驻主题内;iroh 不回显自己的 broadcast,发送方本地直接落库。
- **离线补齐**:消息 uuid 幂等去重;服务器递增 `server_seq`,客户端 `after_seq/before_seq` 增量拉历史。
- **局域网直连**:N0 presets + mDNS 地址发现,同网段不依赖公网 DNS。
- **未读/置顶**:客户端本地 SQLite 游标计算;在线状态 30s 心跳 / 120s 窗口。
- **附件**:HTTP 上传服务器(`uploads/`),消息体只带元数据;图片发送前压缩 ≤256px(对齐旧前端行为)。

## 依赖

iroh 1.4 / iroh-gossip 0.102 / eframe-egui 0.37(与 pointtofix、ai-runtime 同版)/ axum 0.8 / rusqlite 0.37 bundled / tokio 1。构建配置见 `.cargo/config.toml`(rsproxy 镜像)。

## 更多文档

- 运维/联调/协议手册:[`.agents/skills/ltlchat/SKILL.md`](.agents/skills/ltlchat/SKILL.md)
- Casdoor 部署清单(二级域名 nginx / Providers / 回调白名单)见 SKILL.md

## 本期不含(后续项)

iroh-blobs P2P 附件、端到端消息加密、已读回执/typing、白板/视频通话、消息转任务(旧耦合)。当前 gossip 明文广播,仅限主题成员可见,主题 id 由服务器按成员签发。
