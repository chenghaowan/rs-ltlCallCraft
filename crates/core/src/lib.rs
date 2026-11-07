//! ltl-chat 共享核心。
//!
//! 模块划分:
//! - [`types`]:领域类型 + gossip wire 协议
//! - [`node`]:iroh Endpoint/Gossip/Router 封装(服务器与客户端共用)
//! - [`store`]:客户端本地存储(rusqlite)
//! - [`api`]:聊天服务器 REST 客户端
//! - [`ym`]:YM Auth 客户端(服务器侧)
//! - [`util`]:配色/图片判断/时间/目录工具

pub mod api;
pub mod casdoor;
pub mod error;
pub mod node;
pub mod store;
pub mod types;
pub mod util;
pub mod ym;

// 统一版本出口:上层 crate(server/gui/cli)直接经此引用, 避免各处版本漂移
pub use futures_lite;
pub use iroh;
pub use iroh_gossip;
