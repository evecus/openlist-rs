use super::DownloadInfo;
use crate::config::{Credential, Entry, Store};
use base64::Engine;
use reqwest::{Client, Method, redirect};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const MAIN_API: &str = "https://yun.123pan.com/b/api";
const SIGN_IN: &str = "https://login.123pan.com/api/user/sign_in";
const FILE_LIST: &str = "/file/list/new";
const DOWNLOAD_INFO: &str = "/file/download_info";
const USER_INFO: &str = "/user/info";
/// 对齐 Go 版 APIRateLimit: 700ms/请求（仅列表接口）
const LIST_RATE_LIMIT_MS: u64 = 700;

// ---------- signPath 移植（对齐 Go 版 drivers/123/util.go） ----------

const SIGN_TABLE: &[u8] = b"adefghlmyijnopkqrstubcvwsz";

/// IEEE CRC32
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = !0;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// unix 秒 -> 北京时间 (UTC+8) "yyyyMMddHHmm" 字符串
fn beijing_yyyymmddhhmm(unix_secs: u64) -> String {
    let secs = unix_secs + 8 * 3600;
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    // Howard Hinnant civil_from_days
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
    format!("{y:04}{m:02}{d:02}{hour:02}{min:02}")
}

/// 对齐 Go 版 signPath(): 返回 (k=timeSign, v="timestamp-random-dataSign")
fn sign_path(path: &str, os: &str, version: &str) -> (String, String) {
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // Go: %.f of math.Round(1e7*rand.Float64()) -> 0..10000000 的整数十进制串
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as u64;
    let random = (nanos.wrapping_mul(2_654_435_761) % 10_000_001).to_string();
    let timestamp = now_unix.to_string();

    // 时间数字串经 table 映射后取 CRC32
    let now_str = beijing_yyyymmddhhmm(now_unix);
    let mapped: Vec<u8> = now_str
        .bytes()
        .map(|b| SIGN_TABLE[(b - b'0') as usize])
        .collect();
    let time_sign = crc32(&mapped).to_string();

    let data = format!("{timestamp}|{random}|{path}|{os}|{version}|{time_sign}");
    let data_sign = crc32(data.as_bytes()).to_string();
    (time_sign, format!("{timestamp}-{random}-{data_sign}"))
}

// ---------- 驱动 ----------

pub struct Pan123 {
    account_id: String,
    http: Client,
    http_no_redirect: Client,
    username: String,
    password: String,
    access_token: Mutex<String>,
    platform: String,
    store: Arc<Store>,
}

impl Pan123 {
    pub fn new(
        account_id: &str,
        username: String,
        password: String,
        access_token: String,
        platform: String,
        store: Arc<Store>,
    ) -> Self {
        Pan123 {
            account_id: account_id.to_string(),
            http: Client::new(),
            http_no_redirect: Client::builder()
                .redirect(redirect::Policy::none())
                .build()
                .unwrap(),
            username,
            password,
            access_token: Mutex::new(access_token),
            platform,
            store,
        }
    }

    fn save_token(&self, token: &str) {
        *self.access_token.lock().unwrap() = token.to_string();
        let token = token.to_string();
        let id = self.account_id.clone();
        self.store.update_credential(&id, |cred| {
            if let Credential::Pan123 { access_token, .. } = cred {
                *access_token = token.clone();
            }
        });
    }

    /// 对齐 Go 版 login(): 账号密码换 token
    pub async fn login(&self) -> Result<(), String> {
        let is_email = self.username.contains('@') && self.username.contains('.');
        let body = if is_email {
            json!({ "mail": self.username, "password": self.password, "type": 2 })
        } else {
            json!({ "passport": self.username, "password": self.password, "remember": true })
        };
        let resp = self
            .http
            .post(SIGN_IN)
            .header("origin", "https://yun.123pan.com")
            .header("referer", "https://yun.123pan.com/")
            .header("user-agent", "Dart/2.19(dart:io)-openlist")
            .header("platform", "web")
            .header("app-version", "3")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("123 登录请求失败: {e}"))?;
        let v: Value = resp.json().await.map_err(|e| format!("登录响应解析失败: {e}"))?;
        let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 200 {
            let msg = v
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            return Err(format!("123 登录失败(code={code}): {msg}"));
        }
        let token = v
            .pointer("/data/token")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        if token.is_empty() {
            return Err("123 登录成功但未返回 token".into());
        }
        self.save_token(&token);
        Ok(())
    }

    /// 对齐 Go 版 GetApi(): 在 query 上追加签名参数
    fn signed_url(&self, path: &str, query: &[(String, String)]) -> String {
        let (k, v) = sign_path(path, "web", "3");
        let mut qs: Vec<String> = query
            .iter()
            .map(|(key, val)| format!("{}={}", urlencoded(key), urlencoded(val)))
            .collect();
        qs.push(format!("{}={}", urlencoded(&k), urlencoded(&v)));
        format!("{}{}?{}", MAIN_API, path, qs.join("&"))
    }

    /// 对齐 Go 版 Request(): 401 自动重登一次
    async fn request(
        &self,
        method: Method,
        path: &str,
        query: Option<&[(String, String)]>,
        body: Option<Value>,
        retried: bool,
    ) -> Result<Value, String> {
        let url = self.signed_url(path, query.unwrap_or(&[]));
        let mut req = self
            .http
            .request(method.clone(), &url)
            .header("origin", "https://yun.123pan.com")
            .header("referer", "https://yun.123pan.com/")
            .header("authorization", format!("Bearer {}", self.access_token.lock().unwrap()))
            .header("user-agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) openlist-client")
            .header("platform", &self.platform)
            .header("app-version", "3");
        if let Some(b) = body.clone() {
            req = req.json(&b);
        }
        let resp = req.send().await.map_err(|e| format!("请求失败: {e}"))?;
        let v: Value = resp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
        let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            if code == 401 && !retried {
                self.login().await?;
                return Box::pin(self.request(method, path, query, body, true)).await;
            }
            let msg = v
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            return Err(format!("123 接口错误(code={code}): {msg}"));
        }
        Ok(v)
    }

    /// 对齐 Go 版 Init(): GET /user/info 验证 token
    pub async fn validate(&self) -> Result<(), String> {
        if self.access_token.lock().unwrap().is_empty() {
            self.login().await?;
        }
        self.request(Method::GET, USER_INFO, None, None, false).await?;
        Ok(())
    }

    /// 对齐 Go 版 getFiles(): GET /file/list/new 分页拉取，700ms 限速
    pub async fn list(&self, parent_fid: &str) -> Result<Vec<Entry>, String> {
        let mut files = Vec::new();
        let mut page: u32 = 1;
        loop {
            if page > 1 {
                tokio::time::sleep(std::time::Duration::from_millis(LIST_RATE_LIMIT_MS)).await;
            }
            let query: Vec<(String, String)> = vec![
                ("driveId".into(), "0".into()),
                ("limit".into(), "100".into()),
                ("next".into(), "0".into()),
                ("orderBy".into(), "file_id".into()),
                ("orderDirection".into(), "desc".into()),
                ("parentFileId".into(), parent_fid.to_string()),
                ("trashed".into(), "false".into()),
                ("SearchData".into(), String::new()),
                ("Page".into(), page.to_string()),
                ("OnlyLookAbnormalFile".into(), "0".into()),
                ("event".into(), "homeListFile".into()),
                ("operateType".into(), "4".into()),
                ("inDirectSpace".into(), "false".into()),
            ];
            let resp = self
                .request(Method::GET, FILE_LIST, Some(&query), None, false)
                .await?;
            let info_list = resp
                .pointer("/data/InfoList")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let next = resp
                .pointer("/data/Next")
                .and_then(|v| v.as_str())
                .unwrap_or("-1")
                .to_string();
            for f in &info_list {
                let ftype = f.get("Type").and_then(|v| v.as_i64()).unwrap_or(0);
                files.push(Entry {
                    fid: f
                        .get("FileId")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0)
                        .to_string(),
                    name: f
                        .get("FileName")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    size: f.get("Size").and_then(|v| v.as_u64()).unwrap_or(0),
                    is_dir: ftype == 1,
                    updated_at: None,
                    etag: f.get("Etag").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    s3_key_flag: f
                        .get("S3KeyFlag")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    file_type: Some(ftype),
                    extra: None,
                });
            }
            if info_list.is_empty() || next == "-1" {
                break;
            }
            page += 1;
        }
        Ok(files)
    }

    /// 对齐 Go 版 Link(): POST /file/download_info -> 跟 302 拿最终直链
    pub async fn download(&self, e: &Entry) -> Result<DownloadInfo, String> {
        let body = json!({
            "driveId": 0,
            "etag": e.etag.clone().unwrap_or_default(),
            "fileId": e.fid.parse::<i64>().unwrap_or(0),
            "fileName": e.name,
            "s3keyFlag": e.s3_key_flag.clone().unwrap_or_default(),
            "size": e.size,
            "type": e.file_type.unwrap_or(0),
        });
        let resp = self
            .request(Method::POST, DOWNLOAD_INFO, None, Some(body), false)
            .await?;
        let mut download_url = resp
            .pointer("/data/DownloadUrl")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if download_url.is_empty() {
            return Err("123 未返回下载直链".into());
        }
        // Go 版：DownloadUrl 的 query 里若带 params=<base64 url>，解码后才是真实地址
        if let Ok(mut parsed) = url::Url::parse(&download_url) {
            if let Some(params) = parsed.query_pairs().find(|(k, _)| k == "params").map(|(_, v)| v.to_string()) {
                if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&params) {
                    if let Ok(real) = String::from_utf8(decoded) {
                        if url::Url::parse(&real).is_ok() {
                            download_url = real;
                            parsed = url::Url::parse(&download_url).unwrap();
                        }
                    }
                }
            }
            let scheme_host = format!("{}://{}/", parsed.scheme(), parsed.host_str().unwrap_or(""));
            // 跟随一次 302 / JSON redirect_url 拿最终直链
            let r = self
                .http_no_redirect
                .get(&download_url)
                .header("Referer", "https://yun.123pan.com/")
                .send()
                .await
                .map_err(|e| format!("获取直链失败: {e}"))?;
            let final_url = match r.status().as_u16() {
                301 | 302 | 303 | 307 | 308 => r
                    .headers()
                    .get("location")
                    .and_then(|l| l.to_str().ok())
                    .unwrap_or(&download_url)
                    .to_string(),
                s if (200..300).contains(&s) => {
                    let v: Value = r.json().await.unwrap_or(Value::Null);
                    v.pointer("/data/redirect_url")
                        .and_then(|x| x.as_str())
                        .unwrap_or(&download_url)
                        .to_string()
                }
                _ => download_url.clone(),
            };
            Ok(DownloadInfo {
                url: final_url,
                headers: vec![("Referer".into(), scheme_host)],
                proxy: false,
            })
        } else {
            Err(format!("123 直链格式异常: {download_url}"))
        }
    }
}

fn urlencoded(s: &str) -> String {
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
