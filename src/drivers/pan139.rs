//! 中国移动云盘（139）驱动
//!
//! 移植自 OpenList Go 版 drivers/139，支持：
//! - personal_new：新版个人云（默认，对应 OpenList 139Yun 的 `personal_new` 类型）
//! - personal：旧版个人云
//!
//! 凭据为 Authorization（OpenList 139Yun 同款格式）：
//! base64("Basic <tokenblob>:<手机号>:<token>|<...>|<过期时间ms>|...")，
//! 剩余有效期不足 15 天时自动调用 tellin/authTokenRefresh.do 刷新并回写配置。
//!
//! 家庭云 / 共享组 / 分享链接类型未实现（openlist-rs 仅支持 list + download）。

use super::DownloadInfo;
use crate::config::{Credential, Entry, Store};
use base64::Engine;
use md5::{Digest, Md5};
use rand::Rng;
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

/// Authorization 距过期的提前刷新阈值：15 天（对齐 Go 版）
const REFRESH_THRESHOLD_MS: i64 = 1000 * 60 * 60 * 24 * 15;
const AUTH_REFRESH_URL: &str = "https://aas.caiyun.feixin.10086.cn:443/tellin/authTokenRefresh.do";
const OLD_BASE: &str = "https://yun.139.com";

// ---------- 时间工具（对齐 Go 版 getTime / getPersonalTime，CN = UTC+8） ----------

/// Howard Hinnant days_from_civil
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe
}

/// 北京时间 (UTC+8) 各字段 -> epoch 毫秒
fn cn_ymdhms_to_ms(y: i64, m: u32, d: u32, hh: u32, mm: u32, ss: u32, ms: u32) -> i64 {
    let days = days_from_civil(y, m, d);
    (days * 86_400 + hh as i64 * 3600 + mm as i64 * 60 + ss as i64 - 8 * 3600) * 1000 + ms as i64
}

/// 当前北京时间字段（用于 mcloud-sign 的 ts）
fn cn_now_parts() -> (i64, u32, u32, u32, u32, u32) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let secs = now.as_secs() as i64 + 8 * 3600;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (
        y,
        m as u32,
        d as u32,
        (rem / 3600) as u32,
        (rem % 3600 / 60) as u32,
        (rem % 60) as u32,
    )
}

/// 解析 "2024-01-02T15:04:05(.fff)?(Z|±hh:mm)" -> epoch 毫秒（对齐 Go getPersonalTime）
fn parse_personal_time(t: &str) -> Option<i64> {
    let b = t.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| std::str::from_utf8(&b[r]).ok()?.parse::<u32>().ok();
    let y = num(0..4)? as i64;
    let m = num(5..7)?;
    let d = num(8..10)?;
    let hh = num(11..13)?;
    let mm = num(14..16)?;
    let ss = num(17..19)?;
    let mut rest = &t[19..];
    // 毫秒
    let mut ms = 0u32;
    if let Some(frac) = rest.strip_prefix('.') {
        let end = frac
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(frac.len());
        let digits = &frac[..end];
        if !digits.is_empty() {
            ms = format!("{:0<3}", &digits[..digits.len().min(3)])
                .parse()
                .unwrap_or(0);
        }
        rest = &frac[end..];
    }
    // 时区偏移
    let offset_secs: i64 = if rest.starts_with('+') || rest.starts_with('-') {
        let sign = if rest.starts_with('-') { -1 } else { 1 };
        let rb = rest.as_bytes();
        if rb.len() < 6 {
            return None;
        }
        let oh: i64 = rest[1..3].parse().ok()?;
        let om: i64 = rest[4..6].parse().ok()?;
        sign * (oh * 3600 + om * 60)
    } else {
        0 // Z 或缺失按 UTC 处理
    };
    let base = days_from_civil(y, m, d) * 86_400
        + hh as i64 * 3600
        + mm as i64 * 60
        + ss as i64
        - offset_secs;
    Some(base * 1000 + ms as i64)
}

/// 解析 "20240102150405"（北京时间）-> epoch 毫秒（对齐 Go getTime）
fn parse_cn_compact_time(t: &str) -> Option<i64> {
    let b = t.as_bytes();
    if b.len() < 14 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| std::str::from_utf8(&b[r]).ok()?.parse::<u32>().ok();
    Some(cn_ymdhms_to_ms(
        num(0..4)? as i64,
        num(4..6)?,
        num(6..8)?,
        num(8..10)?,
        num(10..12)?,
        num(12..14)?,
        0,
    ))
}

// ---------- 签名（对齐 Go 版 calSign） ----------

/// RFC3986 percent-encode（等价 Go encodeURIComponent）
fn percent_encode(s: &str) -> String {
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

fn md5_hex(s: &str) -> String {
    let mut h = Md5::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

/// 对齐 Go calSign(body, ts, randStr)
fn cal_sign(body: &str, ts: &str, rand_str: &str) -> String {
    let encoded = percent_encode(body);
    // 按字符排序后拼接（UTF-8 字节序与码点序一致，等价 Go 按 rune 排序）
    let mut chars: Vec<char> = encoded.chars().collect();
    chars.sort();
    let sorted: String = chars.into_iter().collect();
    let b64 = base64::engine::general_purpose::STANDARD.encode(sorted.as_bytes());
    let res = format!("{}{}", md5_hex(&b64), md5_hex(&format!("{ts}:{rand_str}")));
    md5_hex(&res).to_uppercase()
}

fn rand_string(n: usize) -> String {
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::thread_rng();
    (0..n)
        .map(|_| CHARSET[rng.gen_range(0..CHARSET.len()) as usize] as char)
        .collect()
}

// ---------- 驱动 ----------

pub struct Yun139 {
    account_id: String,
    http: Client,
    authorization: Mutex<String>,
    /// 从 authorization 中解析出的手机号（commonAccountInfo.account）
    account: Mutex<String>,
    drive_type: String,
    /// 新版个人云 API host（路由策略查询获得）
    personal_host: Mutex<String>,
    store: Arc<Store>,
}

/// authorization 解码结果
struct AuthParts {
    prefix: String,
    account: String,
    token: String,
    expire_ms: i64,
}

impl Yun139 {
    pub fn new(
        account_id: &str,
        authorization: String,
        drive_type: String,
        store: Arc<Store>,
    ) -> Self {
        let account = Self::decode_authorization(&authorization)
            .map(|p| p.account)
            .unwrap_or_default();
        Yun139 {
            account_id: account_id.to_string(),
            http: Client::new(),
            authorization: Mutex::new(authorization),
            account: Mutex::new(account),
            drive_type,
            personal_host: Mutex::new(String::new()),
            store,
        }
    }

    /// base64 解码 authorization 并拆出 (prefix, account, token, expire_ms)
    fn decode_authorization(auth: &str) -> Result<AuthParts, String> {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(auth.trim())
            .map_err(|e| format!("139 authorization 解码失败: {e}"))?;
        let decoded = String::from_utf8(decoded)
            .map_err(|e| format!("139 authorization 非 UTF-8: {e}"))?;
        let splits: Vec<&str> = decoded.split(':').collect();
        if splits.len() < 3 {
            return Err("139 authorization 格式非法（冒号段不足 3 段）".into());
        }
        let strs: Vec<&str> = splits[2].split('|').collect();
        if strs.len() < 4 {
            return Err("139 authorization 的 token 段非法（| 段不足 4 段）".into());
        }
        let expire_ms: i64 = strs[3]
            .parse()
            .map_err(|_| "139 authorization 的过期时间非法".to_string())?;
        Ok(AuthParts {
            prefix: splits[0].to_string(),
            account: splits[1].to_string(),
            token: splits[2].to_string(),
            expire_ms,
        })
    }

    fn save_authorization(&self, auth: &str) {
        *self.authorization.lock().unwrap() = auth.to_string();
        if let Ok(parts) = Self::decode_authorization(auth) {
            *self.account.lock().unwrap() = parts.account;
        }
        let auth = auth.to_string();
        let id = self.account_id.clone();
        self.store.update_credential(&id, |cred| {
            if let Credential::Yun139 { authorization, .. } = cred {
                *authorization = auth.clone();
            }
        });
    }

    /// 对齐 Go refreshToken()：临期自动刷新 token
    async fn refresh_token(&self) -> Result<(), String> {
        let parts = {
            let auth = self.authorization.lock().unwrap();
            Self::decode_authorization(&auth)?
        };
        let remaining = parts.expire_ms - now_ms();
        if remaining > REFRESH_THRESHOLD_MS {
            return Ok(());
        }
        if remaining < 0 {
            return Err("139 authorization 已过期，请重新抓取".into());
        }
        let req_body = format!(
            "<root><token>{}</token><account>{}</account><clienttype>656</clienttype></root>",
            parts.token, parts.account
        );
        let resp = self
            .http
            .post(AUTH_REFRESH_URL)
            .header("Content-Type", "application/xml")
            .body(req_body)
            .send()
            .await
            .map_err(|e| format!("139 token 刷新请求失败: {e}"))?;
        let text = resp.text().await.map_err(|e| format!("139 刷新响应读取失败: {e}"))?;
        let tag = |name: &str| -> Option<String> {
            let re = format!("<{name}>(.*?)</{name}>");
            regex::Regex::new(&re)
                .ok()?
                .captures(&text)?
                .get(1)
                .map(|m| m.as_str().to_string())
        };
        let ret = tag("return").unwrap_or_default();
        if ret != "0" {
            let desc = tag("desc").unwrap_or_default();
            return Err(format!("139 token 刷新失败: return={ret}, desc={desc}"));
        }
        let new_token = tag("token")
            .filter(|t| !t.is_empty())
            .ok_or_else(|| format!("139 刷新响应中无 token: {text}"))?;
        let new_auth = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}:{}", parts.prefix, parts.account, new_token));
        self.save_authorization(&new_auth);
        Ok(())
    }

    /// 统一请求封装：
    /// - style="old"：旧版头部（https://yun.139.com / 路由策略）
    /// - style="new"：新版个人云头部（路由策略返回的 host）
    async fn api_request(&self, style: &str, base: &str, pathname: &str, data: Value) -> Result<Value, String> {
        let body = serde_json::to_string(&data).map_err(|e| format!("139 序列化失败: {e}"))?;
        let (y, m, d, hh, mm, ss) = cn_now_parts();
        let ts = format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}");
        let rand_str = rand_string(16);
        let sign = cal_sign(&body, &ts, &rand_str);
        let auth = self.authorization.lock().unwrap().clone();
        let url = format!("{base}{pathname}");

        let mut req = self.http.post(&url).header("Accept", "application/json, text/plain, */*");
        if style == "new" {
            req = req
                .header("Authorization", format!("Basic {auth}"))
                .header("Caller", "web")
                .header("Cms-Device", "default")
                .header("Mcloud-Channel", "1000101")
                .header("Mcloud-Client", "10701")
                .header("Mcloud-Route", "001")
                .header("Mcloud-Sign", format!("{ts},{rand_str},{sign}"))
                .header("Mcloud-Version", "7.14.0")
                .header("x-DeviceInfo", "||9|7.14.0|chrome|120.0.0.0|||windows 10||zh-CN|||")
                .header("x-huawei-channelSrc", "10000034")
                .header("x-inner-ntwk", "2")
                .header("x-m4c-caller", "PC")
                .header("x-m4c-src", "10002")
                .header("x-SvcType", "1")
                .header("X-Yun-Api-Version", "v1")
                .header("X-Yun-App-Channel", "10000034")
                .header("X-Yun-Channel-Source", "10000034")
                .header("X-Yun-Client-Info", "||9|7.14.0|chrome|120.0.0.0|||windows 10||zh-CN|||dW5kZWZpbmVk||")
                .header("X-Yun-Module-Type", "100")
                .header("X-Yun-Svc-Type", "1");
        } else {
            req = req
                .header("CMS-DEVICE", "default")
                .header("Authorization", format!("Basic {auth}"))
                .header("mcloud-channel", "1000101")
                .header("mcloud-client", "10701")
                .header("mcloud-sign", format!("{ts},{rand_str},{sign}"))
                .header("mcloud-version", "7.14.0")
                .header("Origin", "https://yun.139.com")
                .header("Referer", "https://yun.139.com/w/")
                .header("x-DeviceInfo", "||9|7.14.0|chrome|120.0.0.0|||windows 10||zh-CN|||")
                .header("x-huawei-channelSrc", "10000034")
                .header("x-inner-ntwk", "2")
                .header("x-m4c-caller", "PC")
                .header("x-m4c-src", "10002")
                .header("x-SvcType", "1")
                .header("Inner-Hcy-Router-Https", "1");
        }

        let resp = req
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("139 请求失败: {e}"))?;
        let v: Value = resp
            .json()
            .await
            .map_err(|e| format!("139 响应解析失败: {e}"))?;
        // 对齐 Go BaseResp：success=false 视为错误
        let success = v.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
        if !success {
            let msg = v
                .get("message")
                .and_then(|m| m.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("unknown");
            return Err(format!("139 接口错误: {msg}"));
        }
        Ok(v)
    }

    /// 对齐 Go requestRoute()：查询路由策略，取新版个人云 host
    async fn refresh_route_policy(&self) -> Result<(), String> {
        let account = self.account.lock().unwrap().clone();
        let data = json!({
            "userInfo": {
                "userType": 1,
                "accountType": 1,
                "accountName": account,
            },
            "modAddrType": 1,
        });
        let v = self
            .api_request("old", "https://user-njs.yun.139.com", "/user/route/qryRoutePolicy", data)
            .await?;
        let mut host = String::new();
        if let Some(list) = v.pointer("/data/routePolicyList").and_then(|l| l.as_array()) {
            for item in list {
                if item.get("modName").and_then(|m| m.as_str()) == Some("personal") {
                    host = item
                        .get("httpsUrl")
                        .and_then(|u| u.as_str())
                        .unwrap_or("")
                        .trim_end_matches('/')
                        .to_string();
                }
            }
        }
        if host.is_empty() {
            return Err("139 PersonalCloudHost 为空".into());
        }
        *self.personal_host.lock().unwrap() = host;
        Ok(())
    }

    /// 对齐 Go Init()
    pub async fn validate(&self) -> Result<(), String> {
        self.refresh_token().await?;
        if self.drive_type == "personal_new" {
            self.refresh_route_policy().await?;
        }
        Ok(())
    }

    fn normalize_fid(&self, fid: &str) -> String {
        let is_new = self.drive_type == "personal_new";
        let root = if is_new { "/" } else { "root" };
        if fid.is_empty() || fid == "0" {
            root.to_string()
        } else {
            fid.to_string()
        }
    }

    fn common_account(&self) -> Value {
        json!({
            "account": self.account.lock().unwrap().clone(),
            "accountType": 1,
        })
    }

    /// 对齐 Go personalGetFiles()：新版个人云目录列表（cursor 分页）
    async fn personal_list(&self, fid: &str) -> Result<Vec<Entry>, String> {
        let host = self.personal_host.lock().unwrap().clone();
        let mut files = Vec::new();
        let mut cursor = String::new();
        loop {
            let data = json!({
                "imageThumbnailStyleList": ["Small", "Large"],
                "orderBy": "updated_at",
                "orderDirection": "DESC",
                "pageInfo": {
                    "pageCursor": cursor,
                    "pageSize": 100,
                },
                "parentFileId": fid,
            });
            let v = self.api_request("new", &host, "/file/list", data).await?;
            cursor = v
                .pointer("/data/nextPageCursor")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            let items = v
                .pointer("/data/items")
                .and_then(|i| i.as_array())
                .cloned()
                .unwrap_or_default();
            for item in &items {
                let is_dir = item.get("type").and_then(|t| t.as_str()) == Some("folder");
                files.push(Entry {
                    fid: item.get("fileId").and_then(|f| f.as_str()).unwrap_or("").to_string(),
                    name: item.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                    size: item.get("size").and_then(|s| s.as_u64()).unwrap_or(0),
                    is_dir,
                    updated_at: item
                        .get("updatedAt")
                        .and_then(|u| u.as_str())
                        .and_then(parse_personal_time),
                    etag: None,
                    s3_key_flag: None,
                    file_type: None,
                    extra: None,
                });
            }
            if cursor.is_empty() {
                break;
            }
        }
        Ok(files)
    }

    /// 对齐 Go getFiles()：旧版个人云目录列表（start/end 分页）
    async fn old_personal_list(&self, fid: &str) -> Result<Vec<Entry>, String> {
        let mut files = Vec::new();
        let mut start: usize = 0;
        let limit: usize = 100;
        loop {
            let data = json!({
                "catalogID": fid,
                "sortDirection": 1,
                "startNumber": start + 1,
                "endNumber": start + limit,
                "filterType": 0,
                "catalogSortType": 0,
                "contentSortType": 0,
                "commonAccountInfo": self.common_account(),
            });
            let v = self
                .api_request(
                    "old",
                    OLD_BASE,
                    "/orchestration/personalCloud/catalog/v1.0/getDisk",
                    data,
                )
                .await?;
            let result = v.pointer("/data/getDiskResult").cloned().unwrap_or(Value::Null);
            if let Some(list) = result.get("catalogList").and_then(|l| l.as_array()) {
                for c in list {
                    files.push(Entry {
                        fid: c.get("catalogID").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        name: c.get("catalogName").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        size: 0,
                        is_dir: true,
                        updated_at: c
                            .get("updateTime")
                            .and_then(|v| v.as_str())
                            .and_then(parse_cn_compact_time),
                        etag: None,
                        s3_key_flag: None,
                        file_type: None,
                        extra: None,
                    });
                }
            }
            if let Some(list) = result.get("contentList").and_then(|l| l.as_array()) {
                for c in list {
                    files.push(Entry {
                        fid: c.get("contentID").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        name: c.get("contentName").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        size: c.get("contentSize").and_then(|v| v.as_u64()).unwrap_or(0),
                        is_dir: false,
                        updated_at: c
                            .get("updateTime")
                            .and_then(|v| v.as_str())
                            .and_then(parse_cn_compact_time),
                        etag: c.get("digest").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        s3_key_flag: None,
                        file_type: None,
                        extra: None,
                    });
                }
            }
            let node_count = result
                .get("nodeCount")
                .and_then(|n| n.as_u64())
                .unwrap_or(0) as usize;
            if start + limit >= node_count {
                break;
            }
            start += limit;
        }
        Ok(files)
    }

    /// 对齐 Go personalGetLink()：新版个人云直链
    async fn personal_link(&self, fid: &str) -> Result<String, String> {
        let host = self.personal_host.lock().unwrap().clone();
        let v = self
            .api_request("new", &host, "/file/getDownloadUrl", json!({ "fileId": fid }))
            .await?;
        let cdn_url = v.pointer("/data/cdnUrl").and_then(|u| u.as_str()).unwrap_or("");
        if !cdn_url.is_empty()
            && v.pointer("/data/cdnSwitch").and_then(|s| s.as_bool()).unwrap_or(false)
        {
            return Ok(cdn_url.to_string());
        }
        let url = v.pointer("/data/url").and_then(|u| u.as_str()).unwrap_or("");
        if url.is_empty() {
            return Err("139 未返回下载直链".into());
        }
        Ok(url.to_string())
    }

    /// 对齐 Go getLink()：旧版个人云直链
    async fn old_personal_link(&self, fid: &str) -> Result<String, String> {
        let v = self
            .api_request(
                "old",
                OLD_BASE,
                "/orchestration/personalCloud/uploadAndDownload/v1.0/downloadRequest",
                json!({
                    "appName": "",
                    "contentID": fid,
                    "commonAccountInfo": self.common_account(),
                }),
            )
            .await?;
        let url = v
            .pointer("/data/downloadURL")
            .and_then(|u| u.as_str())
            .unwrap_or("");
        if url.is_empty() {
            return Err("139 未返回下载直链".into());
        }
        Ok(url.to_string())
    }

    pub async fn list(&self, parent_fid: &str) -> Result<Vec<Entry>, String> {
        let fid = self.normalize_fid(parent_fid);
        if self.drive_type == "personal_new" {
            self.personal_list(&fid).await
        } else {
            self.old_personal_list(&fid).await
        }
    }

    pub async fn download(&self, e: &Entry) -> Result<DownloadInfo, String> {
        let url = if self.drive_type == "personal_new" {
            self.personal_link(&e.fid).await?
        } else {
            self.old_personal_link(&e.fid).await?
        };
        Ok(DownloadInfo {
            url,
            headers: vec![],
            proxy: false,
        })
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
