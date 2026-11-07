# rs-ltlCallCraft · ltl-chat

[English](README.en.md) | 简体中文

A standalone communication module extracted from `ltl-manage`: the server supports **configurable authentication** (YM Auth / self-hosted Casdoor / built-in accounts / static tokens), while chat and group messaging run entirely over **iroh + iroh-gossip** P2P networking, with a Feishu-style desktop UI built on egui. The `ltl-manage` codebase itself is left untouched.

![Rust](https://img.shields.io/badge/Rust-1.96+-deA587?logo=rust&logoColor=white)
![iroh](https://img.shields.io/badge/iroh-1.4-blue)
![egui](https://img.shields.io/badge/egui-0.37-orange)
![Platform](https://img.shields.io/badge/Platform-Windows%20x64-0078d4?logo=windows)

## Architecture

```
rs-ltlCallCraft/
├── crates/core     ltl-chat-core     Domain types / gossip wire protocol / local storage (rusqlite) / iroh node wrapper (N0 + mDNS) / YM Auth & Casdoor clients
├── crates/server   ltl-chat-server   Four auth modes + always-on iroh node (bootstrap + history archive) + attachment storage
├── crates/gui      ltl-chat-gui      eframe/egui 0.37 desktop client (Feishu-style) — the primary entry point
└── crates/cli      ltl-chat-cli      clap subcommands (login / roster / conversations / send / listen)
```

```
┌─────────┐   gossip topics (one TopicId per conversation)   ┌─────────┐
│  GUI A  │◄─────────────── P2P / relay ────────────────────►│  GUI B  │
└────┬────┘             ▲                    ▲                └────┬────┘
     │ REST (auth / history / attachments)     │ gossip archive
     ▼                                         │
┌──────────────────────────────────────────────────────────┐
│ ltl-chat-server: auth + JWT + SQLite history + attachments│
│ Always-on iroh node = bootstrap for every topic (its id is │
│ delivered in the login response)                           │
└──────────────────────────────────────────────────────────┘
```

## Authentication (configurable)

Pick one of four modes via `config.toml [auth] mode` (template: [`config.example.toml`](config.example.toml)):

| mode | How users sign in | Notes |
|---|---|---|
| `ym` (default) | YM Auth phone number / token | Legacy compatibility; default endpoint `https://yimei2000.cn/auth` |
| `casdoor` | Login completed in the browser | Self-hosted [Casdoor](https://casdoor.ai) (single binary + SQLite) offers password / **email OTP** / SMS / **Feishu QR scan** out of the box; chat-server only calls OIDC userinfo, and the GUI opens the browser then polls for the result |
| `local` | Username + password | Built-in accounts (argon2, generated via `ltl-chat-server hash`), works fully offline on LAN |
| `token` | Static token whitelist | Simplest possible `token → display name` mapping |

All modes issue the service's own JWT (24h, renewable within 30 days); set `demo = false` to disable the demo backdoor. Adding WeChat QR, passkeys, etc. later only requires changes in the Casdoor console — zero code changes in chat-server/GUI.

## Quick Start

```bash
# 1. Start the server (default 127.0.0.1:8901; without a config file = ym mode + demo enabled)
cargo run -p ltl-chat-server -- serve --data-dir data

# 2. Start the GUI (primary entry point); click "Local demo login" to test without any external service
cargo run -p ltl-chat-gui --release

# 3. A second identity (multi-instance; --profile isolates the data directory)
cargo run -p ltl-chat-gui --release -- --profile b
```

Built-in account mode example:

```bash
ltl-chat-server hash your-password      # prints a $argon2id$… hash
# put it into config.toml [auth.users], then:
ltl-chat-server serve --config config.toml --data-dir data
```

## CLI

```bash
ltl-chat-cli login --token demo_local_dev       # or --user/--password (local), --oauth (casdoor)
ltl-chat-cli users | conversations | whoami
ltl-chat-cli create-dm --user bob
ltl-chat-cli create-group --title Team --members bob,charlie
ltl-chat-cli send --conv 1 --text "hello"
ltl-chat-cli history --conv 1
ltl-chat-cli listen --conv 1                    # long-running listener, great for testing
```

## Design Notes

- **Real-time messaging**: iroh-gossip broadcast (one TopicId per conversation); the server stays inside every topic. iroh does not echo your own broadcast back, so the sender persists messages locally.
- **Offline catch-up**: messages deduplicate by uuid; the server assigns a monotonic `server_seq` and clients pull history incrementally via `after_seq`/`before_seq`.
- **LAN direct connections**: N0 presets + mDNS address lookup, so peers on the same network segment need no public DNS.
- **Unread / pinning**: computed locally from SQLite cursors; presence via 30s heartbeat within a 120s window.
- **Attachments**: uploaded to the server over HTTP (`uploads/`); messages only carry metadata. Images are compressed to ≤256px before sending (matching the legacy frontend).

## Dependencies

iroh 1.4 / iroh-gossip 0.102 / eframe-egui 0.37 (same versions as pointtofix and ai-runtime) / axum 0.8 / rusqlite 0.37 bundled / tokio 1. Build configuration in `.cargo/config.toml` (rsproxy mirror).

## More Documentation

- Operations / integration / protocol handbook: [`.agents/skills/ltlchat/SKILL.md`](.agents/skills/ltlchat/SKILL.md) (Chinese)
- The Casdoor deployment checklist (subdomain nginx / providers / callback whitelist) lives in SKILL.md

## Not in this release (future work)

iroh-blobs P2P attachments, end-to-end message encryption, read receipts / typing indicators, whiteboard / video calls, message-to-task (legacy coupling). Gossip broadcasts are currently plaintext but visible only to topic members; topic ids are issued by the server per membership.
