//! 迅雷网盘驱动（对齐 Go 版 drivers/thunder XunLeiCommon）
//!
//! - 授权：账号密码登录（v3 core login 换 sessionID -> signin/token 换令牌），
//!   或用已保存的 refresh_token 刷新
//! - 列目录：GET /drive/v1/files（page_token 分页）
//! - 下载：GET /drive/v1/files/{id} 取 web_content_link，访问需下载端 UA
//!
//! 签名：captcha_sign = "1." + 对 (clientID+clientVersion+packageName+
//! deviceID+timestamp) 逐项追加 algorithms 后连续取 md5。

use super::DownloadInfo;
use crate::config::{Credential, Entry, Store};
use md5::Digest;
use reqwest::{Client, Method};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const FILE_API_URL: &str = "https://api-pan.xunlei.com/drive/v1/files";
const XLUSER_API_URL: &str = "https://xluser-ssl.xunlei.com/v1";
const XLUSER_API_BASE_URL: &str = "https://xluser-ssl.xunlei.com";

const CLIENT_ID: &str = "Xp6vsxz_7IYVw2BB";
const CLIENT_SECRET: &str = "Xp6vsy4tN9toTVdMSpomVdXpRmES";
const CLIENT_VERSION: &str = "8.31.0.9726";
const PACKAGE_NAME: &str = "com.xunlei.downloadprovider";
const USER_AGENT: &str = "ANDROID-com.xunlei.downloadprovider/8.31.0.9726 netWorkType/5G appid/40 deviceName/Xiaomi_M2004j7ac deviceModel/M2004J7AC OSVersion/12 protocolVersion/301 platformVersion/10 sdkVersion/512000 Oauth2Client/0.9 (Linux 4_14_186-perf-gddfs8vbb238b) (JAVA 0)";
const DOWNLOAD_USER_AGENT: &str =
    "Dalvik/2.1.0 (Linux; U; Android 12; M2004J7AC Build/SP1A.210812.016)";
const SIGN_PROVIDER: &str = "access_end_point_token";
const APPID: &str = "40";
const APP_KEY: &str = "34a062aaa22f906fca4fefe9fb3a3021";
/// 对齐 Go 版默认算法列表
const ALGORITHMS: [&str; 10] = [
    "9uJNVj/wLmdwKrJaVj/omlQ",
    "Oz64Lp0GigmChHMf/6TNfxx7O9PyopcczMsnf",
    "Eb+L7Ce+Ej48u",
    "jKY0",
    "ASr0zCl6v8W4aidjPK5KHd1Lq3t+vBFf41dqv5+fnOd",
    "wQlozdg6r1qxh0eRmt3QgNXOvSZO6q/GXK",
    "gmirk+ciAvIgA/cxUUCema47jr/YToixTT+Q6O",
    "5IiCoM9B1/788ntB",
    "P07JH0h6qoM6TSUAK2aL9T5s2QBVeY9JWvalf",
    "+oK0AN",
];

fn md5_hex(s: &str) -> String {
    let mut h = md5::Md5::new();
    h.update(s.as_bytes());
    hex(&h.finalize())
}

fn sha1_hex(s: &str) -> String {
    let mut h = sha1::Sha1::new();
    h.update(s.as_bytes());
    hex(&h.finalize())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// 对齐 generateDeviceSign()
fn generate_device_sign(device_id: &str, package_name: &str) -> String {
    let base = format!("{device_id}{package_name}{APPID}{APP_KEY}");
    let sha = sha1_hex(&base);
    let md5 = md5_hex(&sha);
    format!("div101.{device_id}{md5}")
}

/// 对齐 GetAction(): method + ":" + url 域名后的路径段
fn get_action(method: &str, url: &str) -> String {
    let idx = url.find("://").map(|i| i + 3).unwrap_or(0);
    let rest = &url[idx..];
    let path = match rest.split_once('/') {
        Some((_, p)) => format!("/{}", p.split(['?', '#']).next().unwrap_or("")),
        None => "/".to_string(),
    };
    format!("{method}:{path}")
}

#[derive(Debug, Clone, Default)]
struct TokenResp {
    token_type: String,
    access_token: String,
    refresh_token: String,
    user_id: String,
}

fn token_from(v: &Value) -> TokenResp {
    TokenResp {
        token_type: v
            .get("token_type")
            .and_then(|x| x.as_str())
            .unwrap_or("Bearer")
            .to_string(),
        access_token: v
            .get("access_token")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        refresh_token: v
            .get("refresh_token")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        user_id: v
            .get("user_id")
            .map(|x| match x {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => String::new(),
            })
            .unwrap_or_default(),
    }
}

pub struct Thunder {
    account_id: String,
    http: Client,
    username: String,
    password: String,
    device_id: String,
    /// 构造时已保存的 refresh_token（首次 validate 时使用）
    saved_refresh: String,
    token: Mutex<Option<TokenResp>>,
    captcha_token: Mutex<String>,
    store: Arc<Store>,
}

impl Thunder {
    pub fn new(
        account_id: &str,
        username: String,
        password: String,
        refresh_token: String,
        captcha_token: String,
        device_id: String,
        store: Arc<Store>,
    ) -> Self {
        // 对齐 Init：device_id 缺省为 md5(username+password)
        let device_id = if device_id.len() == 32 {
            device_id
        } else {
            md5_hex(&format!("{username}{password}"))
        };
        Thunder {
            account_id: account_id.to_string(),
            http: Client::new(),
            username,
            password,
            device_id,
            saved_refresh: refresh_token,
            token: Mutex::new(None),
            captcha_token: Mutex::new(captcha_token),
            store,
        }
    }

    /// 持久化 refresh_token / captcha_token / device_id（空值不覆盖）
    fn save_credential(&self, refresh_token: &str, captcha_token: &str) {
        let id = self.account_id.clone();
        let (r, c) = (refresh_token.to_string(), captcha_token.to_string());
        let dev = self.device_id.clone();
        self.store.update_credential(&id, |cred| {
            if let Credential::Thunder {
                refresh_token,
                captcha_token,
                device_id,
                ..
            } = cred
            {
                if !r.is_empty() {
                    *refresh_token = r.clone();
                }
                if !c.is_empty() {
                    *captcha_token = c.clone();
                }
                *device_id = dev.clone();
            }
        });
    }

    /// 基础请求：公共头 + 错误码处理（对齐 Common.Request + XunLeiCommon.Request）
    async fn request(
        &self,
        method: Method,
        url: &str,
        query: Option<&[(String, String)]>,
        body: Option<Value>,
        auth: bool,
        retried: bool,
    ) -> Result<Value, String> {
        let mut req = self
            .http
            .request(method.clone(), url)
            .header("user-agent", USER_AGENT)
            .header("accept", "application/json;charset=UTF-8")
            .header("x-device-id", &self.device_id)
            .header("x-client-id", CLIENT_ID)
            .header("x-client-version", CLIENT_VERSION);
        if auth {
            let token = self.token.lock().unwrap().clone();
            let Some(t) = token else {
                return Err("迅雷未登录（token 为空）".into());
            };
            req = req
                .header("Authorization", format!("{} {}", t.token_type, t.access_token))
                .header(
                    "x-captcha-token",
                    self.captcha_token.lock().unwrap().clone(),
                );
        }
        if let Some(q) = query {
            req = req.query(q);
        }
        if let Some(b) = &body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| format!("请求失败: {e}"))?;
        let v: Value = resp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
        let err_code = v.get("error_code").and_then(|x| x.as_i64()).unwrap_or(0);
        let err_msg = v
            .get("error")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let is_error = err_msg != "success" && (err_code != 0 || !err_msg.is_empty());
        if is_error {
            if err_msg == "review_panel" {
                let creditkey = v
                    .get("creditkey")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let reviewurl = v
                    .get("reviewurl")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let sign = generate_device_sign(&self.device_id, PACKAGE_NAME);
                return Err(format!(
                    "迅雷本次登录需要验证，请在浏览器打开并完成验证后重试（creditkey={creditkey}）: {reviewurl}&deviceid={sign}"
                ));
            }
            if !retried {
                // 4122/4121/10/16: token 过期；9: captcha token 过期
                // 注意：request -> do_refresh_token -> login/_refresh_token -> request
                // 构成 async 递归环，所有环边均需 Box::pin 断开
                match err_code {
                    4122 | 4121 | 10 | 16 => {
                        Box::pin(self.do_refresh_token()).await?;
                        return Box::pin(self.request(method, url, query, body, auth, true)).await;
                    }
                    9 => {
                        let user_id = self
                            .token
                            .lock()
                            .unwrap()
                            .as_ref()
                            .map(|t| t.user_id.clone())
                            .unwrap_or_default();
                        let action = get_action(method.as_str(), url);
                        Box::pin(self.refresh_captcha_token_at_login(&action, &user_id)).await?;
                        return Box::pin(self.request(method, url, query, body, auth, true)).await;
                    }
                    _ => {}
                }
            }
            let desc = v
                .get("error_description")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            return Err(format!("迅雷接口错误(code={err_code}): {err_msg} {desc}"));
        }
        Ok(v)
    }

    /// 对齐 GetCaptchaSign(): "1." + 连续 md5
    fn captcha_sign(&self) -> (String, String) {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
            .to_string();
        let mut str_val = format!(
            "{}{}{}{}{}",
            CLIENT_ID, CLIENT_VERSION, PACKAGE_NAME, self.device_id, timestamp
        );
        for algo in ALGORITHMS {
            str_val = md5_hex(&format!("{str_val}{algo}"));
        }
        (timestamp, format!("1.{str_val}"))
    }

    /// 对齐 refreshCaptchaToken()
    async fn refresh_captcha_token(&self, action: &str, metas: Value) -> Result<(), String> {
        let cur = self.captcha_token.lock().unwrap().clone();
        let body = json!({
            "action": action,
            "captcha_token": cur,
            "client_id": CLIENT_ID,
            "device_id": self.device_id,
            "meta": metas,
            "redirect_uri": "xlaccsdk01://xunlei.com/callback?state=harbor",
        });
        let resp = self
            .request(
                Method::POST,
                &format!("{XLUSER_API_URL}/shield/captcha/init"),
                None,
                Some(body),
                false,
                false,
            )
            .await?;
        if let Some(u) = resp
            .get("url")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
        {
            return Err(format!("迅雷需要人机验证，请在浏览器打开: {u}"));
        }
        let token = resp
            .get("captcha_token")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if token.is_empty() {
            return Err("迅雷 captcha_token 为空".into());
        }
        *self.captcha_token.lock().unwrap() = token.clone();
        self.save_credential("", &token);
        Ok(())
    }

    /// 对齐 RefreshCaptchaTokenAtLogin()
    async fn refresh_captcha_token_at_login(
        &self,
        action: &str,
        user_id: &str,
    ) -> Result<(), String> {
        let (timestamp, sign) = self.captcha_sign();
        let metas = json!({
            "client_version": CLIENT_VERSION,
            "package_name": PACKAGE_NAME,
            "user_id": user_id,
            "timestamp": timestamp,
            "captcha_sign": sign,
        });
        self.refresh_captcha_token(action, metas).await
    }

    /// 对齐 RefreshCaptchaTokenInLogin()
    async fn refresh_captcha_token_in_login(&self, action: &str) -> Result<(), String> {
        let metas = if self.username.contains('@') && self.username.contains('.') {
            json!({ "email": self.username })
        } else if (11..=18).contains(&self.username.len()) {
            json!({ "phone_number": self.username })
        } else {
            json!({ "username": self.username })
        };
        self.refresh_captcha_token(action, metas).await
    }

    /// 对齐 refreshTokenFunc：先刷新，失败则重新登录
    async fn do_refresh_token(&self) -> Result<(), String> {
        let cur = self
            .token
            .lock()
            .unwrap()
            .as_ref()
            .map(|t| t.refresh_token.clone())
            .or_else(|| Some(self.saved_refresh.clone()))
            .filter(|s| !s.is_empty());
        if let Some(refresh) = cur {
            match self._refresh_token(&refresh).await {
                Ok(t) => {
                    *self.token.lock().unwrap() = Some(t);
                    return Ok(());
                }
                Err(_) => return self.login().await,
            }
        }
        self.login().await
    }

    /// 对齐 RefreshToken()
    async fn _refresh_token(&self, refresh_token: &str) -> Result<TokenResp, String> {
        let body = json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": CLIENT_ID,
            "client_secret": CLIENT_SECRET,
        });
        // request -> do_refresh_token -> _refresh_token -> request 构成 async 递归环，
        // 此处装箱断开（对齐 pan123 的 Box::pin 做法）
        let resp = Box::pin(self.request(
            Method::POST,
            &format!("{XLUSER_API_URL}/auth/token"),
            None,
            Some(body),
            false,
            false,
        ))
        .await?;
        if resp
            .get("refresh_token")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .is_empty()
        {
            return Err("迅雷刷新 token 返回空 refresh_token".into());
        }
        let t = token_from(&resp);
        self.save_credential(&t.refresh_token, "");
        Ok(t)
    }

    /// 对齐 CoreLogin()：v3 登录取 sessionID
    async fn core_login(&self) -> Result<String, String> {
        let body = json!({
            "protocolVersion": "301",
            "sequenceNo": "1000012",
            "platformVersion": "10",
            "isCompressed": "0",
            "appid": APPID,
            "clientVersion": CLIENT_VERSION,
            "peerID": "00000000000000000000000000000000",
            "appName": "ANDROID-com.xunlei.downloadprovider",
            "sdkVersion": "512000",
            "devicesign": generate_device_sign(&self.device_id, PACKAGE_NAME),
            "netWorkType": "WIFI",
            "providerName": "NONE",
            "deviceModel": "M2004J7AC",
            "deviceName": "Xiaomi_M2004j7ac",
            "OSVersion": "12",
            "creditkey": "",
            "hl": "zh-CN",
            "userName": self.username,
            "passWord": self.password,
            "verifyKey": "",
            "verifyCode": "",
            "isMd5Pwd": "0",
        });
        let resp = self
            .http
            .post(format!("{XLUSER_API_BASE_URL}/xluser.core.login/v3/login"))
            .header(
                "User-Agent",
                "android-ok-http-client/xl-acc-sdk/version-5.0.12.512000",
            )
            .header("accept", "application/json;charset=UTF-8")
            .header("x-device-id", &self.device_id)
            .header("x-client-id", CLIENT_ID)
            .header("x-client-version", CLIENT_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("迅雷登录请求失败: {e}"))?;
        let v: Value = resp
            .json()
            .await
            .map_err(|e| format!("登录响应解析失败: {e}"))?;
        if let Some(err) = v
            .get("error")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty() && *s != "success")
        {
            let desc = v
                .get("error_description")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            return Err(format!("迅雷登录失败: {err} {desc}"));
        }
        let session_id = v
            .get("sessionID")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if session_id.is_empty() {
            return Err("迅雷登录失败：未返回 sessionID".into());
        }
        Ok(session_id)
    }

    /// 对齐 Login()：core login -> captcha token -> signin/token
    async fn login(&self) -> Result<(), String> {
        let session_id = self.core_login().await?;
        let url = format!("{XLUSER_API_URL}/auth/signin/token");
        let action = get_action("POST", &url);
        self.refresh_captcha_token_in_login(&action).await?;
        let body = json!({
            "client_id": CLIENT_ID,
            "client_secret": CLIENT_SECRET,
            "provider": SIGN_PROVIDER,
            "signin_token": session_id,
        });
        let resp = self
            .request(Method::POST, &url, None, Some(body), false, false)
            .await?;
        if resp
            .get("refresh_token")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .is_empty()
        {
            return Err("迅雷登录失败：未返回 refresh_token".into());
        }
        let t = token_from(&resp);
        *self.token.lock().unwrap() = Some(t.clone());
        self.save_credential(&t.refresh_token, "");
        Ok(())
    }

    /// 对齐 Init()/IsLogin()：登录并校验
    pub async fn validate(&self) -> Result<(), String> {
        if self.token.lock().unwrap().is_none() {
            if !self.saved_refresh.is_empty() {
                match self._refresh_token(&self.saved_refresh.clone()).await {
                    Ok(t) => {
                        *self.token.lock().unwrap() = Some(t);
                    }
                    Err(_) => self.login().await?,
                }
            } else {
                self.login().await?;
            }
        }
        self.request(
            Method::GET,
            &format!("{XLUSER_API_URL}/user/me"),
            None,
            None,
            true,
            false,
        )
        .await?;
        Ok(())
    }

    /// 对齐 getFiles()：page_token 分页
    pub async fn list(&self, parent_id: &str) -> Result<Vec<Entry>, String> {
        let mut files = Vec::new();
        let mut page_token = String::new();
        loop {
            let query = vec![
                ("space".into(), String::new()),
                ("__type".into(), "drive".into()),
                ("refresh".into(), "true".into()),
                ("__sync".into(), "true".into()),
                ("parent_id".into(), parent_id.to_string()),
                ("page_token".into(), page_token.clone()),
                ("with_audit".into(), "true".into()),
                ("limit".into(), "100".into()),
                (
                    "filters".into(),
                    r#"{"phase":{"eq":"PHASE_TYPE_COMPLETE"},"trashed":{"eq":false}}"#.into(),
                ),
            ];
            let resp = self
                .request(Method::GET, FILE_API_URL, Some(&query), None, true, false)
                .await?;
            let list = resp
                .get("files")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for f in &list {
                let updated_at = f
                    .get("modified_time")
                    .and_then(|v| v.as_str())
                    .and_then(super::aliyundrive_open::iso_to_ms);
                files.push(Entry {
                    fid: f.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    name: f
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    size: f
                        .get("size")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0),
                    is_dir: f.get("kind").and_then(|v| v.as_str()) == Some("drive#folder"),
                    updated_at,
                    etag: None,
                    s3_key_flag: None,
                    file_type: None,
                    extra: None,
                });
            }
            page_token = resp
                .get("next_page_token")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if page_token.is_empty() {
                break;
            }
        }
        Ok(files)
    }

    /// 对齐 Link()：取 web_content_link，下载需 Dalvik UA
    pub async fn download(&self, e: &Entry) -> Result<DownloadInfo, String> {
        let url = format!("{FILE_API_URL}/{}", e.fid);
        let resp = self
            .request(
                Method::GET,
                &url,
                Some(&[("space".into(), String::new())]),
                None,
                true,
                false,
            )
            .await?;
        let link = resp
            .get("web_content_link")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if link.is_empty() {
            return Err("迅雷未返回下载直链".into());
        }
        Ok(DownloadInfo {
            url: link,
            headers: vec![("User-Agent".into(), DOWNLOAD_USER_AGENT.into())],
            proxy: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_action() {
        assert_eq!(
            get_action("POST", "https://xluser-ssl.xunlei.com/v1/auth/signin/token"),
            "POST:/v1/auth/signin/token"
        );
        assert_eq!(get_action("GET", "https://a.b/c/d?x=1"), "GET:/c/d");
    }

    #[test]
    fn test_device_sign_format() {
        let s = generate_device_sign("00000000000000000000000000000000", PACKAGE_NAME);
        assert!(s.starts_with("div101."));
    }
}
