//! Google Drive 驱动（对齐 Go 版 drivers/google_drive，只读浏览 + 下载）
//!
//! - 授权：refresh_token 刷新 access_token（走 olist 在线刷新 API，
//!   对齐本仓库 aliyundrive_open / baidu_netdisk 的做法；Go 版的服务账号
//!   JSON 文件模式不适用本场景，未实现）
//! - 列目录：GET /drive/v3/files?q='parentID' in parents and trashed = false
//!   （pageSize=1000，pageToken 翻页，orderBy 对齐 Go 版默认值；
//!   文件快捷方式回源取目标文件大小等信息）
//! - 下载：GET /drive/v3/files/{id}?alt=media（需 Bearer 头，
//!   Go 版 OnlyProxy，仅支持服务器代理中转）

use super::DownloadInfo;
use super::aliyundrive_open::iso_to_ms;
use crate::config::{Credential, Entry, Store};
use reqwest::{Client, Method};
use serde_json::Value;
use std::sync::{Arc, Mutex};

const API: &str = "https://www.googleapis.com";
/// 对齐 Go 版默认在线刷新 API
const ONLINE_REFRESH_API: &str = "https://api.oplist.org/googleui/renewapi";
/// 对齐 Go 版 FilesListFields
const FILES_LIST_FIELDS: &str = "files(id,name,mimeType,size,modifiedTime,createdTime,thumbnailLink,shortcutDetails,md5Checksum,sha1Checksum,sha256Checksum),nextPageToken";
/// 对齐 Go 版 FileInfoFields（快捷方式回源用）
const FILE_INFO_FIELDS: &str = "id,name,mimeType,size,md5Checksum,sha1Checksum,sha256Checksum";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const SHORTCUT_MIME: &str = "application/vnd.google-apps.shortcut";

pub struct GoogleDrive {
    account_id: String,
    http: Client,
    refresh_token: Mutex<String>,
    access_token: Mutex<String>,
    /// 根目录文件夹 id（对齐 Go 版 RootID），空为 "root"
    root_folder_id: String,
    store: Arc<Store>,
}

impl GoogleDrive {
    pub fn new(
        account_id: &str,
        refresh_token: String,
        access_token: String,
        root_folder_id: String,
        store: Arc<Store>,
    ) -> Self {
        GoogleDrive {
            account_id: account_id.to_string(),
            http: Client::new(),
            refresh_token: Mutex::new(refresh_token),
            access_token: Mutex::new(access_token),
            root_folder_id,
            store,
        }
    }

    fn save_tokens(&self, refresh: &str, access: &str) {
        *self.refresh_token.lock().unwrap() = refresh.to_string();
        *self.access_token.lock().unwrap() = access.to_string();
        let (r, a) = (refresh.to_string(), access.to_string());
        let id = self.account_id.clone();
        self.store.update_credential(&id, |cred| {
            if let Credential::GoogleDrive {
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

    /// 根目录 fid：配置了 root_folder_id 用之，否则 "root"（对齐 Go 版 DefaultRoot）
    fn root_fid(&self) -> String {
        if self.root_folder_id.is_empty() {
            "root".to_string()
        } else {
            self.root_folder_id.clone()
        }
    }

    /// 对齐 Go 版 refreshToken()：olist 在线 API 刷新
    async fn refresh_token(&self) -> Result<(), String> {
        let cur = self.refresh_token.lock().unwrap().clone();
        let url = format!(
            "{}?refresh_ui={}&server_use=true&driver_txt=googleui_go",
            ONLINE_REFRESH_API,
            urlencode(&cur)
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("刷新 Google Drive token 失败: {e}"))?;
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
            return Err(format!("刷新 Google Drive token 失败: {msg}"));
        }
        self.save_tokens(&refresh, &access);
        Ok(())
    }

    /// 对齐 Go 版 request()：401 自动刷新重试一次
    async fn request(
        &self,
        method: Method,
        url: &str,
        params: &[(String, String)],
        retried: bool,
    ) -> Result<Value, String> {
        let resp = self
            .http
            .request(method.clone(), url)
            .header("Authorization", format!("Bearer {}", self.access_token()))
            // 对齐 Go 版 request() 里恒定的两个查询参数
            .query(&[
                ("includeItemsFromAllDrives", "true"),
                ("supportsAllDrives", "true"),
            ])
            .query(params)
            .send()
            .await
            .map_err(|e| format!("请求失败: {e}"))?;
        let status = resp.status();
        let v: Value = resp
            .json()
            .await
            .map_err(|e| format!("响应解析失败: {e}"))?;
        let code = v
            .pointer("/error/code")
            .and_then(|c| c.as_i64())
            .unwrap_or(0);
        if code != 0 {
            if code == 401 && !retried {
                self.refresh_token().await?;
                return Box::pin(self.request(method, url, params, true)).await;
            }
            let msg = v
                .pointer("/error/message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            return Err(format!("Google Drive 接口错误({code}): {msg}"));
        }
        if status.as_u16() >= 400 {
            return Err(format!("Google Drive 接口 HTTP {status}"));
        }
        Ok(v)
    }

    /// 对齐 Go 版 Init()：刷新 token 并验证（取 about 配额信息）
    pub async fn validate(&self) -> Result<(), String> {
        if self.access_token().is_empty() {
            self.refresh_token().await?;
        }
        let url = format!("{API}/drive/v3/about");
        self.request(Method::GET, &url, &[("fields".into(), "storageQuota".into())], false)
            .await?;
        Ok(())
    }

    /// 对齐 Go 版 getFiles()：pageToken 翻页 + 文件快捷方式回源
    pub async fn list(&self, parent_fid: &str) -> Result<Vec<Entry>, String> {
        // 根目录为 "root"（Go 版 DefaultRoot）；旧配置里可能存了兜底值 "0"
        let parent = match parent_fid {
            "" | "0" => self.root_fid(),
            other => other.to_string(),
        };
        let mut files = Vec::new();
        let mut page_token = String::new();
        loop {
            let mut params: Vec<(String, String)> = vec![
                (
                    "orderBy".into(),
                    // 对齐 Go 版默认排序
                    "folder,name,modifiedTime desc".into(),
                ),
                ("fields".into(), FILES_LIST_FIELDS.into()),
                ("pageSize".into(), "1000".into()),
                (
                    "q".into(),
                    format!("'{parent}' in parents and trashed = false"),
                ),
            ];
            if !page_token.is_empty() {
                params.push(("pageToken".into(), page_token.clone()));
            }
            let resp = self
                .request(Method::GET, &format!("{API}/drive/v3/files"), &params, false)
                .await?;
            let mut items = resp
                .get("files")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            page_token = resp
                .get("nextPageToken")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // 文件快捷方式回源（对齐 Go 版 batchGetTargetFilesInfo，顺序请求），
            // 回填目标文件 size 后再统一转换
            for item in &mut items {
                let mime = item
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let target_id = item
                    .pointer("/shortcutDetails/targetId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let target_mime = item
                    .pointer("/shortcutDetails/targetMimeType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if mime == SHORTCUT_MIME && !target_id.is_empty() && target_mime != FOLDER_MIME {
                    if let Ok(target) = self.get_target_file_info(&target_id).await {
                        if let Some(size) = target.get("size").and_then(|v| v.as_str()) {
                            if let Some(obj) = item.as_object_mut() {
                                // API 对文件返回的 size 本就是字符串，直接覆盖
                                obj.insert("size".into(), Value::String(size.to_string()));
                            }
                        }
                    }
                }
            }

            for f in &items {
                files.push(Self::file_to_entry(f));
            }
            if page_token.is_empty() {
                break;
            }
        }
        Ok(files)
    }

    /// 对齐 Go 版 getTargetFileInfo：快捷方式目标文件信息
    async fn get_target_file_info(&self, target_id: &str) -> Result<Value, String> {
        let url = format!("{API}/drive/v3/files/{target_id}");
        self.request(Method::GET, &url, &[("fields".into(), FILE_INFO_FIELDS.into())], false)
            .await
    }

    /// 对齐 Go 版 fileToObj：快捷方式条目的 fid 直接替换为目标 id
    fn file_to_entry(f: &Value) -> Entry {
        let mime = f
            .get("mimeType")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let id = f.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let target_id = f
            .pointer("/shortcutDetails/targetId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let target_mime = f
            .pointer("/shortcutDetails/targetMimeType")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let (fid, is_dir) = if mime == SHORTCUT_MIME && !target_id.is_empty() {
            (target_id, target_mime == FOLDER_MIME)
        } else {
            (id, mime == FOLDER_MIME)
        };
        // Go 版 size 为字符串形式的整数
        let size = f
            .get("size")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .or_else(|| f.get("size").and_then(|v| v.as_u64()))
            .unwrap_or(0);
        let updated_at = f
            .get("modifiedTime")
            .and_then(|v| v.as_str())
            .and_then(iso_to_ms);
        Entry {
            fid,
            name: f.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            size,
            is_dir,
            updated_at,
            etag: f
                .get("md5Checksum")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            s3_key_flag: None,
            file_type: None,
            extra: None,
        }
    }

    /// 对齐 Go 版 Link()：先请求一次元数据验证 token（401 会自动刷新），
    /// 再拼 alt=media 直链（需 Bearer 头，仅代理中转）
    pub async fn download(&self, e: &Entry) -> Result<DownloadInfo, String> {
        let meta_url = format!("{API}/drive/v3/files/{}", urlencode(&e.fid));
        self.request(Method::GET, &meta_url, &[], false).await?;
        let url = format!("{meta_url}&alt=media&acknowledgeAbuse=true");
        Ok(DownloadInfo {
            url,
            headers: vec![(
                "Authorization".into(),
                format!("Bearer {}", self.access_token()),
            )],
            // 浏览器无法附带 Bearer 头，必须经后端 /api/stream 中转（对齐 Go 版 OnlyProxy）
            proxy: true,
            local_path: None,
        })
    }
}

fn urlencode(s: &str) -> String {
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
