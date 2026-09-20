//! 夸克开放平台驱动（对齐 Go 版 drivers/quark_open）
//!
//! - 授权：refresh_token + app_id + sign_key，access_token 在线刷新
//! - 签名：x-pan-token = sha256(method & pathname & timestamp & signKey)
//! - 列目录：POST /open/v1/file/list
//! - 下载：GET /open/v1/file/download

use super::DownloadInfo;
use crate::config::{Credential, Entry, Store};
use md5::Md5;
use reqwest::{Client, Method};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const API: &str = "https://open-api-drive.quark.cn";
const ONLINE_REFRESH: &str = "https://api.oplist.org/quarkyun/renewapi";
const UA: &str = "go-resty/3.0.0-beta.1 (https://resty.dev)";

pub struct QuarkOpen {
    account_id: String,
    http: Client,
    refresh_token: Mutex<String>,
    access_token: Mutex<String>,
    app_id: String,
    sign_key: String,
    use_online_api: bool,
    api_address: String,
    store: Arc<Store>,
}

impl QuarkOpen {
    pub fn new(
        account_id: &str,
        refresh_token: String,
        access_token: String,
        app_id: String,
        sign_key: String,
        use_online_api: bool,
        api_address: String,
        store: Arc<Store>,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            http: Client::new(),
            refresh_token: Mutex::new(refresh_token),
            access_token: Mutex::new(access_token),
            app_id,
            sign_key,
            use_online_api,
            api_address,
            store,
        }
    }

    fn persist_tokens(&self) {
        let rt = self.refresh_token.lock().unwrap().clone();
        let at = self.access_token.lock().unwrap().clone();
        let _ = self.store.update_account_credential(
            &self.account_id,
            |c| {
                if let Credential::QuarkOpen {
                    refresh_token,
                    access_token,
                    ..
                } = c
                {
                    *refresh_token = rt.clone();
                    *access_token = at.clone();
                }
            },
        );
    }

    async fn refresh(&self) -> Result<(), String> {
        let rt = self.refresh_token.lock().unwrap().clone();
        if rt.is_empty() {
            return Err("refresh_token 为空".into());
        }
        let url = if self.use_online_api && !self.api_address.is_empty() {
            self.api_address.as_str()
        } else if self.use_online_api {
            ONLINE_REFRESH
        } else {
            return Err("需要配置 online api 或手动提供 access_token".into());
        };
        let body = json!({ "refresh_token": rt, "app_id": self.app_id });
        let res: Value = self
            .http
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let at = res
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("refresh 失败: {res}"))?
            .to_string();
        if let Some(nrt) = res.get("refresh_token").and_then(|v| v.as_str()) {
            *self.refresh_token.lock().unwrap() = nrt.to_string();
        }
        *self.access_token.lock().unwrap() = at;
        self.persist_tokens();
        Ok(())
    }

    /// method & pathname & timestamp & signKey -> sha256 hex
    fn sign(&self, method: &str, pathname: &str, ts: &str) -> String {
        let token_data = format!(
            "{}&{}&{}&{}",
            method.to_uppercase(),
            pathname,
            ts,
            self.sign_key
        );
        let hash = Sha256::digest(token_data.as_bytes());
        hex::encode(hash)
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        query: Option<Vec<(&str, String)>>,
        body: Option<Value>,
        auth: bool,
    ) -> Result<Value, String> {
        let ts = chrono::Utc::now().timestamp_millis().to_string();
        let token = self.sign(method.as_str(), path, &ts);
        let mut url = format!("{API}{path}");
        if let Some(q) = &query {
            let qs: Vec<String> = q.iter().map(|(k, v)| format!("{k}={v}")).collect();
            if !qs.is_empty() {
                url.push('?');
                url.push_str(&qs.join("&"));
            }
        }
        let mut req = self
            .http
            .request(method.clone(), &url)
            .header("User-Agent", UA)
            .header("x-pan-tm", &ts)
            .header("x-pan-token", &token)
            .header("x-pan-client-id", &self.app_id);
        if auth {
            let at = self.access_token.lock().unwrap().clone();
            if at.is_empty() {
                self.refresh().await?;
            }
            let at = self.access_token.lock().unwrap().clone();
            req = req.header("Authorization", format!("Bearer {at}"));
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }));
        if !status.is_success() {
            // token 过期尝试刷新一次
            if status.as_u16() == 401 || status.as_u16() == 403 {
                self.refresh().await?;
                return Err(format!("auth failed, refreshed: {v}"));
            }
            return Err(format!("HTTP {status}: {v}"));
        }
        // 业务 code
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
        if self.refresh_token.lock().unwrap().is_empty() && self.access_token.lock().unwrap().is_empty()
        {
            return Err("refresh_token 或 access_token 需要一个".into());
        }
        if self.access_token.lock().unwrap().is_empty() {
            self.refresh().await?;
        }
        // 用列根目录验证
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
            let res = self
                .request(
                    Method::POST,
                    "/open/v1/file/list",
                    None,
                    Some(body),
                    true,
                )
                .await;
            let res = match res {
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
                        true,
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
                let fid = item
                    .get("fid")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = item
                    .get("file_name")
                    .or_else(|| item.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let is_dir = item
                    .get("dir")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
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
        let res = self
            .request(
                Method::GET,
                "/open/v1/file/download",
                Some(vec![("fids", fid.to_string())]),
                None,
                true,
            )
            .await;
        let res = match res {
            Ok(v) => v,
            Err(e) if e.contains("auth failed") => {
                self.request(
                    Method::GET,
                    "/open/v1/file/download",
                    Some(vec![("fids", fid.to_string())]),
                    None,
                    true,
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

    pub async fn mkdir(&self, parent_fid: &str, name: &str) -> Result<(), String> {
        let body = json!({ "pdir_fid": parent_fid, "file_name": name });
        self.request(Method::POST, "/open/v1/file/create", None, Some(body), true)
            .await?;
        Ok(())
    }

    pub async fn rename(&self, _parent: &str, e: &Entry, new_name: &str) -> Result<(), String> {
        let fid = e.id.as_deref().unwrap_or(&e.path);
        let body = json!({ "fid": fid, "file_name": new_name });
        self.request(Method::POST, "/open/v1/file/rename", None, Some(body), true)
            .await?;
        Ok(())
    }

    pub async fn move_entry(&self, _p: &str, e: &Entry, dst: &str) -> Result<(), String> {
        let fid = e.id.as_deref().unwrap_or(&e.path);
        let body = json!({ "filelist": [{"fid": fid}], "to_pdir_fid": dst });
        self.request(Method::POST, "/open/v1/file/move", None, Some(body), true)
            .await?;
        Ok(())
    }

    pub async fn remove(&self, _p: &str, e: &Entry) -> Result<(), String> {
        let fid = e.id.as_deref().unwrap_or(&e.path);
        let body = json!({ "filelist": [{"fid": fid}] });
        self.request(Method::POST, "/open/v1/file/delete", None, Some(body), true)
            .await?;
        Ok(())
    }

    pub async fn put(&self, _dir: &str, _input: super::PutInput) -> Result<(), String> {
        Err("quark_open 暂不支持上传".into())
    }
}

fn _md5_hex(s: &str) -> String {
    hex::encode(Md5::digest(s.as_bytes()))
}
