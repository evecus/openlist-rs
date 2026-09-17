use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

// ---------- 数据模型 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "driver", rename_all = "snake_case")]
pub enum Credential {
    Quark {
        cookie: String,
    },
    Pan123 {
        username: String,
        password: String,
        #[serde(default)]
        access_token: String,
        #[serde(default = "default_platform")]
        platform: String,
    },
    /// 阿里云盘开放平台（refresh_token 授权）
    AliyundriveOpen {
        refresh_token: String,
        #[serde(default)]
        access_token: String,
        #[serde(default = "default_alipan_type")]
        alipan_type: String,
    },
    /// 百度网盘（refresh_token 授权，可用 olist 在线 API 刷新）
    BaiduNetdisk {
        refresh_token: String,
        #[serde(default)]
        access_token: String,
    },
    /// 115 网盘（cookie：UID/CID/SEID）
    Pan115 {
        cookie: String,
    },
    /// 迅雷网盘（账号密码登录）
    Thunder {
        username: String,
        password: String,
        #[serde(default)]
        refresh_token: String,
        #[serde(default)]
        captcha_token: String,
        #[serde(default)]
        device_id: String,
    },
    /// 蓝奏云（account 模式：账号密码自动登录并续期；cookie 模式：仅填 cookie）
    Lanzou {
        #[serde(default)]
        cookie: String,
        #[serde(default)]
        account: String,
        #[serde(default)]
        password: String,
    },
    /// 中国移动云盘（139，Authorization 授权，即 OpenList 139Yun 的 authorization 字段：
    /// base64("Basic xxx:手机号:token|...")，过期后自动刷新）
    Yun139 {
        authorization: String,
        /// personal_new（新版个人云，默认）或 personal（旧版个人云）
        #[serde(default = "default_yun139_type")]
        drive_type: String,
    },
    /// 天翼云盘（189，账号密码登录）
    Cloud189 {
        username: String,
        password: String,
        /// 需要验证码时可手工填充登录后的 cookie（目前实现未使用，保留字段）
        #[serde(default)]
        cookie: String,
    },
    /// UC 网盘（与夸克同源接口，域名不同）
    QuarkUC {
        cookie: String,
    },
    /// 本机存储（root_path 为挂载的本地目录）
    Local {
        root_path: String,
    },
    /// WebDAV（客户端模式，挂载其他 WebDAV 服务器）
    Webdav {
        url: String,
        #[serde(default)]
        username: String,
        #[serde(default)]
        password: String,
        #[serde(default = "default_webdav_root")]
        root_path: String,
    },
    /// 123 云盘分享（shareKey + 分享密码，只读）
    Pan123Share {
        share_key: String,
        #[serde(default)]
        share_pwd: String,
        #[serde(default)]
        access_token: String,
    },
    /// 腾讯微云（www.weiyun.com 登录后的 cookie）
    Weiyun {
        cookies: String,
        /// 根目录 dir_key，留空则自动取账号主目录
        #[serde(default)]
        root_folder_id: String,
    },
    /// OneDrive / SharePoint（对齐 Go 版 drivers/onedrive，refresh_token 授权，
    /// 默认走 olist 在线刷新 API；fid 即网盘内路径）
    Onedrive {
        /// global / cn / us / de（对齐 Go 版 onedriveHostMap）
        #[serde(default = "default_onedrive_region")]
        region: String,
        /// true = SharePoint 站点（需配合 site_id），false = 个人 OneDrive
        #[serde(default)]
        is_sharepoint: bool,
        /// SharePoint 站点 id
        #[serde(default)]
        site_id: String,
        /// 根目录路径（对齐 Go 版 RootFolderPath），默认 "/"
        #[serde(default = "default_onedrive_root")]
        root_path: String,
        refresh_token: String,
        #[serde(default)]
        access_token: String,
    },
    /// Google Drive（对齐 Go 版 drivers/google_drive，refresh_token 授权，
    /// 默认走 olist 在线刷新 API；下载需 Bearer 头，仅支持服务器代理）
    GoogleDrive {
        refresh_token: String,
        #[serde(default)]
        access_token: String,
        /// 根目录文件夹 id（对齐 Go 版 RootID），留空为 "root"
        #[serde(default)]
        root_folder_id: String,
    },
}

fn default_alipan_type() -> String {
    "default".to_string()
}

fn default_platform() -> String {
    "web".to_string()
}

fn default_yun139_type() -> String {
    "personal_new".to_string()
}

fn default_webdav_root() -> String {
    "/".to_string()
}

fn default_onedrive_region() -> String {
    "global".to_string()
}

fn default_onedrive_root() -> String {
    "/".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub cred: Credential,
    #[serde(default = "default_root")]
    pub root_fid: String,
    /// 服务器代理开关：true = 所有下载经本服务中转；false = 直接 302 跳转真实链接
    #[serde(default)]
    pub server_proxy: bool,
}

fn default_root() -> String {
    "0".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub accounts: Vec<Account>,
}

/// 文件条目（驱动无关的统一模型）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Entry {
    pub fid: String,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    /// 毫秒时间戳
    #[serde(default)]
    pub updated_at: Option<i64>,
    // 123pan 下载接口需要的额外字段
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub s3_key_flag: Option<String>,
    #[serde(default)]
    pub file_type: Option<i64>,
    /// 驱动私有扩展字段（下载时需要而 list 才能拿到的信息），
    /// 前端/兼容层需原样透传（如 115 pick_code、百度 fs_id、蓝奏云分享信息等）
    #[serde(default)]
    pub extra: Option<serde_json::Value>,
}

// ---------- 持久化 ----------

pub struct Store {
    path: PathBuf,
    pub data: Mutex<Config>,
}

impl Store {
    pub fn load(path: &str) -> Self {
        let p = PathBuf::from(path);
        let data = std::fs::read(&p)
            .ok()
            .and_then(|b| serde_json::from_slice::<Config>(&b).ok())
            .unwrap_or_default();
        Store {
            path: p,
            data: Mutex::new(data),
        }
    }

    pub fn save(&self, cfg: &Config) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(cfg)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// 驱动回写 cookie / token
    pub fn update_credential(&self, id: &str, f: impl FnOnce(&mut Credential)) {
        let mut data = self.data.lock().unwrap();
        if let Some(acc) = data.accounts.iter_mut().find(|a| a.id == id) {
            f(&mut acc.cred);
            let _ = self.save(&data);
        }
    }
}
