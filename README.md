# rs-ltlCallCraft · ltl-chat(老头通讯)

从 `ltl-manage` 拆出的独立通讯模块:服务器端对接 **YM Auth** 登记登录,聊天与群聊全部走 **iroh + iroh-gossip** P2P 网络,egui 复刻飞书界面。**ltl-manage 本体不改动。**

```
rs-ltlCallCraft/
├── crates/core     ltl-chat-core     领域类型 / gossip wire 协议 / 本地存储(rusqlite) / iroh 节点封装 / YM Auth 客户端
├── crates/server   ltl-chat-server   axum 登记登录 + 常驻 iroh 节点(引导节点 + 历史留档)+ 附件存储
├── crates/gui      ltl-chat-gui      eframe/egui 0.37 桌面端(飞书风)—— 本项目主入口
└── crates/cli      ltl-chat-cli      clap 子命令(登录/名单/会话/收发/监听)
```

## 快速开始

```bash
# 1. 起服务器(默认 127.0.0.1:8901)
cargo run -p ltl-chat-server -- --data-dir data

# 2. 起 GUI(主入口)
cargo run -p ltl-chat-gui --release
#    登录页点「本地演示登录」(demo_local_dev)即可无外网联调

# 3. 第二个身份(多实例)
cargo run -p ltl-chat-gui -- --profile b
```

有 YM Auth(yimei2000.cn)可达时用手机号登录,首次登录自动登记用户并同步全员名单。

## 架构

```
┌─────────┐   gossip 主题(每会话一个 TopicId)   ┌─────────┐
│  GUI A  │◄──────────── P2P / relay ───────────►│  GUI B  │
└────┬────┘         ▲                  ▲            └────┬────┘
     │ REST(登录/历史/附件)                 │ gossip 收包留档
     ▼                                     │
┌──────────────────────────────────────────────────────────┐
│ ltl-chat-server:YM Auth 校验 + JWT + SQLite 历史 + 附件    │
│ 常驻 iroh 节点 = 所有主题的 bootstrap(登录响应下发其 id)   │
└──────────────────────────────────────────────────────────┘
```

- **实时消息**:iroh-gossip 广播(epidemic broadcast trees),服务器常驻主题内,离线成员靠 HTTP `after_seq/before_seq` 增量补齐;消息 uuid 幂等去重。
- **在线状态**:30s 心跳,窗口 120s。
- **附件**:HTTP 上传到服务器(`uploads/`),消息体只带元数据;图片发送前压缩到 ≤256px(对齐旧前端行为)。
- **未读/置顶**:客户端本地 SQLite 游标计算,不依赖服务器。

## 依赖版本

iroh 1.4 / iroh-gossip 0.102 / eframe-egui 0.37(与 pointtofix、ai-runtime 同版)/ axum 0.8 / rusqlite 0.37 bundled / tokio 1。构建配置见 `.cargo/config.toml`(rsproxy 镜像)。

## CLI

```bash
cargo run -p ltl-chat-cli -- login --token demo_local_dev
cargo run -p ltl-chat-cli -- users | conversations | history --conv 1
cargo run -p ltl-chat-cli -- send --conv 1 --text "你好"
cargo run -p ltl-chat-cli -- listen --conv 1
```

多实例加 `--profile b`(隔离数据目录)。详见 `.agents/skills/ltlchat/SKILL.md`。

## 本期不含(后续项)

iroh-blobs P2P 附件、端到端消息加密、已读回执/typing、白板/视频通话、消息转任务(旧耦合)、服务器部署脚本。当前 gossip 明文广播,仅限主题成员可见,主题 id 由服务器按成员签发。
