//! 服务器配置(config.toml)。
//!
//! ```toml
//! [auth]
//! mode = "casdoor"      # ym(默认)| casdoor | local | token
//! demo = false          # 允许 demo_ 前缀演示 token(缺省 true, 生产建议关)
//!
//! [auth.casdoor]        # mode = casdoor
//! endpoint       = "https://auth.yimei2000.cn"
//! client_id      = "ltl-chat"
//! client_secret  = "…"
//!
//! [auth.ym]             # mode = ym(兼容旧 YM Auth)
//! endpoint = "https://yimei2000.cn/auth"
//!
//! [auth.users]          # mode = local(内置账号, 哈希用 `ltl-chat-server hash <密码>` 生成)
//! alice = "$argon2id$…"
//!
//! [auth.tokens]         # mode = token(静态令牌 → 显示名)
//! "abc123" = "张三"
//! ```

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use ltl_chat_core::error::{LtlError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    #[default]
    Ym,
    Casdoor,
    Local,
    Token,
}

impl AuthMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthMode::Ym => "ym",
            AuthMode::Casdoor => "casdoor",
            AuthMode::Local => "local",
            AuthMode::Token => "token",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct CasdoorSection {
    pub endpoint: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct YmSection {
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AuthSection {
    pub mode: AuthMode,
    /// 缺省 true(本地联调友好);生产配置建议显式写 false
    pub demo: Option<bool>,
    pub casdoor: CasdoorSection,
    pub ym: YmSection,
    pub users: HashMap<String, String>,
    pub tokens: HashMap<String, String>,
}

impl Default for AuthSection {
    fn default() -> Self {
        Self {
            mode: AuthMode::Ym,
            demo: None,
            casdoor: CasdoorSection::default(),
            ym: YmSection::default(),
            users: HashMap::new(),
            tokens: HashMap::new(),
        }
    }
}

impl AuthSection {
    pub fn demo_enabled(&self) -> bool {
        self.demo.unwrap_or(true)
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct FileConfig {
    pub auth: AuthSection,
}

impl FileConfig {
    /// `--config` 指定则必读;否则若 ./config.toml 存在则读取,不存在用默认(ym + demo)。
    /// `ym_endpoint_fallback` 来自旧 CLI 参数 --ym-auth-base。
    pub fn load(path: Option<&Path>, ym_endpoint_fallback: Option<&str>) -> Result<Self> {
        let cfg: FileConfig = match path {
            Some(p) => {
                let text = std::fs::read_to_string(p).map_err(|e| {
                    LtlError::Other(format!("读取配置 {} 失败: {e}", p.display()))
                })?;
                toml::from_str(&text)
                    .map_err(|e| LtlError::Other(format!("解析配置 {} 失败: {e}", p.display())))?
            }
            None => {
                let default_path = Path::new("config.toml");
                if default_path.exists() {
                    let text = std::fs::read_to_string(default_path).map_err(|e| {
                        LtlError::Other(format!("读取配置 config.toml 失败: {e}"))
                    })?;
                    toml::from_str(&text).map_err(|e| {
                        LtlError::Other(format!("解析配置 config.toml 失败: {e}"))
                    })?
                } else {
                    FileConfig::default()
                }
            }
        };
        let mut cfg = cfg;
        if let Some(ep) = ym_endpoint_fallback {
            if cfg.auth.ym.endpoint.is_none() {
                cfg.auth.ym.endpoint = Some(ep.to_string());
            }
        }
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        let a = &self.auth;
        match a.mode {
            AuthMode::Ym => {
                // 兼容旧行为:未配置时默认指向 yimei2000.cn 的 YM Auth
                if a.ym.endpoint.is_none() {
                    tracing::info!("未配置 [auth.ym] endpoint, 使用默认 https://yimei2000.cn/auth");
                }
            }
            AuthMode::Casdoor => {
                if a.casdoor.endpoint.is_none() || a.casdoor.client_id.is_none() {
                    return Err(LtlError::Other(
                        "auth.mode = casdoor 需要配置 [auth.casdoor] endpoint 与 client_id".into(),
                    ));
                }
            }
            AuthMode::Local => {
                if a.users.is_empty() {
                    tracing::warn!("auth.mode = local 但 [auth.users] 为空, 将无人能登录");
                }
            }
            AuthMode::Token => {
                if a.tokens.is_empty() {
                    tracing::warn!("auth.mode = token 但 [auth.tokens] 为空, 将无人能登录");
                }
            }
        }
        Ok(())
    }

    /// 对外提供 ym endpoint(含默认兜底)
    pub fn ym_endpoint(&self) -> &str {
        self.auth
            .ym
            .endpoint
            .as_deref()
            .unwrap_or("https://yimei2000.cn/auth")
    }
}
