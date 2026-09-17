//! 阿里云盘开放平台驱动（对齐 Go 版 drivers/aliyundrive_open）
//!
//! - 授权：refresh_token 换 access_token（默认走 olist 在线刷新 API，
//!   无需 client_id/client_secret）
//! - 列目录：POST /adrive/v1.0/openFile/list（marker 分页，limit 200）
//! - 下载：POST /adrive/v1.0/openFile/getDownloadUrl（返回无需附加头的直链）

use super::DownloadInfo;
use crate::config::{Credential, Entry, Store};
use base64::Engine;
use reqwest::{Client, Method};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const API_URL: &str = "https://openapi.alipan.com";
/// 对齐 Go 版默认在线刷新 API
const ONLINE_REFRESH_API: &str = "https://api.oplist.org/alicloud/renewapi";
/// access_token 失效错误码
const TOKEN_EXPIRED_CODES: [&str; 3] = ["AccessTokenInvalid", "AccessTokenExpired", "I400JD"];

pub struct AliyundriveOpen {
    account_id: String,
    http: Client,
    refresh_token: Mutex<String>,
    access_token: Mutex<String>,
    alipan_type: String,
    drive_id: Mutex<String>,
    store: Arc<Store>,
}

/// ISO8601 时间（如 2024-01-02T15:04:05.000Z / +08:00）-> unix 毫秒
pub(crate) fn iso_to_ms(s: &str) -> Option<i64> {
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    if s.len() < 19 {
        return None;
    }
    let y = num(0..4)?;
    let mo = num(5..7)?;
    let d = num(8..10)?;
    let h = num(11..13)?;
    let mi = num(14..16)?;
    let se = num(17..19)?;
    let ms = num(20..23).unwrap_or(0);
    let mut secs = days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se;
    // 时区后缀：Z / +hh:mm / -hh:mm（可能跟在毫秒段之后，需先定位符号位）
    let tz = &s[19..];
    if let Some(pos) = tz.find(['+', '-']) {
        let sign: i64 = if tz.as_bytes()[pos] == b'+' { -1 } else { 1 };
        let nums: Vec<i64> = tz[pos + 1..]
            .split(':')
            .filter_map(|p| p.parse().ok())
            .collect();
        if nums.len() == 2 {
            secs += sign * (nums[0] * 3600 + nums[1] * 60);
        }
    }
    Some(secs * 1000 + ms)
}

/// Howard Hinnant days_from_civil
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

impl AliyundriveOpen {
    pub fn new(
        account_id: &str,
        refresh_token: String,
        access_token: String,
        alipan_type: String,
        store: Arc<Store>,
    ) -> Self {
        AliyundriveOpen {
            account_id: account_id.to_string(),
            http: Client::new(),
            refresh_token: Mutex::new(refresh_token),
            access_token: Mutex::new(access_token),
            alipan_type,
            drive_id: Mutex::new(String::new()),
            store,
        }
    }

    fn save_tokens(&self, refresh: &str, access: &str) {
        *self.refresh_token.lock().unwrap() = refresh.to_string();
        *self.access_token.lock().unwrap() = access.to_string();
        let (r, a) = (refresh.to_string(), access.to_string());
        let id = self.account_id.clone();
        self.store.update_credential(&id, |cred| {
            if let Credential::AliyundriveOpen {
                refresh_token,
                access_token,
                ..
            } = cred
            {
                *refresh_token = r.clone();
                *access_token = a.clone();
            }
        });
    }

    fn access_token(&self) -> String {
        self.access_token.lock().unwrap().clone()
    }

    /// 对齐 Go 版 refreshToken()：走在线刷新 API
    async fn refresh_token(&self) -> Result<(), String> {
        let cur = self.refresh_token.lock().unwrap().clone();
        let url = format!(
            "{}?refresh_ui={}&server_use=true&driver_txt={}",
            ONLINE_REFRESH_API,
            urlencoding(&cur),
            if self.alipan_type == "alipanTV" {
                "alicloud_tv"
            } else {
                "alicloud_qr"
            }
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("刷新阿里云盘 token 失败: {e}"))?;
        let v: Value = resp
            .json()
            .await
            .map_err(|e| format!("刷新响应解析失败: {e}"))?;
        let refresh = v
            .get("refresh_token")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let access = v
            .get("access_token")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if refresh.is_empty() || access.is_empty() {
            let msg = v
                .get("text")
                .and_then(|x| x.as_str())
                .unwrap_or("在线 API 返回空 token，refresh_token 可能已失效");
            return Err(format!("刷新阿里云盘 token 失败: {msg}"));
        }
        // 校验 jwt sub 一致（对齐 Go 版 getSub 比对）
        if !cur.is_empty() && sub_of(&cur) != sub_of(&refresh) {
            return Err("刷新阿里云盘 token 失败: sub 不匹配".into());
        }
        self.save_tokens(&refresh, &access);
        Ok(())
    }

    /// 对齐 Go 版 requestReturnErrResp：token 失效自动刷新重试一次
    async fn request(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
        retried: bool,
    ) -> Result<Value, String> {
        let url = format!("{API_URL}{uri}");
        let mut req = self
            .http
            .request(method, &url)
            .header("Authorization", format!("Bearer {}", self.access_token()));
        if let Some(b) = &body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| format!("请求失败: {e}"))?;
        let status = resp.status();
        let v: Value = resp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
        let code = v.get("code").and_then(|c| c.as_str()).unwrap_or("");
        if !code.is_empty() {
            if !retried && (TOKEN_EXPIRED_CODES.contains(&code) || self.access_token().is_empty())
            {
                self.refresh_token().await?;
                return Box::pin(self.request(Method::POST, uri, body, true)).await;
            }
            let msg = v
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            return Err(format!("阿里云盘接口错误({code}): {msg}"));
        }
        if status.as_u16() >= 400 {
            return Err(format!("阿里云盘接口 HTTP {status}"));
        }
        Ok(v)
    }

    /// 对齐 Go 版 Init()：取 drive_id 并验证 token
    pub async fn validate(&self) -> Result<(), String> {
        let res = self
            .request(Method::POST, "/adrive/v1.0/user/getDriveInfo", None, false)
            .await?;
        // AlipanType=alipanTV 用资源盘，其余按 default -> resource -> backup 兜底
        let keys: &[&str] = if self.alipan_type == "alipanTV" {
            &["resource_drive_id", "default_drive_id"]
        } else {
            &["default_drive_id", "resource_drive_id", "backup_drive_id"]
        };
        let drive_id = keys
            .iter()
            .find_map(|k| res.get(k).and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        *self.drive_id.lock().unwrap() = drive_id;
        Ok(())
    }

    /// 对齐 Go 版 getFiles()：marker 分页
    pub async fn list(&self, parent_file_id: &str) -> Result<Vec<Entry>, String> {
        // 根目录固定为 "root"（Go 版 DefaultRoot）；旧配置里可能存了兜底值 "0"
        let parent_file_id = match parent_file_id {
            "" | "0" => "root",
            other => other,
        };
        let drive_id = self.drive_id.lock().unwrap().clone();
        let mut files = Vec::new();
        let mut marker = String::new();
        loop {
            let body = json!({
                "drive_id": drive_id,
                "limit": 200,
                "marker": marker,
                "order_by": "updated_at",
                "order_direction": "DESC",
                "parent_file_id": parent_file_id,
            });
            let resp = self
                .request(Method::POST, "/adrive/v1.0/openFile/list", Some(body), false)
                .await?;
            let items = resp
                .get("items")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for f in &items {
                let updated_at = f
                    .get("updated_at")
                    .and_then(|v| v.as_str())
                    .and_then(iso_to_ms);
                files.push(Entry {
                    fid: f
                        .get("file_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    name: f.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    size: f.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
                    is_dir: f.get("type").and_then(|v| v.as_str()) == Some("folder"),
                    updated_at,
                    etag: None,
                    s3_key_flag: None,
                    file_type: None,
                    extra: None,
                });
            }
            marker = resp
                .get("next_marker")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if marker.is_empty() {
                break;
            }
        }
        Ok(files)
    }

    /// 对齐 Go 版 Link()：POST /adrive/v1.0/openFile/getDownloadUrl
    pub async fn download(&self, e: &Entry) -> Result<DownloadInfo, String> {
        let drive_id = self.drive_id.lock().unwrap().clone();
        let body = json!({
            "drive_id": drive_id,
            "file_id": e.fid,
            "expire_sec": 14400,
        });
        let resp = self
            .request(
                Method::POST,
                "/adrive/v1.0/openFile/getDownloadUrl",
                Some(body),
                false,
            )
            .await?;
        let url = resp
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if url.is_empty() {
            return Err("阿里云盘未返回下载直链".into());
        }
        // 直链为 CDN 地址，无附加头即可访问
        Ok(DownloadInfo {
            url,
            headers: vec![],
            proxy: false,
            local_path: None,
        })
    }
}

/// 从 JWT payload 里取 sub
fn sub_of(token: &str) -> String {
    let segs: Vec<&str> = token.split('.').collect();
    if segs.len() != 3 {
        return String::new();
    }
    base64::engine::general_purpose::STANDARD
        .decode(segs[1])
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| {
            v.get("sub")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default()
}

fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::iso_to_ms;

    #[test]
    fn test_iso_to_ms() {
        // 2024-01-02T15:04:05.000Z = 1704207845000
        assert_eq!(iso_to_ms("2024-01-02T15:04:05.000Z"), Some(1704207845000));
        // 1970-01-01T00:00:00Z
        assert_eq!(iso_to_ms("1970-01-01T00:00:00Z"), Some(0));
        // +08:00 时区（23:04:05+08:00 = 15:04:05Z）
        assert_eq!(
            iso_to_ms("2024-01-02T23:04:05.000+08:00"),
            Some(1704207845000)
        );
    }
}
