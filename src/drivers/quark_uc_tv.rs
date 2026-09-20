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
}

pub const QUARK_TV: TvConf = TvConf {
    api: "https://open-api-drive.quark.cn",
    client_id: "d3194e61504e493eb6222857bccfed94",
    sign_key: "kw2dvtd7p4t3pjl2d9ed9yc8yej8kw2d",
    app_ver: "1.8.2.2",
    channel: "GENERAL",
};

pub const UC_TV: TvConf = TvConf {
    api: "https://open-api-drive.uc.cn",
    client_id: "5acf882d27b74502b7040b0c65519aa7",
    sign_key: "l3srvtd7p42l0d0x1u8d7yc8ye9kki4d",
    app_ver: "1.7.2.2",
    channel: "UCTVOFFICIALWEB",
};

pub struct QuarkUcTv {
    account_id: String,
    http: Client,
    conf: TvConf,
    is_uc: bool,
    refresh_token: Mutex<String>,
    access_token: Mutex<String>,
    device_id: Mutex<String>,
    store: Arc<Store>,
}

impl QuarkUcTv {
    pub fn new(
        account_id: &str,
        conf: TvConf,
        is_uc: bool,
        refresh_token: String,
        access_token: String,
        device_id: String,
        store: Arc<Store>,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            http: Client::new(),
            conf,
            is_uc,
            refresh_token: Mutex::new(refresh_token),
            access_token: Mutex::new(access_token),
            device_id: Mutex::new(device_id),
            store,
        }
    }

    fn persist(&self) {
        let rt = self.refresh_token.lock().unwrap().clone();
        let at = self.access_token.lock().unwrap().clone();
        let did = self.device_id.lock().unwrap().clone();
        let _ = self.store.update_account_credential(&self.account_id, |c| {
            match c {
                Credential::QuarkTv {
                    refresh_token,
                    access_token,
                    device_id,
                    ..
                }
                | Credential::UcTv {
                    refresh_token,
                    access_token,
                    device_id,
                    ..
                } => {
                    *refresh_token = rt.clone();
                    *access_token = at.clone();
                    *device_id = did.clone();
                }
                _ => {}
            }
        });
    }

    async fn refresh(&self) -> Result<(), String> {
        let rt = self.refresh_token.lock().unwrap().clone();
        if rt.is_empty() {
            return Err("refresh_token 为空".into());
        }
        let mut device_id = self.device_id.lock().unwrap().clone();
        let tm = chrono::Utc::now().timestamp_millis().to_string();
        if device_id.is_empty() {
            device_id = hex::encode(Md5::digest(tm.as_bytes()));
            *self.device_id.lock().unwrap() = device_id.clone();
        }
        let req_id = hex::encode(Md5::digest(format!("{device_id}{tm}").as_bytes()));
        let token_data = format!("GET&/token&{tm}&{}", self.conf.sign_key);
        let token = hex::encode(Sha256::digest(token_data.as_bytes()));

        let url = format!("{}/token", self.conf.api);
        let body = json!({
            "req_id": req_id,
            "app_ver": self.conf.app_ver,
            "device_id": device_id,
            "device_brand": "Xiaomi",
            "platform": "tv",
            "channel": self.conf.channel,
            "refresh_token": rt,
        });
        let res: Value = self
            .http
            .post(&url)
            .header("x-pan-tm", &tm)
            .header("x-pan-token", &token)
            .header("x-pan-client-id", self.conf.client_id)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;

        let data = res.get("data").cloned().unwrap_or(res.clone());
        let at = data
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("TV refresh 失败: {res}"))?
            .to_string();
        if let Some(nrt) = data.get("refresh_token").and_then(|v| v.as_str()) {
            *self.refresh_token.lock().unwrap() = nrt.to_string();
        }
        *self.access_token.lock().unwrap() = at;
        self.persist();
        Ok(())
    }

    fn sign(&self, method: &str, path: &str, ts: &str) -> String {
        let token_data = format!(
            "{}&{}&{}&{}",
            method.to_uppercase(),
            path,
            ts,
            self.conf.sign_key
        );
        hex::encode(Sha256::digest(token_data.as_bytes()))
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        query: Option<Vec<(&str, String)>>,
        body: Option<Value>,
    ) -> Result<Value, String> {
        if self.access_token.lock().unwrap().is_empty() {
            self.refresh().await?;
        }
        let ts = chrono::Utc::now().timestamp_millis().to_string();
        let token = self.sign(method.as_str(), path, &ts);
        let mut url = format!("{}{path}", self.conf.api);
        if let Some(q) = &query {
            let qs: Vec<String> = q.iter().map(|(k, v)| format!("{k}={v}")).collect();
            if !qs.is_empty() {
                url.push('?');
                url.push_str(&qs.join("&"));
            }
        }
        let at = self.access_token.lock().unwrap().clone();
        let mut req = self
            .http
            .request(method.clone(), &url)
            .header("x-pan-tm", &ts)
            .header("x-pan-token", &token)
            .header("x-pan-client-id", self.conf.client_id)
            .header("Authorization", format!("Bearer {at}"));
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }));
        if status.as_u16() == 401 || status.as_u16() == 403 {
            self.refresh().await?;
            return Err(format!("auth failed: {v}"));
        }
        if !status.is_success() {
            return Err(format!("HTTP {status}: {v}"));
        }
        if let Some(code) = v.get("code").and_then(|c| c.as_i64()) {
            if code != 0 {
                return Err(format!(
                    "api code={code}: {}",
                    v.get("message").and_then(|m| m.as_str()).unwrap_or("")
                ));
            }
        }
        Ok(v)
    }

    pub async fn validate(&self) -> Result<(), String> {
        if self.refresh_token.lock().unwrap().is_empty()
            && self.access_token.lock().unwrap().is_empty()
        {
            return Err("refresh_token 或 access_token 需要一个".into());
        }
        let _ = self.list("0").await?;
        Ok(())
    }

    pub async fn list(&self, parent_fid: &str) -> Result<Vec<Entry>, String> {
        let mut out = Vec::new();
        let mut page = 1u32;
        loop {
            let body = json!({
                "pdir_fid": parent_fid,
                "_page": page,
                "_size": 100,
            });
            let res = match self
                .request(Method::POST, "/open/v1/file/list", None, Some(body))
                .await
            {
                Ok(v) => v,
                Err(e) if e.contains("auth failed") => {
                    self.request(
                        Method::POST,
                        "/open/v1/file/list",
                        None,
                        Some(json!({
                            "pdir_fid": parent_fid,
                            "_page": page,
                            "_size": 100,
                        })),
                    )
                    .await?
                }
                Err(e) => return Err(e),
            };
            let data = res.get("data").cloned().unwrap_or(Value::Null);
            let list = data
                .get("list")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if list.is_empty() {
                break;
            }
            for item in list {
                let fid = item.get("fid").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = item
                    .get("file_name")
                    .or_else(|| item.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let is_dir = item.get("dir").and_then(|v| v.as_bool()).unwrap_or(false)
                    || item.get("file_type").and_then(|v| v.as_i64()) == Some(0);
                let size = item.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
                let modified = item
                    .get("updated_at")
                    .or_else(|| item.get("l_updated_at"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                out.push(Entry {
                    name,
                    path: fid.clone(),
                    is_dir,
                    size,
                    modified,
                    id: Some(fid),
                    hash: None,
                });
            }
            let total = data.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
            if (out.len() as u64) >= total || page >= 100 {
                break;
            }
            page += 1;
        }
        Ok(out)
    }

    pub async fn download(&self, e: &Entry) -> Result<DownloadInfo, String> {
        let fid = e.id.as_deref().unwrap_or(&e.path);
        let res = match self
            .request(
                Method::GET,
                "/open/v1/file/download",
                Some(vec![("fids", fid.to_string())]),
                None,
            )
            .await
        {
            Ok(v) => v,
            Err(e) if e.contains("auth failed") => {
                self.request(
                    Method::GET,
                    "/open/v1/file/download",
                    Some(vec![("fids", fid.to_string())]),
                    None,
                )
                .await?
            }
            Err(e) => return Err(e),
        };
        let data = res.get("data").cloned().unwrap_or(Value::Null);
        let arr = data.as_array().cloned().unwrap_or_else(|| {
            if data.is_object() {
                vec![data]
            } else {
                vec![]
            }
        });
        let first = arr.first().ok_or("无下载地址".to_string())?;
        let url = first
            .get("download_url")
            .or_else(|| first.get("url"))
            .and_then(|v| v.as_str())
            .ok_or("无 download_url")?
            .to_string();
        Ok(DownloadInfo {
            url,
            headers: vec![],
            filename: Some(e.name.clone()),
            size: if e.size > 0 { Some(e.size) } else { None },
        })
    }

    pub async fn mkdir(&self, _p: &str, _n: &str) -> Result<(), String> {
        Err(format!(
            "{} TV 只读，不支持写入",
            if self.is_uc { "UC" } else { "夸克" }
        ))
    }
    pub async fn rename(&self, _p: &str, _e: &Entry, _n: &str) -> Result<(), String> {
        Err("TV 只读".into())
    }
    pub async fn move_entry(&self, _p: &str, _e: &Entry, _d: &str) -> Result<(), String> {
        Err("TV 只读".into())
    }
    pub async fn remove(&self, _p: &str, _e: &Entry) -> Result<(), String> {
        Err("TV 只读".into())
    }
    pub async fn put(&self, _d: &str, _i: super::PutInput) -> Result<(), String> {
        Err("TV 只读".into())
    }
}
