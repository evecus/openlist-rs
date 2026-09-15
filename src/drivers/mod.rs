pub mod aliyundrive_open;
pub mod baidu_netdisk;
pub mod google_drive;
pub mod lanzou;
pub mod local;
pub mod onedrive;
pub mod pan115;
pub mod pan123;
pub mod pan139;
pub mod pan189;
pub mod quark;
pub mod share123;
pub mod thunder;
pub mod webdav;
pub mod weiyun;

use crate::config::{Credential, Entry, Store};
use std::sync::Arc;

/// 直链信息：url + 抓取该直链所需请求头 + 是否需要代理
#[derive(Debug, Clone, serde::Serialize)]
pub struct DownloadInfo {
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// true = 浏览器无法直接访问（需要带 Cookie 等头），必须走后端 /api/stream
    pub proxy: bool,
    /// 本机存储专用：非空时后端直接从磁盘流式读取（url 不使用）
    #[serde(skip)]
    pub local_path: Option<String>,
}

pub enum Driver {
    Quark(quark::QuarkOrUC),
    QuarkUC(quark::QuarkOrUC),
    Pan123(pan123::Pan123),
    AliyundriveOpen(aliyundrive_open::AliyundriveOpen),
    BaiduNetdisk(baidu_netdisk::BaiduNetdisk),
    Pan115(pan115::Pan115),
    Thunder(thunder::Thunder),
    Lanzou(lanzou::Lanzou),
    Yun139(pan139::Yun139),
    Cloud189(pan189::Cloud189),
    Local(local::Local),
    Webdav(webdav::Webdav),
    Pan123Share(share123::Pan123Share),
    Weiyun(weiyun::Weiyun),
    Onedrive(onedrive::Onedrive),
    GoogleDrive(google_drive::GoogleDrive),
}

impl Driver {
    /// 由账号构建驱动实例并验证凭据
    pub async fn new(id: &str, cred: &Credential, store: Arc<Store>) -> Result<Self, String> {
        let d = match cred {
            Credential::Quark { cookie } => {
                Driver::Quark(quark::QuarkOrUC::new(id, cookie.clone(), store))
            }
            Credential::QuarkUC { cookie } => {
                Driver::QuarkUC(quark::QuarkOrUC::new_uc(id, cookie.clone(), store))
            }
            Credential::Pan123 {
                username,
                password,
                access_token,
                platform,
            } => Driver::Pan123(pan123::Pan123::new(
                id,
                username.clone(),
                password.clone(),
                access_token.clone(),
                platform.clone(),
                store,
            )),
            Credential::AliyundriveOpen {
                refresh_token,
                access_token,
                alipan_type,
            } => Driver::AliyundriveOpen(aliyundrive_open::AliyundriveOpen::new(
                id,
                refresh_token.clone(),
                access_token.clone(),
                alipan_type.clone(),
                store,
            )),
            Credential::BaiduNetdisk {
                refresh_token,
                access_token,
            } => Driver::BaiduNetdisk(baidu_netdisk::BaiduNetdisk::new(
                id,
                refresh_token.clone(),
                access_token.clone(),
                store,
            )),
            Credential::Pan115 { cookie } => Driver::Pan115(pan115::Pan115::new(cookie.clone())),
            Credential::Thunder {
                username,
                password,
                refresh_token,
                captcha_token,
                device_id,
            } => Driver::Thunder(thunder::Thunder::new(
                id,
                username.clone(),
                password.clone(),
                refresh_token.clone(),
                captcha_token.clone(),
                device_id.clone(),
                store,
            )),
            Credential::Lanzou { cookie, account, password } => Driver::Lanzou(lanzou::Lanzou::new(
                cookie.clone(),
                account.clone(),
                password.clone(),
            )),
            Credential::Yun139 {
                authorization,
                drive_type,
            } => Driver::Yun139(pan139::Yun139::new(
                id,
                authorization.clone(),
                drive_type.clone(),
                store,
            )),
            Credential::Cloud189 { username, password, .. } => {
                Driver::Cloud189(pan189::Cloud189::new(username.clone(), password.clone()))
            }
            Credential::Local { root_path } => {
                Driver::Local(local::Local::new(root_path.clone()))
            }
            Credential::Webdav {
                url,
                username,
                password,
                root_path,
            } => Driver::Webdav(webdav::Webdav::new(
                url.clone(),
                username.clone(),
                password.clone(),
                root_path.clone(),
            )),
            Credential::Pan123Share {
                share_key,
                share_pwd,
                access_token,
            } => Driver::Pan123Share(share123::Pan123Share::new(
                share_key.clone(),
                share_pwd.clone(),
                access_token.clone(),
            )),
            Credential::Weiyun {
                cookies,
                root_folder_id,
            } => Driver::Weiyun(weiyun::Weiyun::new(
                cookies.clone(),
                root_folder_id.clone(),
            )),
            Credential::Onedrive {
                region,
                is_sharepoint,
                site_id,
                root_path,
                refresh_token,
                access_token,
            } => Driver::Onedrive(onedrive::Onedrive::new(
                id,
                region.clone(),
                *is_sharepoint,
                site_id.clone(),
                root_path.clone(),
                refresh_token.clone(),
                access_token.clone(),
                store,
            )),
            Credential::GoogleDrive {
                refresh_token,
                access_token,
                root_folder_id,
            } => Driver::GoogleDrive(google_drive::GoogleDrive::new(
                id,
                refresh_token.clone(),
                access_token.clone(),
                root_folder_id.clone(),
                store,
            )),
        };
        // 统一验证凭据（对齐各驱动 Init()）
        match &d {
            Driver::Quark(x) | Driver::QuarkUC(x) => x.validate().await?,
            Driver::Pan123(x) => x.validate().await?,
            Driver::AliyundriveOpen(x) => x.validate().await?,
            Driver::BaiduNetdisk(x) => x.validate().await?,
            Driver::Pan115(x) => x.validate().await?,
            Driver::Thunder(x) => x.validate().await?,
            Driver::Lanzou(x) => x.validate().await?,
            Driver::Yun139(x) => x.validate().await?,
            Driver::Cloud189(x) => x.validate().await?,
            Driver::Local(x) => x.validate()?,
            Driver::Webdav(x) => x.validate().await?,
            Driver::Pan123Share(x) => x.validate()?,
            Driver::Weiyun(x) => x.validate().await?,
            Driver::Onedrive(x) => x.validate().await?,
            Driver::GoogleDrive(x) => x.validate().await?,
        }
        Ok(d)
    }

    pub async fn list(&self, parent_fid: &str) -> Result<Vec<Entry>, String> {
        match self {
            Driver::Quark(d) | Driver::QuarkUC(d) => d.list(parent_fid).await,
            Driver::Pan123(d) => d.list(parent_fid).await,
            Driver::AliyundriveOpen(d) => d.list(parent_fid).await,
            Driver::BaiduNetdisk(d) => d.list(parent_fid).await,
            Driver::Pan115(d) => d.list(parent_fid).await,
            Driver::Thunder(d) => d.list(parent_fid).await,
            Driver::Lanzou(d) => d.list(parent_fid).await,
            Driver::Yun139(d) => d.list(parent_fid).await,
            Driver::Cloud189(d) => d.list(parent_fid).await,
            Driver::Local(d) => d.list(parent_fid),
            Driver::Webdav(d) => d.list(parent_fid).await,
            Driver::Pan123Share(d) => d.list(parent_fid).await,
            Driver::Weiyun(d) => d.list(parent_fid).await,
            Driver::Onedrive(d) => d.list(parent_fid).await,
            Driver::GoogleDrive(d) => d.list(parent_fid).await,
        }
    }

    pub async fn download(&self, e: &Entry) -> Result<DownloadInfo, String> {
        match self {
            Driver::Quark(d) | Driver::QuarkUC(d) => d.download(e).await,
            Driver::Pan123(d) => d.download(e).await,
            Driver::AliyundriveOpen(d) => d.download(e).await,
            Driver::BaiduNetdisk(d) => d.download(e).await,
            Driver::Pan115(d) => d.download(e).await,
            Driver::Thunder(d) => d.download(e).await,
            Driver::Lanzou(d) => d.download(e).await,
            Driver::Yun139(d) => d.download(e).await,
            Driver::Cloud189(d) => d.download(e).await,
            Driver::Local(d) => d.download(e),
            Driver::Webdav(d) => d.download(e).await,
            Driver::Pan123Share(d) => d.download(e).await,
            Driver::Weiyun(d) => d.download(e).await,
            Driver::Onedrive(d) => d.download(e).await,
            Driver::GoogleDrive(d) => d.download(e).await,
        }
    }
}
