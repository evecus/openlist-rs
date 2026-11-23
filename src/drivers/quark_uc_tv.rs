//! 夸克 TV / UC TV 驱动（对齐 Go 版 drivers/quark_uc_tv，只读）
//!
//! - 授权：refresh_token + device_id（TV OAuth）
//! - 签名同 quark_open：x-pan-token = sha256(method & path & ts & signKey)
//! - 列目录 / 下载走 open-api-drive.quark.cn 或 open-api-drive.uc.cn

use super::DownloadInfo;
use crate::config::{Credential, Entry, Store};
use md5::Md5;
use reqwest::{Client, Method};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
pub struct TvConf {
    pub api: &'static str,
    pub client_id: &'static str,
    pub sign_key: &'static str,
    pub app_ver: &'static str,
    pub channel: &'static str,
    /// code 换 refresh_token 的第三方中转服务（对齐 Go 版 meta.go）
    pub code_api: &'static str,
}

pub const QUARK_TV: TvConf = TvConf {
    api: "https://open-api-drive.quark.cn",
    client_id: "d3194e61504e493eb6222857bccfed94",
    sign_key: "kw2dvtd7p4t3pjl2d9ed9yc8yej8kw2d",
    app_ver: "1.8.2.2",
    channel: "GENERAL",
    code_api: "http://api.extscreen.com/quarkdrive",
};

pub const UC_TV: TvConf = TvConf {
    api: "https://open-api-drive.uc.cn",
    client_id: "5acf882d27b74502b7040b0c65519aa7",
    sign_key: "l3srvtd7p42l0d0x1u8d7yc8ye9kki4d",
    app_ver: "1.7.2.2",
    channel: "UCTVOFFICIALWEB",
    code_api: "http://api.extscreen.com/ucdrive",
};

/// 官方 TV App 伪装参数（对齐 Go 版 util.go 常量）
const DEVICE_BRAND: &str = "Xiaomi";
const PLATFORM: &str = "tv";
const DEVICE_NAME: &str = "M2004J7AC";
const DEVICE_MODEL: &str = "M2004J7AC";
const BUILD_DEVICE: &str = "M2004J7AC";
const BUILD_PRODUCT: &str = "M2004J7AC";
const DEVICE_GPU: &str = "Adreno (TM) 550";
const ACTIVITY_RECT: &str = "{}";

/// 依据 driver 名取对应配置
pub fn conf_for(driver: &str) -> Option<TvConf> {
    match driver {
        "quark_tv" => Some(QUARK_TV),
        "uc_tv" => Some(UC_TV),
        _ => None,
    }
}

pub struct QuarkUcTv {
    account_id: String,
    http: Client,
    conf: TvConf,
    refresh_token: Mutex<String>,
    access_token: Mutex<String>,
    device_id: Mutex<String>,
    /// download | streaming
    link_method: String,
    store: Arc<Store>,
}

impl QuarkUcTv {
    pub fn new_quark_tv(
        account_id: &str,
        refresh_token: String,
        access_token: String,
        device_id: String,
        link_method: String,
        store: Arc<Store>,
    ) -> Self {
        Self::with_conf(
            account_id,
            refresh_token,
            access_token,
            device_id,
            link_method,
            store,
            QUARK_TV,
        )
    }

    pub fn new_uc_tv(
        account_id: &str,
        refresh_token: String,
        access_token: String,
        device_id: String,
        link_method: String,
        store: Arc<Store>,
    ) -> Self {
        Self::with_conf(
            account_id,
            refresh_token,
            access_token,
            device_id,
            link_method,
            store,
            UC_TV,
        )
    }

    fn with_conf(
        account_id: &str,
        refresh_token: String,
        access_token: String,
        device_id: String,
        link_method: String,
        store: Arc<Store>,
        conf: TvConf,
    ) -> Self {
        QuarkUcTv {
            account_id: account_id.to_string(),
            http: Client::new(),
            conf,
            refresh_token: Mutex::new(refresh_token),
            access_token: Mutex::new(access_token),
            device_id: Mutex::new(device_id),
            link_method: if link_method.is_empty() {
                "download".into()
            } else {
                link_method
            },
            store,
        }
    }

    fn save_tokens(&self, refresh: &str, access: &str, device_id: &str) {
        *self.refresh_token.lock().unwrap() = refresh.to_string();
        *self.access_token.lock().unwrap() = access.to_string();
        *self.device_id.lock().unwrap() = device_id.to_string();
        let (r, a, d) = (
            refresh.to_string(),
            access.to_string(),
            device_id.to_string(),
        );
        let id = self.account_id.clone();
        let is_uc = self.conf.api == UC_TV.api;
        self.store.update_credential(&id, |cred| match cred {
            Credential::QuarkTv {
                refresh_token,
                access_token,
                device_id,
                ..
            } if !is_uc => {
                *refresh_token = r.clone();
                *access_token = a.clone();
                *device_id = d.clone();
            }
            Credential::UcTv {
                refresh_token,
                access_token,
                device_id,
                ..
            } if is_uc => {
                *refresh_token = r.clone();
                *access_token = a.clone();
                *device_id = d.clone();
            }
            _ => {}
        });
    }

    fn generate_req_sign(&self, method: &str, pathname: &str) -> (String, String, String) {
        let tm = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
            .to_string();
        let mut device_id = self.device_id.lock().unwrap().clone();
        if device_id.is_empty() {
            device_id = hex::encode(Md5::digest(tm.as_bytes()));
            *self.device_id.lock().unwrap() = device_id.clone();
        }
        let req_id = hex::encode(Md5::digest(format!("{device_id}{tm}").as_bytes()));
        let token_data = format!("{method}&{pathname}&{tm}&{}", self.conf.sign_key);
        let token = hex::encode(Sha256::digest(token_data.as_bytes()));
        (tm, token, req_id)
    }

    async fn refresh_by_token(&self) -> Result<(), String> {
        let rt = self.refresh_token.lock().unwrap().clone();
        if rt.is_empty() {
            return Err("refresh_token 为空，请先在 TV 端完成扫码授权后填入".into());
        }
        let pathname = "/token";
        let (tm, token, req_id) = self.generate_req_sign("POST", pathname);
        let url = format!("{}{pathname}", self.conf.api);
        let body = json!({
            "client_id": self.conf.client_id,
            "grant_type": "refresh_token",
            "refresh_token": rt,
        });
        let resp = self
            .http
            .post(&url)
            .header("x-pan-tm", &tm)
            .header("x-pan-token", &token)
            .header("x-pan-client-id", self.conf.client_id)
            .query(&[("req_id", req_id.as_str())])
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("刷新 TV token 失败: {e}"))?;
        let v: Value = resp
            .json()
            .await
            .map_err(|e| format!("刷新响应解析失败: {e}"))?;
        let access = v
            .get("access_token")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let refresh = v
            .get("refresh_token")
            .and_then(|x| x.as_str())
            .unwrap_or(&rt)
            .to_string();
        if access.is_empty() {
            let msg = v
                .get("error_info")
                .or_else(|| v.get("error"))
                .and_then(|x| x.as_str())
                .unwrap_or("unknown");
            return Err(format!("刷新 TV token 失败: {msg}"));
        }
        let did = self.device_id.lock().unwrap().clone();
        self.save_tokens(&refresh, &access, &did);
        Ok(())
    }

    async fn request(
        &self,
        method: Method,
        pathname: &str,
        query: Option<&[(&str, String)]>,
        body: Option<Value>,
        retried: bool,
    ) -> Result<Value, String> {
        let (tm, token, req_id) = self.generate_req_sign(method.as_str(), pathname);
        let access = self.access_token.lock().unwrap().clone();
        let url = format!("{}{pathname}", self.conf.api);
        let mut req = self
            .http
            .request(method.clone(), &url)
            .header("x-pan-tm", &tm)
            .header("x-pan-token", &token)
            .header("x-pan-client-id", self.conf.client_id)
            .header("User-Agent", format!("QuarkUCTV/{}", self.conf.app_ver))
            .query(&[
                ("req_id", req_id.as_str()),
                ("access_token", access.as_str()),
            ]);
        if let Some(q) = query {
            for (k, v) in q {
                req = req.query(&[(k, v)]);
            }
        }
        if let Some(b) = &body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| format!("请求失败: {e}"))?;
        let v: Value = resp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
        let status = v.get("status").and_then(|s| s.as_i64()).unwrap_or(0);
        let errno = v.get("errno").and_then(|e| e.as_i64()).unwrap_or(0);
        let err_info = v
            .get("error_info")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_lowercase();
        let token_bad = (status == -1 && (errno == 10001 || errno == 11001))
            || err_info.contains("access token")
            || err_info.contains("access_token")
            || err_info.contains("token无效")
            || err_info.contains("token 无效");
        if token_bad && !retried {
            self.refresh_by_token().await?;
            return Box::pin(self.request(method, pathname, query, body, true)).await;
        }
        if status >= 400 || errno != 0 {
            let msg = v
                .get("error_info")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown");
            return Err(format!("TV 接口错误: {msg}"));
        }
        Ok(v)
    }

    pub async fn validate(&self) -> Result<(), String> {
        if self.access_token.lock().unwrap().is_empty() {
            self.refresh_by_token().await?;
        }
        // 探测列表
        let _ = self
            .request(
                Method::GET,
                "/file",
                Some(&[
                    ("method", "list".into()),
                    ("parent_fid", "0".into()),
                    ("size", "1".into()),
                ]),
                None,
                false,
            )
            .await;
        // 不强制失败（接口路径可能因版本变化），token 刷新成功即可
        Ok(())
    }

    pub async fn list(&self, parent_fid: &str) -> Result<Vec<Entry>, String> {
        let parent = if parent_fid.is_empty() {
            "0"
        } else {
            parent_fid
        };
        let mut files = Vec::new();
        let mut page = 1u32;
        loop {
            let resp = self
                .request(
                    Method::GET,
                    "/file",
                    Some(&[
                        ("method", "list".into()),
                        ("parent_fid", parent.to_string()),
                        ("size", "100".into()),
                        ("page", page.to_string()),
                        ("order_by", "updated_at".into()),
                        ("order_direction", "desc".into()),
                    ]),
                    None,
                    false,
                )
                .await?;
            let list = resp
                .pointer("/data/list")
                .or_else(|| resp.pointer("/data/file_list"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for f in &list {
                let is_dir = f
                    .get("dir")
                    .and_then(|v| v.as_bool())
                    .unwrap_or_else(|| {
                        !f.get("file").and_then(|v| v.as_bool()).unwrap_or(true)
                    });
                files.push(Entry {
                    fid: f
                        .get("fid")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    name: f
                        .get("file_name")
                        .or_else(|| f.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    size: f.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
                    is_dir,
                    updated_at: f.get("updated_at").and_then(|v| v.as_i64()),
                    etag: None,
                    s3_key_flag: None,
                    file_type: None,
                    extra: None,
                });
            }
            if list.len() < 100 {
                break;
            }
            page += 1;
        }
        Ok(files)
    }

    pub async fn download(&self, e: &Entry) -> Result<DownloadInfo, String> {
        if e.is_dir {
            return Err("目录无法下载".into());
        }
        let method = if self.link_method == "streaming" {
            "streaming"
        } else {
            "download"
        };
        let resp = self
            .request(
                Method::GET,
                "/file",
                Some(&[
                    ("method", method.into()),
                    ("fid", e.fid.clone()),
                    ("group_by", "source".into()),
                ]),
                None,
                false,
            )
            .await?;
        let url = resp
            .pointer("/data/download_url")
            .or_else(|| resp.pointer("/data/video_info/0/url"))
            .or_else(|| resp.pointer("/data/url"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if url.is_empty() {
            return Err("TV 未返回下载/播放地址".into());
        }
        // TV 直链为带签名的临时 URL、无任何鉴权头（headers 为空），
        // 外部播放器可直接访问，无需服务器中转。与夸克网盘（需 Cookie/Referer/UA）不同。
        Ok(DownloadInfo {
            url,
            headers: vec![],
            proxy: false,
            local_path: None,
        })
    }

    pub async fn mkdir(&self, _p: &str, _n: &str) -> Result<(), String> {
        Err("夸克/UC TV 为只读驱动，不支持此操作".into())
    }
    pub async fn rename(&self, _p: &str, _e: &Entry, _n: &str) -> Result<(), String> {
        Err("夸克/UC TV 为只读驱动，不支持此操作".into())
    }
    pub async fn move_entry(&self, _p: &str, _e: &Entry, _d: &str) -> Result<(), String> {
        Err("夸克/UC TV 为只读驱动，不支持此操作".into())
    }
    pub async fn copy(&self, _p: &str, _e: &Entry, _d: &str) -> Result<(), String> {
        Err("夸克/UC TV 为只读驱动，不支持此操作".into())
    }
    pub async fn remove(&self, _p: &str, _e: &Entry) -> Result<(), String> {
        Err("夸克/UC TV 为只读驱动，不支持此操作".into())
    }
    pub async fn put(&self, _d: &str, _input: super::PutInput) -> Result<(), String> {
        Err("夸克/UC TV 为只读驱动，不支持此操作".into())
    }
}

// ============================================================================
// 扫码登录（对齐 Go 版 getLoginCode / getCode / getRefreshTokenByTV）
// 纯静态流程，不依赖已存在的账号实例，供 Web 面板「添加存储」时调用。
// ============================================================================

/// 扫码登录会话：持有 device_id（一次扫码流程内保持稳定）
pub struct TvLogin {
    http: Client,
    conf: TvConf,
    device_id: String,
}

/// 第 1 步结果：二维码内容 + 轮询用的 query_token + 本次会话的 device_id
#[derive(serde::Serialize)]
pub struct QrBegin {
    pub qr_data: String,
    pub query_token: String,
    pub device_id: String,
}

/// 第 2 步结果：pending = 尚未扫码/未确认；ok = 已拿到 token
#[derive(serde::Serialize)]
pub struct QrPoll {
    pub status: &'static str, // "pending" | "ok"
    #[serde(skip_serializing_if = "String::is_empty")]
    pub refresh_token: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub access_token: String,
}

impl TvLogin {
    /// 创建扫码会话；device_id 为空则生成一个（对齐 Go 版 generateReqSign 的空值补位）
    pub fn new(conf: TvConf, device_id: String) -> Self {
        let device_id = if device_id.trim().is_empty() {
            Self::new_device_id()
        } else {
            device_id
        };
        TvLogin { http: Client::new(), conf, device_id }
    }

    fn new_device_id() -> String {
        let tm = Self::now_ms();
        hex::encode(Md5::digest(tm.as_bytes()))
    }

    fn now_ms() -> String {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
            .to_string()
    }

    /// 与 QuarkUcTv::generate_req_sign 完全一致（同一签名体系）
    fn sign(&self, method: &str, pathname: &str) -> (String, String, String) {
        let tm = Self::now_ms();
        let req_id = hex::encode(Md5::digest(format!("{}{}", self.device_id, tm).as_bytes()));
        let token_data = format!("{method}&{pathname}&{tm}&{}", self.conf.sign_key);
        let token = hex::encode(Sha256::digest(token_data.as_bytes()));
        (tm, token, req_id)
    }

    /// 官方 API 请求：带全套 TV 伪装参数（对齐 Go 版 request）
    async fn api_request(
        &self,
        method: Method,
        pathname: &str,
        extra_query: &[(&str, String)],
    ) -> Result<Value, String> {
        let (tm, token, req_id) = self.sign(method.as_str(), pathname);
        let url = format!("{}{pathname}", self.conf.api);
        let mut query: Vec<(String, String)> = vec![
            ("req_id".into(), req_id),
            ("access_token".into(), String::new()),
            ("app_ver".into(), self.conf.app_ver.into()),
            ("device_id".into(), self.device_id.clone()),
            ("device_brand".into(), DEVICE_BRAND.into()),
            ("platform".into(), PLATFORM.into()),
            ("device_name".into(), DEVICE_NAME.into()),
            ("device_model".into(), DEVICE_MODEL.into()),
            ("build_device".into(), BUILD_DEVICE.into()),
            ("build_product".into(), BUILD_PRODUCT.into()),
            ("device_gpu".into(), DEVICE_GPU.into()),
            ("activity_rect".into(), ACTIVITY_RECT.into()),
            ("channel".into(), self.conf.channel.into()),
        ];
        for (k, v) in extra_query {
            query.push((k.to_string(), v.clone()));
        }
        let resp = self
            .http
            .request(method, &url)
            .header("Accept", "application/json, text/plain, */*")
            .header("User-Agent", format!("QuarkUCTV/{}", self.conf.app_ver))
            .header("x-pan-tm", &tm)
            .header("x-pan-token", &token)
            .header("x-pan-client-id", self.conf.client_id)
            .query(&query)
            .send()
            .await
            .map_err(|e| format!("请求失败: {e}"))?;
        let v: Value = resp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
        Ok(v)
    }

    /// 第 1 步：获取登录二维码（qr_data 为二维码内容文本）
    pub async fn qr_begin(&self) -> Result<QrBegin, String> {
        let v = self
            .api_request(
                Method::GET,
                "/oauth/authorize",
                &[
                    ("auth_type", "code".into()),
                    ("client_id", self.conf.client_id.into()),
                    ("scope", "netdisk".into()),
                    ("qrcode", "1".into()),
                    ("qr_width", "460".into()),
                    ("qr_height", "460".into()),
                ],
            )
            .await?;
        let data = v.get("data").unwrap_or(&v);
        let qr_data = data.get("qr_data").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let query_token = data.get("query_token").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if qr_data.is_empty() {
            let msg = v.get("error_info").or_else(|| v.get("message"))
                .and_then(|x| x.as_str()).unwrap_or("获取二维码失败");
            return Err(msg.to_string());
        }
        Ok(QrBegin { qr_data, query_token, device_id: self.device_id.clone() })
    }

    /// 第 2 步：轮询 query_token 换 code，再换 refresh_token/access_token
    pub async fn qr_poll(&self, query_token: &str) -> Result<QrPoll, String> {
        if query_token.trim().is_empty() {
            return Err("query_token 为空，请重新获取二维码".into());
        }
        let v = self
            .api_request(
                Method::GET,
                "/oauth/code",
                &[
                    ("client_id", self.conf.client_id.into()),
                    ("scope", "netdisk".into()),
                    ("query_token", query_token.to_string()),
                ],
            )
            .await?;
        let data = v.get("data").unwrap_or(&v);
        let code = data.get("code").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if code.is_empty() {
            return Ok(QrPoll { status: "pending", refresh_token: String::new(), access_token: String::new() });
        }
        let (refresh, access) = self.exchange_code(&code).await?;
        Ok(QrPoll { status: "ok", refresh_token: refresh, access_token: access })
    }

    /// code 或 refresh_token 走第三方中转换 token 对（对齐 Go 版 getRefreshTokenByTV）
    async fn exchange_code(&self, code: &str) -> Result<(String, String), String> {
        let (_, _, req_id) = self.sign("POST", "/token");
        let url = format!("{}/token", self.conf.code_api);
        let body = json!({
            "req_id": req_id,
            "app_ver": self.conf.app_ver,
            "device_id": self.device_id,
            "device_brand": DEVICE_BRAND,
            "platform": PLATFORM,
            "device_name": DEVICE_NAME,
            "device_model": DEVICE_MODEL,
            "build_device": BUILD_DEVICE,
            "build_product": BUILD_PRODUCT,
            "device_gpu": DEVICE_GPU,
            "activity_rect": ACTIVITY_RECT,
            "channel": self.conf.channel,
            "code": code,
        });
        let v: Value = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("换取 token 失败: {e}"))?
            .json()
            .await
            .map_err(|e| format!("token 响应解析失败: {e}"))?;
        if v.get("code").and_then(|x| x.as_i64()).unwrap_or(0) != 200 {
            let msg = v.get("message").and_then(|x| x.as_str()).unwrap_or("token 换取失败");
            return Err(msg.to_string());
        }
        let data = v.get("data").ok_or("token 响应缺少 data")?;
        let refresh = data.get("refresh_token").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let access = data.get("access_token").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if refresh.is_empty() {
            return Err("refresh_token 为空".into());
        }
        Ok((refresh, access))
    }
}
