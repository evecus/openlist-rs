//! OneDrive / SharePoint 驱动（对齐 Go 版 drivers/onedrive，只读浏览 + 下载）
//!
//! - 授权：refresh_token 刷新 access_token（默认走 olist 在线刷新 API，
//!   对齐本仓库 aliyundrive_open / baidu_netdisk 的做法，不存 client_id/secret）
//! - 寻址：fid 即网盘内路径（对齐 Go 版按 path 寻址），根目录 = root_path（默认 /）
//! - 列目录：GET /v1.0/{me|sites/{siteId}}/drive/root:/path:/children
//!   （$top=1000，@odata.nextLink 翻页）
//! - 下载：GET 文件元数据取 @microsoft.graph.downloadUrl（预鉴权直链，无需附加头）
//! - region：global / cn / us / de（对齐 Go 版 onedriveHostMap）

use super::DownloadInfo;
use super::aliyundrive_open::iso_to_ms;
use crate::config::{Credential, Entry, Store};
use reqwest::{Client, Method};
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// 对齐 Go 版默认在线刷新 API
const ONLINE_REFRESH_API: &str = "https://api.oplist.org/onedrive/renewapi";
/// children 列表查询参数（对齐 Go 版 getFiles）
const CHILDREN_QUERY: &str =
    "?$top=1000&$expand=thumbnails($select=medium)&$select=id,name,size,fileSystemInfo,content.downloadUrl,file,parentReference";
/// access_token 失效错误码（对齐 Go 版 Request 的 InvalidAuthenticationToken 重试）
const TOKEN_EXPIRED_CODE: &str = "InvalidAuthenticationToken";

pub struct Onedrive {
    account_id: String,
    http: Client,
    region: String,
    is_sharepoint: bool,
    site_id: String,
    root_path: String,
    refresh_token: Mutex<String>,
    access_token: Mutex<String>,
    store: Arc<Store>,
}

/// region -> (oauth host, graph api host)，对齐 Go 版 onedriveHostMap
fn host_of(region: &str) -> (&'static str, &'static str) {
    match region {
        "cn" => (
            "https://login.chinacloudapi.cn",
            "https://microsoftgraph.chinacloudapi.cn",
        ),
        "us" => (
            "https://login.microsoftonline.us",
            "https://graph.microsoft.us",
        ),
        "de" => (
            "https://login.microsoftonline.de",
            "https://graph.microsoft.de",
        ),
        // 默认 global
        _ => (
            "https://login.microsoftonline.com",
            "https://graph.microsoft.com",
        ),
    }
}

impl Onedrive {
    pub fn new(
        account_id: &str,
        region: String,
        is_sharepoint: bool,
        site_id: String,
        root_path: String,
        refresh_token: String,
        access_token: String,
        store: Arc<Store>,
    ) -> Self {
        Onedrive {
            account_id: account_id.to_string(),
            http: Client::new(),
            region,
            is_sharepoint,
            site_id,
            root_path,
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
            if let Credential::Onedrive {
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

    /// 对齐 Go 版 refreshToken()：olist 在线 API 刷新
    async fn refresh_token(&self) -> Result<(), String> {
        let cur = self.refresh_token.lock().unwrap().clone();
        let url = format!(
            "{}?refresh_ui={}&server_use=true&driver_txt=onedrive_pr",
            ONLINE_REFRESH_API,
            urlencode(&cur)
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("刷新 OneDrive token 失败: {e}"))?;
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
            return Err(format!("刷新 OneDrive token 失败: {msg}"));
        }
        self.save_tokens(&refresh, &access);
        Ok(())
    }

    /// 对齐 Go 版 GetMetaUrl(false, path)：按 region + 是否 SharePoint 生成元数据 URL，
    /// path 为 "/" 时返回 drive root 本身，否则返回 root:/path: 形式
    fn meta_url(&self, path: &str) -> String {
        let (_, api) = host_of(&self.region);
        let p = encode_path(path);
        let base = if self.is_sharepoint {
            format!("{api}/v1.0/sites/{}/drive/root", self.site_id)
        } else {
            format!("{api}/v1.0/me/drive/root")
        };
        if p.is_empty() || p == "/" {
            base
        } else {
            format!("{base}:{p}:")
        }
    }

    /// 对齐 Go 版 Request()：InvalidAuthenticationToken 自动刷新重试一次
    async fn request(&self, method: Method, url: &str, retried: bool) -> Result<Value, String> {
        let resp = self
            .http
            .request(method, url)
            .header("Authorization", format!("Bearer {}", self.access_token()))
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
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        if !code.is_empty() {
            if code == TOKEN_EXPIRED_CODE && !retried {
                self.refresh_token().await?;
                return Box::pin(self.request(Method::GET, url, true)).await;
            }
            let msg = v
                .pointer("/error/message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            return Err(format!("OneDrive 接口错误({code}): {msg}"));
        }
        if status.as_u16() >= 400 {
            return Err(format!("OneDrive 接口 HTTP {status}"));
        }
        Ok(v)
    }

    /// 对齐 Go 版 Init()：刷新 token 并验证（取 drive 信息）
    pub async fn validate(&self) -> Result<(), String> {
        if self.is_sharepoint && self.site_id.is_empty() {
            return Err("SharePoint 模式需要填写 site_id".into());
        }
        if self.access_token().is_empty() {
            self.refresh_token().await?;
        }
        let (_, api) = host_of(&self.region);
        let url = if self.is_sharepoint {
            format!("{api}/v1.0/sites/{}/drive", self.site_id)
        } else {
            format!("{api}/v1.0/me/drive")
        };
        self.request(Method::GET, &url, false).await?;
        Ok(())
    }

    /// 对齐 Go 版 getFiles()：children 接口 + @odata.nextLink 翻页，
    /// Entry.fid = 网盘内完整路径（父路径 + 名称）
    pub async fn list(&self, parent_fid: &str) -> Result<Vec<Entry>, String> {
        // 根目录 = root_path（对齐 Go 版 DefaultRoot "/" + RootFolderPath）；
        // 旧配置里可能存了兜底值 "0"
        let dir: String = match parent_fid {
            "" | "0" => self.root_path.clone(),
            other => other.to_string(),
        };
        let mut files = Vec::new();
        // 首个 URL 由路径构造，后续直接使用服务端返回的 nextLink（绝对地址）
        let mut next_link = format!("{}{CHILDREN_QUERY}", self.meta_url(&dir));
        loop {
            let resp = self.request(Method::GET, &next_link, false).await?;
            let items = resp
                .get("value")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for f in &items {
                let name = f
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let updated_at = f
                    .pointer("/fileSystemInfo/lastModifiedDateTime")
                    .and_then(|v| v.as_str())
                    .and_then(iso_to_ms);
                files.push(Entry {
                    fid: join_path(&dir, &name),
                    name,
                    size: f.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
                    // 对齐 Go 版 fileToObj：File 字段存在为文件，否则为文件夹
                    is_dir: f.get("file").is_none(),
                    updated_at,
                    etag: None,
                    s3_key_flag: None,
                    file_type: None,
                    extra: None,
                });
            }
            next_link = resp
                .get("@odata.nextLink")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if next_link.is_empty() {
                break;
            }
        }
        Ok(files)
    }

    /// 对齐 Go 版 Link()：取文件元数据中的 @microsoft.graph.downloadUrl
    pub async fn download(&self, e: &Entry) -> Result<DownloadInfo, String> {
        let f = self
            .request(Method::GET, &self.meta_url(&e.fid), false)
            .await?;
        if f.get("file").is_none() {
            return Err("目标不是文件，无法下载".into());
        }
        let url = f
            .get("@microsoft.graph.downloadUrl")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if url.is_empty() {
            return Err("OneDrive 未返回下载直链".into());
        }
        // downloadUrl 为预鉴权直链，浏览器可直接访问
        Ok(DownloadInfo {
            url,
            headers: vec![],
            proxy: false,
            local_path: None,
        })
    }
}

/// 拼接子路径（fid = 网盘内路径）
fn join_path(dir: &str, name: &str) -> String {
    if dir.is_empty() || dir == "/" {
        format!("/{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// 对齐 Go 版 utils.EncodePath：按路径段做百分号编码（保留 "/"）
fn encode_path(p: &str) -> String {
    p.split('/')
        .map(|seg| {
            let mut out = String::new();
            for b in seg.bytes() {
                match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                        out.push(b as char)
                    }
                    _ => out.push_str(&format!("%{b:02X}")),
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join("/")
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

#[cfg(test)]
mod tests {
    use super::{encode_path, join_path};

    #[test]
    fn test_encode_path() {
        assert_eq!(encode_path("/"), "/");
        assert_eq!(encode_path("/a/b/c.txt"), "/a/b/c.txt");
        // 空格、#、% 需要编码，"/" 保留
        assert_eq!(encode_path("/my docs/a#b%.mp4"), "/my%20docs/a%23b%25.mp4");
        assert_eq!(encode_path("/中文/名.txt"), "/%E4%B8%AD%E6%96%87/%E5%90%8D.txt");
    }

    #[test]
    fn test_join_path() {
        assert_eq!(join_path("/", "a.txt"), "/a.txt");
        assert_eq!(join_path("/docs", "a.txt"), "/docs/a.txt");
    }
}
