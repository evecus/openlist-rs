use super::DownloadInfo;
use crate::config::{Entry, Store};
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const QUARK_API: &str = "https://drive.quark.cn/1/clouddrive";
const QUARK_REFERER: &str = "https://pan.quark.cn";
const QUARK_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) quark-cloud-drive/2.5.20 Chrome/100.0.4896.160 Electron/18.3.5.4-b478491100 Safari/537.36 Channel/pckk_other_ch";
const QUARK_PR: &str = "ucpro";

/// 对齐 Go 版 quark_uc/meta.go：UC 与夸克完全同源，仅域名/UA/pr 不同
pub const UC_CONF: Conf = Conf {
    api: "https://pc-api.uc.cn/1/clouddrive",
    referer: "https://drive.uc.cn",
    ua: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) uc-cloud-drive/2.5.20 Chrome/100.0.4896.160 Electron/18.3.5.4-b478491100 Safari/537.36 Channel/pckk_other_ch",
    pr: "UCBrowser",
};

#[derive(Clone, Copy)]
pub struct Conf {
    pub api: &'static str,
    pub referer: &'static str,
    pub ua: &'static str,
    pub pr: &'static str,
}

impl Default for Conf {
    fn default() -> Self {
        Conf {
            api: QUARK_API,
            referer: QUARK_REFERER,
            ua: QUARK_UA,
            pr: QUARK_PR,
        }
    }
}

/// 对齐 Go 版：file_name 里的 HTML 实体反转义
fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

/// 夸克 / UC 网盘（对齐 Go 版 drivers/quark_uc QuarkOrUC）
pub struct QuarkOrUC {
    account_id: String,
    http: Client,
    conf: Conf,
    cookie: Mutex<String>,
    store: Arc<Store>,
}

impl QuarkOrUC {
    pub fn new(account_id: &str, cookie: String, store: Arc<Store>) -> Self {
        Self::with_conf(account_id, cookie, store, Conf::default())
    }

    pub fn new_uc(account_id: &str, cookie: String, store: Arc<Store>) -> Self {
        Self::with_conf(account_id, cookie, store, UC_CONF)
    }

    pub fn with_conf(account_id: &str, cookie: String, store: Arc<Store>, conf: Conf) -> Self {
        QuarkOrUC {
            account_id: account_id.to_string(),
            http: Client::new(),
            conf,
            cookie: Mutex::new(cookie),
            store,
        }
    }

    fn cookie(&self) -> String {
        self.cookie.lock().unwrap().clone()
    }

    /// 响应 Set-Cookie 里的 __puus 会滚动更新，必须回写，否则旧 cookie 很快失效
    fn absorb_set_cookies(&self, resp: &reqwest::Response) {
        let mut updated: Option<String> = None;
        for v in resp.headers().get_all(reqwest::header::SET_COOKIE) {
            if let Ok(s) = v.to_str() {
                for pair in s.split(';') {
                    let mut it = pair.splitn(2, '=');
                    let k = it.next().unwrap_or("").trim();
                    let val = it.next().unwrap_or("").trim();
                    if k == "__puus" && !val.is_empty() {
                        updated = Some(val.to_string());
                    }
                }
            }
        }
        if let Some(puus) = updated {
            let mut c = self.cookie.lock().unwrap();
            let new_cookie = replace_cookie_value(&c, "__puus", &puus);
            *c = new_cookie;
            let snapshot = c.clone();
            let id = self.account_id.clone();
            let is_uc = self.conf.api == UC_CONF.api;
            self.store.update_credential(&id, |cred| match cred {
                crate::config::Credential::Quark { cookie } => *cookie = snapshot.clone(),
                crate::config::Credential::QuarkUC { cookie } if is_uc => {
                    *cookie = snapshot.clone()
                }
                _ => {}
            });
        }
    }

    /// 统一请求入口，对齐 Go 版 request()
    async fn request(
        &self,
        method: reqwest::Method,
        pathname: &str,
        query: Option<&[(&str, &str)]>,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let url = format!("{}{}", self.conf.api, pathname);
        let mut req = self
            .http
            .request(method, &url)
            .query(&[("pr", self.conf.pr), ("fr", "pc")])
            .header("Cookie", self.cookie())
            .header("Accept", "application/json, text/plain, */*")
            .header("Referer", self.conf.referer)
            .header("User-Agent", self.conf.ua);
        if let Some(q) = query {
            req = req.query(q);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.map_err(|e| format!("请求失败: {e}"))?;
        self.absorb_set_cookies(&resp);
        let status = resp.status();
        let json: Value = resp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
        let code = json.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        if status.as_u16() >= 400 || code != 0 {
            let msg = json
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(format!("接口错误(code={code}): {msg}"));
        }
        Ok(json)
    }

    /// 对齐 Go 版 Init(): GET /config 验证 cookie 有效性
    pub async fn validate(&self) -> Result<(), String> {
        self.request(reqwest::Method::GET, "/config", None, None)
            .await?;
        Ok(())
    }

    /// 对齐 Go 版 GetFiles(): GET /file/sort 分页拉取
    pub async fn list(&self, parent_fid: &str) -> Result<Vec<Entry>, String> {
        let mut files = Vec::new();
        let mut page: u32 = 1;
        let size: u32 = 100;
        loop {
            let query: Vec<(String, String)> = vec![
                ("pdir_fid".into(), parent_fid.to_string()),
                ("_size".into(), size.to_string()),
                ("_page".into(), page.to_string()),
                ("_fetch_total".into(), "1".into()),
                ("fetch_all_file".into(), "1".into()),
                ("fetch_risk_file_name".into(), "1".into()),
            ];
            let refs: Vec<(&str, &str)> =
                query.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let resp = self
                .request(reqwest::Method::GET, "/file/sort", Some(&refs), None)
                .await?;
            let list = resp
                .pointer("/data/list")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for f in &list {
                let name = f
                    .get("file_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                files.push(Entry {
                    fid: f.get("fid").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    name: html_unescape(&name),
                    size: f.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
                    is_dir: !f.get("file").and_then(|v| v.as_bool()).unwrap_or(true),
                    updated_at: f.get("updated_at").and_then(|v| v.as_i64()),
                    etag: None,
                    s3_key_flag: None,
                    file_type: None,
                    extra: None,
                });
            }
            let total = resp
                .pointer("/metadata/_total")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            if page * size >= total || list.is_empty() {
                break;
            }
            page += 1;
        }
        Ok(files)
    }

    /// 对齐 Go 版 getDownloadLink(): POST /file/download
    /// 直链需要带 Cookie/Referer/UA 才能访问，因此标记 proxy
    pub async fn download(&self, e: &Entry) -> Result<DownloadInfo, String> {
        let resp = self
            .request(
                reqwest::Method::POST,
                "/file/download",
                None,
                Some(json!({ "fids": [e.fid] })),
            )
            .await?;
        let url = resp
            .pointer("/data/0/download_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if url.is_empty() {
            return Err("未返回下载直链".into());
        }
        Ok(DownloadInfo {
            url,
            headers: vec![
                ("Cookie".into(), self.cookie()),
                ("Referer".into(), self.conf.referer.into()),
                ("User-Agent".into(), self.conf.ua.into()),
            ],
            proxy: true,
            local_path: None,
        })
    }
}

/// 把 cookie 字符串里 name=value 的 value 替换为 new_value；不存在则追加
fn replace_cookie_value(cookie: &str, name: &str, new_value: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut found = false;
    for pair in cookie.split(';') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let key = pair.split('=').next().unwrap_or("").trim();
        if key == name {
            found = true;
            out.push(format!("{name}={new_value}"));
        } else {
            out.push(pair.to_string());
        }
    }
    if !found {
        out.push(format!("{name}={new_value}"));
    }
    out.join("; ")
}
