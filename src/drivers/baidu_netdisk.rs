//! 百度网盘驱动（对齐 Go 版 drivers/baidu_netdisk，官方 openapi 通道）
//!
//! - 授权：refresh_token 刷新 access_token（默认走 olist 在线刷新 API）
//! - 列目录：GET /rest/2.0/xpan/file?method=list（按路径，start/limit 分页）
//! - 下载：GET /rest/2.0/xpan/multimedia?method=filemetas&fsids=[..]&dlink=1
//!   拿到 dlink 后追加 access_token 并跟随 302，访问需 UA "pan.baidu.com"

use super::DownloadInfo;
use crate::config::{Credential, Entry, Store};
use reqwest::{Client, ClientBuilder, Method, redirect};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const API: &str = "https://pan.baidu.com/rest/2.0";
const ONLINE_REFRESH_API: &str = "https://api.oplist.org/baiduyun/renewapi";
const DOWNLOAD_UA: &str = "pan.baidu.com";

pub struct BaiduNetdisk {
    account_id: String,
    http: Client,
    http_no_redirect: Client,
    refresh_token: Mutex<String>,
    access_token: Mutex<String>,
    store: Arc<Store>,
}

impl BaiduNetdisk {
    pub fn new(
        account_id: &str,
        refresh_token: String,
        access_token: String,
        store: Arc<Store>,
    ) -> Self {
        BaiduNetdisk {
            account_id: account_id.to_string(),
            http: Client::new(),
            http_no_redirect: ClientBuilder::new()
                .redirect(redirect::Policy::none())
                .build()
                .unwrap(),
            refresh_token: Mutex::new(refresh_token),
            access_token: Mutex::new(access_token),
            store,
        }
    }

    fn save_tokens(&self, refresh: &str, access: &str) {
        *self.refresh_token.lock().unwrap() = refresh.to_string();
        *self.access_token.lock().unwrap() = access.to_string();
        let (r, a) = (refresh.to_string(), access.to_string());
        let id = self.account_id.clone();
        self.store.update_credential(&id, |cred| {
            if let Credential::BaiduNetdisk {
                refresh_token,
                access_token,
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

    /// 对齐 Go 版 refreshToken()：olist 在线 API 刷新（重试一次防空 token）
    async fn refresh_token(&self) -> Result<(), String> {
        for _ in 0..2 {
            let cur = self.refresh_token.lock().unwrap().clone();
            let url = format!(
                "{}?refresh_ui={}&server_use=true&driver_txt=baiduyun_go",
                ONLINE_REFRESH_API,
                urlencode(&cur)
            );
            let resp = self
                .http
                .get(&url)
                .send()
                .await
                .map_err(|e| format!("刷新百度网盘 token 失败: {e}"))?;
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
            if !refresh.is_empty() && !access.is_empty() {
                self.save_tokens(&refresh, &access);
                return Ok(());
            }
        }
        Err("刷新百度网盘 token 失败：refresh_token 可能已失效".into())
    }

    /// 对齐 Go 版 request()：errno 111/-6 刷新 token 后重试
    async fn request(
        &self,
        method: Method,
        url: &str,
        params: &[(String, String)],
        retried: bool,
    ) -> Result<Value, String> {
        let req = self
            .http
            .request(method, url)
            .query(&[("access_token", self.access_token())])
            .query(params);
        let resp = req.send().await.map_err(|e| format!("请求失败: {e}"))?;
        let v: Value = resp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
        let errno = v.get("errno").and_then(|x| x.as_i64()).unwrap_or(-1);
        if errno != 0 {
            if (errno == 111 || errno == -6) && !retried {
                self.refresh_token().await?;
                return Box::pin(self.request(Method::GET, url, params, true)).await;
            }
            return Err(format!("百度网盘接口错误(errno={errno})，详见 https://pan.baidu.com/union/doc/"));
        }
        Ok(v)
    }

    async fn get(&self, pathname: &str, params: &[(String, String)]) -> Result<Value, String> {
        self.request(
            Method::GET,
            &format!("{API}{pathname}"),
            params,
            false,
        )
        .await
    }

    /// 对齐 Go 版 Init()：GET /xpan/nas?method=uinfo 验证 token
    pub async fn validate(&self) -> Result<(), String> {
        if self.access_token().is_empty() {
            self.refresh_token().await?;
        }
        self.get(
            "/xpan/nas",
            &[("method".into(), "uinfo".into())],
        )
        .await?;
        Ok(())
    }

    /// 对齐 Go 版 getFiles()：目录 fid 即网盘路径，start/limit 分页
    pub async fn list(&self, dir: &str) -> Result<Vec<Entry>, String> {
        let mut files = Vec::new();
        let mut start: i64 = 0;
        let limit: i64 = 1000;
        loop {
            let params = vec![
                ("method".into(), "list".into()),
                ("dir".into(), dir.to_string()),
                ("web".into(), "web".into()),
                ("order".into(), "name".into()),
                ("start".into(), start.to_string()),
                ("limit".into(), limit.to_string()),
            ];
            let resp = self.get("/xpan/file", &params).await?;
            let list = resp
                .get("list")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if list.is_empty() {
                break;
            }
            for f in &list {
                let path = f.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let fs_id = f.get("fs_id").and_then(|v| v.as_i64()).unwrap_or(0);
                files.push(Entry {
                    // fid = 完整路径（下载/列目录都以路径为键）
                    fid: path,
                    name: f
                        .get("server_filename")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    size: f.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
                    is_dir: f.get("isdir").and_then(|v| v.as_i64()).unwrap_or(0) == 1,
                    updated_at: f.get("server_mtime").and_then(|v| v.as_i64()).map(|s| s * 1000),
                    etag: None,
                    s3_key_flag: None,
                    file_type: None,
                    // 下载 filemetas 需要 fs_id
                    extra: Some(json!({ "fsid": fs_id.to_string() })),
                });
            }
            if (list.len() as i64) < limit {
                break;
            }
            start += limit;
        }
        Ok(files)
    }

    /// 对齐 Go 版 linkOfficial()
    pub async fn download(&self, e: &Entry) -> Result<DownloadInfo, String> {
        let fsid = e
            .extra
            .as_ref()
            .and_then(|x| x.get("fsid"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if fsid.is_empty() {
            return Err("缺少文件 fs_id（请从文件列表发起下载）".into());
        }
        let params = vec![
            ("method".into(), "filemetas".into()),
            ("fsids".into(), format!("[{fsid}]")),
            ("dlink".into(), "1".into()),
        ];
        let resp = self.get("/xpan/multimedia", &params).await?;
        let dlink = resp
            .pointer("/list/0/dlink")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if dlink.is_empty() {
            return Err("百度网盘未返回 dlink".into());
        }
        let u = format!("{}&access_token={}", dlink, urlencode(&self.access_token()));
        // 跟随一次 302 拿真实直链
        let res = self
            .http_no_redirect
            .head(&u)
            .header("User-Agent", DOWNLOAD_UA)
            .send()
            .await
            .map_err(|e| format!("获取直链失败: {e}"))?;
        let final_url = res
            .headers()
            .get("location")
            .and_then(|l| l.to_str().ok())
            .unwrap_or(&u)
            .to_string();
        Ok(DownloadInfo {
            url: final_url,
            headers: vec![("User-Agent".into(), DOWNLOAD_UA.into())],
            proxy: true,
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
