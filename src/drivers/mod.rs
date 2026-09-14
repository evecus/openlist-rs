pub mod aliyundrive_open;
pub mod baidu_netdisk;
pub mod lanzou;
pub mod pan115;
pub mod pan123;
pub mod quark;
pub mod thunder;

use crate::config::{Credential, Entry, Store};
use std::sync::Arc;

/// 直链信息：url + 抓取该直链所需请求头 + 是否需要代理
#[derive(Debug, Clone, serde::Serialize)]
pub struct DownloadInfo {
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// true = 浏览器无法直接访问（需要带 Cookie 等头），必须走后端 /api/stream
    pub proxy: bool,
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
            Credential::Lanzou { cookie } => Driver::Lanzou(lanzou::Lanzou::new(cookie.clone())),
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
        }
    }
}
