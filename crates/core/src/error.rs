//! 统一错误类型(本 crate 及上层 server/cli/gui 共用)。

#[derive(Debug, thiserror::Error)]
pub enum LtlError {
    #[error("网络请求失败: {0}")]
    Http(#[from] reqwest::Error),

    #[error("数据库错误: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("iroh/gossip 错误: {0}")]
    Iroh(String),

    #[error("认证失败: {0}")]
    Auth(String),

    #[error("io 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, LtlError>;
