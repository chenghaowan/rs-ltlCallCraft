//! Casdoor 客户端 —— 服务器侧专用。
//!
//! 只用两个 Casdoor 接口:
//! - `GET  {endpoint}/api/userinfo`(Bearer access_token,OIDC 标准声明)
//! - `POST {endpoint}/api/login/oauth/access_token`(授权码换 token,表单)
//!
//! 登录方式(密码/邮箱验证码/短信/飞书扫码)全部发生在 Casdoor 自己的登录页,
//! 本 crate 不关心,chat-server 只认 userinfo 结果。

use serde::Deserialize;

/// 与 YM 客户端共用同一用户信息形状
pub type CasdoorUserInfo = crate::ym::YmUserInfo;

#[derive(Debug, Deserialize, Default)]
struct CasdoorUserinfoResponse {
    #[serde(default)]
    sub: Option<String>,
    #[serde(default)]
    preferred_username: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default, rename = "picture")]
    picture: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CasdoorTokenResponse {
    #[serde(default)]
    access_token: Option<String>,
}

pub struct CasdoorClient {
    http: reqwest::Client,
    endpoint: String,
    #[allow(dead_code)]
    client_id: String,
    #[allow(dead_code)]
    client_secret: String,
}

impl CasdoorClient {
    pub fn new(endpoint: &str, client_id: &str, client_secret: &str) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("reqwest client"),
            endpoint: endpoint.trim_end_matches('/').to_string(),
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
        }
    }

    /// 构造授权页地址(用户在浏览器里完成登录:密码/邮箱验证码/短信/扫码)
    pub fn authorize_url(&self, redirect_uri: &str, state: &str) -> String {
        use reqwest::Url;
        let mut url = Url::parse(&format!("{}/login/oauth/authorize", self.endpoint))
            .expect("casdoor endpoint 非法");
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("response_type", "code")
            .append_pair("scope", "read")
            .append_pair("state", state)
            .append_pair("redirect_uri", redirect_uri);
        url.into()
    }

    /// 本服务的 OAuth 回调地址(基于请求的 Host/Proto, 兼容本机与反代)
    pub fn callback_uri(proto: &str, host: &str) -> String {
        format!("{proto}://{host}/api/v1/auth/callback")
    }

    /// 授权码换 access_token 并拉 userinfo
    pub async fn exchange_code(
        &self,
        code: &str,
        redirect_uri: &str,
    ) -> Result<(String, CasdoorUserInfo), String> {
        let url = format!("{}/api/login/oauth/access_token", self.endpoint);
        let resp = self
            .http
            .post(&url)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("code", code),
                ("redirect_uri", redirect_uri),
            ])
            .send()
            .await
            .map_err(|e| format!("Casdoor 不可达: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("Casdoor 换 token 失败: HTTP {}", resp.status()));
        }
        let data: CasdoorTokenResponse = resp.json().await.map_err(|e| format!("Casdoor 返回异常: {e}"))?;
        let access_token = data.access_token.ok_or("Casdoor 未返回 access_token")?;
        let info = self
            .validate_token(&access_token)
            .await
            .ok_or_else(|| "Casdoor userinfo 校验失败".to_string())?;
        Ok((access_token, info))
    }

    /// 校验 access_token;失败(网络/无效)返回 None
    pub async fn validate_token(&self, token: &str) -> Option<CasdoorUserInfo> {
        let url = format!("{}/api/userinfo", self.endpoint);
        let resp = self.http.get(&url).bearer_auth(token).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let data: CasdoorUserinfoResponse = resp.json().await.ok()?;
        let ymid = data
            .preferred_username
            .or_else(|| {
                data.sub
                    .as_deref()
                    .and_then(|s| s.rsplit('/').next())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();
        if ymid.is_empty() {
            return None;
        }
        Some(CasdoorUserInfo {
            name: data.name.clone().unwrap_or_else(|| ymid.clone()),
            ymid,
            email: data.email,
            avatar: data.picture,
        })
    }
}
