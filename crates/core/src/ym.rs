//! YM Auth(yimei2000.cn)客户端 —— 服务器侧专用。
//!
//! ym_auth 已改造为 Rauthy OIDC SSO(本地不再有手机号登录):
//! - `login_url`:      GET {base}/api/v1/auth/oidc/login?redirect_to=... → {authorization_url}
//!                     (浏览器完成 Rauthy 登录后, ym_auth 中转页把 token 拼在 redirect_to 上带回)
//! - `validate_token`: GET {base}/api/v1/auth/me, Bearer token
//! - `users_list`:     GET {base}/api/v1/users/list, Bearer token
//!
//! `demo_` 前缀 token 的本地演示分支在 server 里实现, 不经过这里。

use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct YmUserInfo {
    pub ymid: String,
    pub name: String,
    pub email: Option<String>,
    pub avatar: Option<String>,
}

#[derive(Debug, Clone)]
pub struct YmListUser {
    pub ymid: String,
    pub name: String,
    pub phone: Option<String>,
    pub email: Option<String>,
}

#[derive(Deserialize)]
struct YmMeResponse {
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    id: Option<serde_json::Value>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct YmLoginUrlResponse {
    #[serde(default)]
    authorization_url: Option<String>,
}

#[derive(Deserialize)]
struct YmUsersListResponse {
    #[serde(default)]
    items: Vec<YmUsersListItem>,
}

#[derive(Deserialize)]
struct YmUsersListItem {
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    phone: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

pub struct YmAuthClient {
    http: reqwest::Client,
    base: String,
}

impl YmAuthClient {
    pub fn new(base: &str) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("reqwest client"),
            base: base.trim_end_matches('/').to_string(),
        }
    }

    /// 校验 YM token;失败(网络/无效)返回 None
    pub async fn validate_token(&self, token: &str) -> Option<YmUserInfo> {
        let url = format!("{}/api/v1/auth/me", self.base);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let data: YmMeResponse = resp.json().await.ok()?;
        let ymid = data
            .username
            .clone()
            .or_else(|| data.id.map(|v| v.to_string()))
            .unwrap_or_default();
        if ymid.is_empty() {
            return None;
        }
        Some(YmUserInfo {
            name: data.name.clone().unwrap_or_else(|| data.username.unwrap_or(ymid.clone())),
            ymid,
            email: data.email,
            avatar: data.avatar_url,
        })
    }

    /// 发起 SSO 登录:请求 ym_auth 生成 Rauthy 授权地址。
    ///
    /// `redirect_to` 是用户在浏览器完成登录后的最终落地页;ym_auth 中转页会把
    /// `access_token` / `refresh_token`(或 `error` / `detail`)拼在它的 `?` 后面带回。
    pub async fn login_url(&self, redirect_to: &str) -> Result<String, String> {
        let url = format!(
            "{}/api/v1/auth/oidc/login?redirect_to={}",
            self.base,
            urlencode(redirect_to)
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("YM Auth 不可达: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("YM Auth 登录接口失败: HTTP {status}"));
        }
        let data: YmLoginUrlResponse = resp
            .json()
            .await
            .map_err(|e| format!("YM Auth 返回异常: {e}"))?;
        data.authorization_url
            .ok_or("YM Auth 返回缺少 authorization_url".to_string())
    }

    /// 拉全员名单(用于登记/同步本地用户表);失败返回空
    pub async fn users_list(&self, token: &str) -> Vec<YmListUser> {
        let url = format!("{}/api/v1/users/list", self.base);
        let Ok(resp) = self.http.get(&url).bearer_auth(token).send().await else {
            return Vec::new();
        };
        if !resp.status().is_success() {
            return Vec::new();
        }
        let data: YmUsersListResponse = resp.json().await.unwrap_or(YmUsersListResponse { items: vec![] });
        data.items
            .into_iter()
            .filter_map(|u| {
                let ymid = u.username?;
                Some(YmListUser {
                    ymid,
                    name: u.name.unwrap_or_else(|| "Unknown".into()),
                    phone: u.phone,
                    email: u.email,
                })
            })
            .collect()
    }
}

/// 极简 percent-encode(查询串值用, 不引额外依赖)
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
