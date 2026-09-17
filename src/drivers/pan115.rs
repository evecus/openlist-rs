//! 115 网盘驱动（对齐 Go 版 drivers/115 + SheltonZhu/115driver v1.3.5）
//!
//! - 授权：cookie（UID/CID/SEID[/KID]）
//! - 列目录：GET https://webapi.115.com/files（offset 分页，单页上限 1150）
//! - 下载：POST https://proapi.115.com/app/chrome/downurl?t={unix秒}
//!   form data 为 m115 加密的 {"pickcode": ...}，响应 data 为 m115 加密的直链映射
//!
//! m115 加密：buf = key(16B) ++ data，三轮变换（xor/reverse/xor）后按
//! RSA-1024 公钥（E=0x10001）分块加密，再 base64。

use super::DownloadInfo;
use crate::config::Entry;
use base64::Engine;
use num_bigint::BigUint;
use reqwest::{Client, Method};
use serde_json::{Value, json};
use std::sync::{Mutex, OnceLock};

const API_FILE_LIST: &str = "https://webapi.115.com/files";
const API_DOWNLOAD: &str = "https://proapi.115.com/app/chrome/downurl";
const API_STATUS_CHECK: &str = "https://my.115.com/?ct=guide&ac=status";
const API_APP_VERSION: &str = "https://appversion.115.com/1/web/1.0/api/chrome";
const FALLBACK_APP_VER: &str = "35.6.0.3";
/// 单页目录上限（对齐 MaxDirPageLimit）
const MAX_PAGE_LIMIT: i64 = 1150;

pub struct Pan115 {
    http: Client,
    cookie: Mutex<String>,
    ua: Mutex<String>,
}

// ---------- m115 加密（对齐 pkg/crypto/m115） ----------

const RSA_N_HEX: &str = "8686980c0f5a24c4b9d43020cd2c22703ff3f450756529058b1cf88f09b86021\
36477198a6e2683149659bd122c33592fdb5ad47944ad1ea4d36c6b172aad633\
8c3bb6ac6227502d010993ac967d1aef00f0c8e038de2e4d3bc2ec368af2e9f1\
0a6f1eda4f7262f136420c07c331b871bf139f74f3010e3c4fe57df3afb71683";
const RSA_E: u32 = 0x10001;
const KEY_LEN: usize = 128;

const XOR_KEY_SEED: [u8; 144] = [
    0xf0, 0xe5, 0x69, 0xae, 0xbf, 0xdc, 0xbf, 0x8a, 0x1a, 0x45, 0xe8, 0xbe, 0x7d, 0xa6, 0x73,
    0xb8, 0xde, 0x8f, 0xe7, 0xc4, 0x45, 0xda, 0x86, 0xc4, 0x9b, 0x64, 0x8b, 0x14, 0x6a, 0xb4,
    0xf1, 0xaa, 0x38, 0x01, 0x35, 0x9e, 0x26, 0x69, 0x2c, 0x86, 0x00, 0x6b, 0x4f, 0xa5, 0x36,
    0x34, 0x62, 0xa6, 0x2a, 0x96, 0x68, 0x18, 0xf2, 0x4a, 0xfd, 0xbd, 0x6b, 0x97, 0x8f, 0x4d,
    0x8f, 0x89, 0x13, 0xb7, 0x6c, 0x8e, 0x93, 0xed, 0x0e, 0x0d, 0x48, 0x3e, 0xd7, 0x2f, 0x88,
    0xd8, 0xfe, 0xfe, 0x7e, 0x86, 0x50, 0x95, 0x4f, 0xd1, 0xeb, 0x83, 0x26, 0x34, 0xdb, 0x66,
    0x7b, 0x9c, 0x7e, 0x9d, 0x7a, 0x81, 0x32, 0xea, 0xb6, 0x33, 0xde, 0x3a, 0xa9, 0x59, 0x34,
    0x66, 0x3b, 0xaa, 0xba, 0x81, 0x60, 0x48, 0xb9, 0xd5, 0x81, 0x9c, 0xf8, 0x6c, 0x84, 0x77,
    0xff, 0x54, 0x78, 0x26, 0x5f, 0xbe, 0xe8, 0x1e, 0x36, 0x9f, 0x34, 0x80, 0x5c, 0x45, 0x2c,
    0x9b, 0x76, 0xd5, 0x1b, 0x8f, 0xcc, 0xc3, 0xb8, 0xf5,
];

const XOR_CLIENT_KEY: [u8; 12] = [
    0x78, 0x06, 0xad, 0x4c, 0x33, 0x86, 0x5d, 0x18, 0x4c, 0x01, 0x3f, 0x46,
];

fn rsa_n() -> &'static BigUint {
    static N: OnceLock<BigUint> = OnceLock::new();
    N.get_or_init(|| BigUint::from_bytes_be(&hex_decode(RSA_N_HEX).unwrap()))
}

fn rsa_e() -> BigUint {
    BigUint::from(RSA_E)
}

/// RSA-1024 PKCS#1 v1.5 type-2 分块加密（对齐 rsaEncrypt）
fn rsa_encrypt(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity((input.len() / (KEY_LEN - 11) + 1) * KEY_LEN);
    let mut input = input;
    while !input.is_empty() {
        let slice_size = (KEY_LEN - 11).min(input.len());
        let slice = &input[..slice_size];
        input = &input[slice_size..];
        // PKCS#1 padding: 0x00 0x02 + non-zero pad + 0x00 + data
        let pad_size = KEY_LEN - slice.len() - 3;
        let mut buf = vec![0u8; KEY_LEN];
        buf[0] = 0;
        buf[1] = 2;
        for b in buf[2..2 + pad_size].iter_mut() {
            // 简单伪随机即可（服务端不校验 padding 内容）
            *b = (pseudo_random_byte() % 0xff) + 0x01;
        }
        buf[2 + pad_size] = 0;
        buf[3 + pad_size..].copy_from_slice(slice);
        let msg = BigUint::from_bytes_be(&buf);
        let ret = msg.modpow(&rsa_e(), rsa_n());
        let ret_bytes = ret.to_bytes_be();
        // 前导补零至 128 字节
        out.extend(std::iter::repeat_n(0, KEY_LEN - ret_bytes.len()));
        out.extend_from_slice(&ret_bytes);
    }
    out
}

/// RSA 公钥指数"解密"（对齐 rsaDecrypt：服务端以私钥方向处理，两侧同用 E）
fn rsa_decrypt(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut input = input;
    while !input.is_empty() {
        let slice_size = KEY_LEN.min(input.len());
        let slice = &input[..slice_size];
        input = &input[slice_size..];
        let msg = BigUint::from_bytes_be(slice);
        let ret = msg.modpow(&rsa_e(), rsa_n());
        let ret = ret.to_bytes_be();
        // unpad：跳过前导零，找第一个 0 字节后的内容
        let mut i = 0;
        while i < ret.len() && ret[i] == 0 {
            i += 1;
        }
        // 此时 i 指向 0x02，继续找分隔 0
        while i < ret.len() {
            if ret[i] == 0 {
                out.extend_from_slice(&ret[i + 1..]);
                break;
            }
            i += 1;
        }
    }
    out
}

fn pseudo_random_byte() -> u8 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    (n ^ (n >> 8) ^ std::process::id()) as u8
}

fn xor_derive_key(seed: &[u8], size: usize) -> Vec<u8> {
    let mut key = vec![0u8; size];
    for i in 0..size {
        // Go 版 (a + b) & 0xff 语义：u8 加法回绕
        key[i] = seed[i].wrapping_add(XOR_KEY_SEED[size * i]);
        key[i] ^= XOR_KEY_SEED[size * (size - i - 1)];
    }
    key
}

fn xor_transform(data: &mut [u8], key: &[u8]) {
    let data_size = data.len();
    let key_size = key.len();
    let mode = data_size % 4;
    for i in 0..mode {
        data[i] ^= key[i % key_size];
    }
    for i in mode..data_size {
        data[i] ^= key[(i - mode) % key_size];
    }
}

fn reverse_bytes(data: &mut [u8]) {
    data.reverse();
}

/// m115 加密（对齐 Encode）
fn m115_encode(input: &[u8], key: &[u8; 16]) -> String {
    let mut buf = Vec::with_capacity(16 + input.len());
    buf.extend_from_slice(key);
    buf.extend_from_slice(input);
    let (head, data) = buf.split_at_mut(16);
    xor_transform(data, &xor_derive_key(head, 4));
    reverse_bytes(data);
    xor_transform(data, &XOR_CLIENT_KEY);
    base64::engine::general_purpose::STANDARD.encode(rsa_encrypt(&buf))
}

/// m115 解密（对齐 Decode）
fn m115_decode(input: &str, key: &[u8; 16]) -> Result<Vec<u8>, String> {
    let data = base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|e| format!("m115 base64 解码失败: {e}"))?;
    let mut data = rsa_decrypt(&data);
    if data.len() < 16 {
        return Err("m115 解密数据过短".into());
    }
    let head: [u8; 16] = data[..16].try_into().unwrap();
    let mut output = data.split_off(16);
    xor_transform(&mut output, &xor_derive_key(&head, 12));
    reverse_bytes(&mut output);
    xor_transform(&mut output, &xor_derive_key(key, 4));
    Ok(output)
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err("hex 长度非法".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

fn random_key() -> [u8; 16] {
    let mut key = [0u8; 16];
    let u1 = uuid::Uuid::new_v4();
    let u2 = uuid::Uuid::new_v4();
    key[..8].copy_from_slice(&u1.as_bytes()[..8]);
    key[8..].copy_from_slice(&u2.as_bytes()[..8]);
    key
}

// ---------- 工具 ----------

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Value 强转字符串（115 接口数字/字符串混用）
fn coerce_str(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// "2024-01-02 15:04"（东八区）-> unix 毫秒
fn parse_115_time(s: &str) -> Option<i64> {
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    if s.len() < 16 {
        return None;
    }
    let y = num(0..4)?;
    let mo = num(5..7)?;
    let d = num(8..10)?;
    let h = num(11..13)?;
    let mi = num(14..16)?;
    let days = days_from_civil(y, mo, d);
    Some((days * 86400 + h * 3600 + mi * 60 - 8 * 3600) * 1000)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

impl Pan115 {
    pub fn new(cookie: String) -> Self {
        Pan115 {
            http: Client::new(),
            cookie: Mutex::new(cookie),
            ua: Mutex::new(format!("Mozilla/5.0 115Browser/{FALLBACK_APP_VER}")),
        }
    }

    fn cookie(&self) -> String {
        self.cookie.lock().unwrap().clone()
    }

    fn ua(&self) -> String {
        self.ua.lock().unwrap().clone()
    }

    /// 对齐 OpenList initAppVer：拉取官方 web 版本号拼 UA
    async fn init_app_ver(&self) {
        let ver = match self.http.get(API_APP_VERSION).send().await {
            Ok(resp) => resp.json::<Value>().await.ok(),
            Err(_) => None,
        }
        .and_then(|v| {
                v.pointer("/data/win/version_code")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string())
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| FALLBACK_APP_VER.to_string());
        *self.ua.lock().unwrap() = format!("Mozilla/5.0 115Browser/{ver}");
    }

    /// 统一请求入口：带 cookie + 115Browser UA
    async fn request(
        &self,
        method: Method,
        url: &str,
        query: Option<&[(String, String)]>,
        form: Option<&[(&str, String)]>,
    ) -> Result<Value, String> {
        let mut req = self
            .http
            .request(method, url)
            .header("Cookie", self.cookie())
            .header("User-Agent", self.ua());
        if let Some(q) = query {
            req = req.query(q);
        }
        if let Some(f) = form {
            req = req.form(f);
        }
        let resp = req.send().await.map_err(|e| format!("请求失败: {e}"))?;
        let status = resp.status();
        let v: Value = resp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
        if status.as_u16() >= 400 {
            return Err(format!("115 接口 HTTP {status}"));
        }
        // 对齐 BasicResp 检查：state 为 false 即出错
        if v.get("state") == Some(&Value::Bool(false)) {
            let msg = v
                .get("error")
                .or_else(|| v.get("msg"))
                .or_else(|| v.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            let errno = coerce_str(v.get("errno"));
            return Err(format!("115 接口错误(errno={errno}): {msg}"));
        }
        Ok(v)
    }

    /// 对齐 login()：校验 cookie 格式并检查登录状态
    pub async fn validate(&self) -> Result<(), String> {
        // cookie 至少要包含 UID/CID/SEID（对齐 Credential.FromCookie）
        let cookie = self.cookie();
        let mut uid = String::new();
        let mut cid = String::new();
        let mut seid = String::new();
        for item in cookie.split(';') {
            let item = item.trim();
            let Some((k, v)) = item.split_once('=') else {
                continue;
            };
            match k.trim().to_uppercase().as_str() {
                "UID" => uid = v.to_string(),
                "CID" => cid = v.to_string(),
                "SEID" => seid = v.to_string(),
                _ => {}
            }
        }
        if uid.is_empty() || cid.is_empty() || seid.is_empty() {
            return Err("115 cookie 无效：缺少 UID/CID/SEID".into());
        }
        self.init_app_ver().await;
        // 对齐 CookieCheck：GET status，state=false 即 cookie 失效
        let url = format!("{API_STATUS_CHECK}&_={}", unix_millis());
        let v = self.request(Method::GET, &url, None, None).await?;
        if v.get("state") != Some(&Value::Bool(true)) {
            return Err("115 cookie 已失效".into());
        }
        Ok(())
    }

    /// 对齐 ListWithLimit + File.from：offset 分页
    pub async fn list(&self, dir_id: &str) -> Result<Vec<Entry>, String> {
        let dir_id = if dir_id.is_empty() { "0" } else { dir_id };
        let mut files = Vec::new();
        let mut offset: i64 = 0;
        let limit: i64 = MAX_PAGE_LIMIT;
        loop {
            let query = vec![
                ("aid".into(), "1".into()),
                ("cid".into(), dir_id.to_string()),
                ("o".into(), "user_ptime".into()),
                ("asc".into(), "0".into()),
                ("offset".into(), offset.to_string()),
                ("show_dir".into(), "1".into()),
                ("limit".into(), limit.to_string()),
                ("snap".into(), "0".into()),
                ("natsort".into(), "0".into()),
                ("record_open_time".into(), "1".into()),
                ("count_folders".into(), "1".into()),
                ("format".into(), "json".into()),
                ("fc_mix".into(), "0".into()),
            ];
            let resp = self
                .request(Method::GET, API_FILE_LIST, Some(&query), None)
                .await?;
            let list = resp
                .get("data")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if list.is_empty() {
                break;
            }
            for f in &list {
                let file_id = coerce_str(f.get("fid"));
                let name = coerce_str(f.get("n"));
                let pick_code = coerce_str(f.get("pc"));
                let (entry_fid, is_dir) = if file_id.is_empty() {
                    // 目录：fid 为空，自身 id 在 cid
                    (coerce_str(f.get("cid")), true)
                } else {
                    (file_id, false)
                };
                let updated_at = if is_dir {
                    // 目录 t 为 unix 秒字符串
                    coerce_str(f.get("t")).parse::<i64>().ok().map(|s| s * 1000)
                } else {
                    // 文件 t 为 "YYYY-MM-DD HH:MM" 东八区字符串
                    coerce_str(f.get("t"))
                        .as_str()
                        .parse::<String>()
                        .ok()
                        .and_then(|_| parse_115_time(&coerce_str(f.get("t"))))
                };
                files.push(Entry {
                    fid: entry_fid,
                    name,
                    size: f.get("s").and_then(|v| v.as_u64()).unwrap_or(0),
                    is_dir,
                    updated_at,
                    etag: None,
                    s3_key_flag: None,
                    file_type: None,
                    // 下载需要 pick_code
                    extra: Some(json!({ "pc": pick_code })),
                });
            }
            let count = resp.get("count").and_then(|v| v.as_i64()).unwrap_or(0);
            offset += limit;
            if offset >= count {
                break;
            }
        }
        Ok(files)
    }

    /// 对齐 DownloadWithUA：m115 加密 pickcode 请求直链
    ///
    /// Go 版关键行为（对齐 buildDownloadHeaders）：
    /// 1. 取链请求本身带 Cookie + User-Agent
    /// 2. 115 服务器会在响应里下发新 Cookie（Set-Cookie），需合并进 info.Header
    /// 3. 下载时必须同时带上这些 Cookie + 相同的 UA，否则 CDN 返回 403
    pub async fn download(&self, e: &Entry) -> Result<DownloadInfo, String> {
        let pick_code = e
            .extra
            .as_ref()
            .and_then(|x| x.get("pc"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if pick_code.is_empty() {
            return Err("缺少 pick_code（请从文件列表发起下载）".into());
        }
        let key = random_key();
        let params = json!({ "pickcode": pick_code }).to_string();
        let encoded = m115_encode(params.as_bytes(), &key);
        let url = format!("{API_DOWNLOAD}?t={}", unix_secs());

        // 直接用 reqwest 发送，需要拿到完整响应头（含 Set-Cookie）
        // 对齐 Go 版 sentRequestHeaders + buildDownloadHeaders
        let ua = self.ua();
        let cookie = self.cookie();
        let raw_resp = self
            .http
            .request(Method::POST, &url)
            .header("Cookie", &cookie)
            .header("User-Agent", &ua)
            .form(&[("data", encoded)])
            .send()
            .await
            .map_err(|e| format!("请求失败: {e}"))?;

        // 收集响应里的 Set-Cookie，合并进下载头（对齐 buildDownloadHeaders）
        let mut extra_cookies: Vec<String> = raw_resp
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .map(|s| {
                // 只取 name=value 部分，去掉 Path/Domain/Expires 等属性
                s.split(';').next().unwrap_or("").trim().to_string()
            })
            .filter(|s| !s.is_empty())
            .collect();

        let v: Value = raw_resp
            .json()
            .await
            .map_err(|e| format!("响应解析失败: {e}"))?;

        if v.get("state") == Some(&Value::Bool(false)) {
            let msg = v
                .get("error")
                .or_else(|| v.get("msg"))
                .or_else(|| v.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            let errno = coerce_str(v.get("errno"));
            return Err(format!("115 接口错误(errno={errno}): {msg}"));
        }

        let encoded_data = match v.get("data") {
            Some(Value::String(s)) => s.clone(),
            _ => return Err("115 未返回有效的下载数据（可能需要验证或 cookie 失效）".into()),
        };
        let decoded = m115_decode(&encoded_data, &key)?;
        let dv: Value =
            serde_json::from_slice(&decoded).map_err(|e| format!("m115 解密结果解析失败: {e}"))?;
        // 响应结构：{ "<file_id>": { "url": { "url": "..." } } }
        let dl_url = dv
            .as_object()
            .and_then(|m| m.values().next())
            .and_then(|info| info.pointer("/url/url"))
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();
        if dl_url.is_empty() {
            return Err("115 未返回下载直链".into());
        }

        // 构建下载请求头（对齐 Go buildDownloadHeaders）：
        // 取链时发送的 Cookie（账号 cookie）+ 响应 Set-Cookie + 相同的 UA
        let mut merged_cookies: Vec<String> = cookie
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        merged_cookies.append(&mut extra_cookies);
        let merged_cookie_str = merged_cookies.join("; ");

        Ok(DownloadInfo {
            url: dl_url,
            headers: vec![
                ("User-Agent".into(), ua),
                ("Cookie".into(), merged_cookie_str),
            ],
            proxy: true,
            local_path: None,
        })
    }
}
