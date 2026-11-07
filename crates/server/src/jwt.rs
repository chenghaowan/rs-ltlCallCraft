//! JWT(HS256)签发与校验, 密钥持久化在 data-dir。
//! 语义对齐旧系统:24h 有效;refresh 忽略过期但要求签发时间在 30 天内。

use std::path::Path;

use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use ltl_chat_core::error::{LtlError, Result};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub typ: String,
    pub iat: usize,
    pub exp: usize,
}

pub struct JwtKeys {
    encoding: EncodingKey,
    decoding: DecodingKey,
}

impl JwtKeys {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        let secret = if path.exists() {
            std::fs::read_to_string(path)?.trim().to_string()
        } else {
            use rand::RngCore;
            let mut buf = [0u8; 32];
            rand::rng().fill_bytes(&mut buf);
            let s = hex::encode(buf);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, &s)?;
            s
        };
        Ok(Self {
            encoding: EncodingKey::from_secret(secret.as_bytes()),
            decoding: DecodingKey::from_secret(secret.as_bytes()),
        })
    }

    /// 签发 access token, `ttl_secs` 一般为 24h
    pub fn issue(&self, user_id: i64, ttl_secs: i64) -> Result<String> {
        let now = chrono::Utc::now().timestamp() as usize;
        let claims = Claims {
            sub: user_id.to_string(),
            typ: "access".into(),
            iat: now,
            exp: now + ttl_secs as usize,
        };
        encode(&Header::new(Algorithm::HS256), &claims, &self.encoding)
            .map_err(|e| LtlError::Other(format!("JWT 签发失败: {e}")))
    }

    fn decode_with(&self, token: &str, validate_exp: bool) -> Result<Claims> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = validate_exp;
        validation.required_spec_claims = Default::default();
        let data = decode::<Claims>(token, &self.decoding, &validation)
            .map_err(|e| LtlError::Auth(format!("token 无效: {e}")))?;
        if data.claims.typ != "access" {
            return Err(LtlError::Auth("token 类型错误".into()));
        }
        Ok(data.claims)
    }

    pub fn decode(&self, token: &str) -> Result<Claims> {
        self.decode_with(token, true)
    }

    /// refresh 场景:忽略过期, 但 iat 需在 30 天内
    pub fn decode_refreshable(&self, token: &str) -> Result<Claims> {
        let claims = self.decode_with(token, false)?;
        let now = chrono::Utc::now().timestamp() as usize;
        if now.saturating_sub(claims.iat) > 30 * 24 * 3600 {
            return Err(LtlError::Auth("token 签发超过 30 天, 需重新登录".into()));
        }
        Ok(claims)
    }
}
