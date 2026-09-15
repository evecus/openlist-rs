use crate::config::{Account, Credential, Entry};
use crate::drivers::Driver;
use crate::state::AppState;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::Response,
    Json,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

/// 从 query string 中读取 JSON 字符串并反序列化为 Value。
/// axum Query 提取器把 URL 参数当纯字符串，所以需要手动 parse。
fn deserialize_json_str<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    match s {
        None => Ok(None),
        Some(ref raw) if raw.is_empty() => Ok(None),
        Some(raw) => serde_json::from_str(&raw)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

// ---------- 账号管理 ----------

/// 驱动展示名
pub(crate) fn driver_display_name(kind: &str) -> &'static str {
    match kind {
        "quark" => "夸克网盘",
        "quark_uc" => "UC 网盘",
        "123pan" => "123 网盘",
        "aliyundrive_open" => "阿里云盘",
        "baidu_netdisk" => "百度网盘",
        "pan115" => "115 网盘",
        "thunder" => "迅雷网盘",
        "lanzou" => "蓝奏云",
        "pan139" => "移动云盘",
        "cloud189" => "天翼云盘",
        "local" => "本机存储",
        "webdav" => "WebDAV",
        "123pan_share" => "123 分享",
        "weiyun" => "腾讯微云",
        "onedrive" => "OneDrive",
        "google_drive" => "谷歌云盘",
        _ => "网盘",
    }
}

/// 各驱动的默认根目录 fid（对齐 Go 版各驱动 meta.go 的 DefaultRoot）
pub(crate) fn default_root_fid(driver_kind: &str, cred: &Credential) -> String {
    match driver_kind {
        "pan139" => "/".into(),
        "cloud189" => "-11".into(),
        "local" => match cred {
            Credential::Local { root_path } => root_path.clone(),
            _ => String::new(),
        },
        "webdav" => "/".into(),
        "123pan_share" => "0".into(),
        "weiyun" => String::new(),
        // OneDrive 根目录为配置的 root_path（Go 版 DefaultRoot "/" + RootFolderPath）
        "onedrive" => match cred {
            Credential::Onedrive { root_path, .. } => {
                if root_path.is_empty() {
                    "/".into()
                } else {
                    root_path.clone()
                }
            }
            _ => "/".into(),
        },
        // Google Drive 根目录为配置的 root_folder_id（Go 版 DefaultRoot "root"）
        "google_drive" => match cred {
            Credential::GoogleDrive { root_folder_id, .. } => {
                if root_folder_id.is_empty() {
                    "root".into()
                } else {
                    root_folder_id.clone()
                }
            }
            _ => "root".into(),
        },
        // 阿里云盘开放平台根目录固定为 "root"，传 "0" 会 Bad Request
        "aliyundrive_open" => "root".into(),
        "baidu_netdisk" => "/".into(),
        "lanzou" => "-1".into(),
        _ => "0".into(),
    }
}

#[derive(Serialize)]
struct AccountView {
    id: String,
    name: String,
    driver: String,
    server_proxy: bool,
}

/// GET /api/accounts
pub(crate) async fn list_accounts(State(st): State<AppState>) -> Json<Value> {
    let data = st.store.data.lock().unwrap();
    let accounts: Vec<AccountView> = data
        .accounts
        .iter()
        .map(|a| AccountView {
            id: a.id.clone(),
            name: a.name.clone(),
            driver: match a.cred {
                Credential::Quark { .. } => "quark".into(),
                Credential::QuarkUC { .. } => "quark_uc".into(),
                Credential::Pan123 { .. } => "123pan".into(),
                Credential::AliyundriveOpen { .. } => "aliyundrive_open".into(),
                Credential::BaiduNetdisk { .. } => "baidu_netdisk".into(),
                Credential::Pan115 { .. } => "pan115".into(),
                Credential::Thunder { .. } => "thunder".into(),
                Credential::Lanzou { .. } => "lanzou".into(),
                Credential::Yun139 { .. } => "pan139".into(),
                Credential::Cloud189 { .. } => "cloud189".into(),
                Credential::Local { .. } => "local".into(),
                Credential::Webdav { .. } => "webdav".into(),
                Credential::Pan123Share { .. } => "123pan_share".into(),
                Credential::Weiyun { .. } => "weiyun".into(),
                Credential::Onedrive { .. } => "onedrive".into(),
                Credential::GoogleDrive { .. } => "google_drive".into(),
            },
            server_proxy: a.server_proxy,
        })
        .collect();
    Json(json!({ "accounts": accounts }))
}

#[derive(Deserialize)]
pub(crate) struct AddAccountReq {
    name: String,
    /// 网盘类型；缺省时按旧逻辑根据凭据字段推断（兼容旧前端）
    #[serde(default)]
    driver: Option<String>,
    // quark / quark_uc / pan115 / lanzou
    cookie: Option<String>,
    // 123pan / thunder
    username: Option<String>,
    password: Option<String>,
    // aliyundrive_open / baidu_netdisk
    refresh_token: Option<String>,
    // aliyundrive_open 可选：default / alipanTV
    #[serde(default)]
    alipan_type: Option<String>,
    // pan139：139 邮箱 Authorization（base64）
    #[serde(default)]
    authorization: Option<String>,
    // local：本机存储挂载目录
    #[serde(default)]
    root_path: Option<String>,
    // webdav：服务器地址（用户名/密码复用 username/password 字段）
    #[serde(default)]
    url: Option<String>,
    // 123pan_share：分享 key / 分享密码 / 可选 accesstoken
    #[serde(default)]
    share_key: Option<String>,
    #[serde(default)]
    share_pwd: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    // weiyun：登录 cookie
    #[serde(default)]
    cookies: Option<String>,
    // onedrive：region（global/cn/us/de），root_path 复用上面的字段
    #[serde(default)]
    region: Option<String>,
    // onedrive：true = SharePoint 站点模式（需配合 site_id）
    #[serde(default)]
    is_sharepoint: bool,
    // onedrive：SharePoint 站点 id
    #[serde(default)]
    site_id: Option<String>,
    // google_drive：根目录文件夹 id（默认 root）
    #[serde(default)]
    root_folder_id: Option<String>,
    /// 服务器代理开关（默认 false = 直链 302）
    #[serde(default)]
    server_proxy: bool,
}

/// POST /api/accounts —— 构建驱动验证凭据后保存
pub(crate) async fn add_account(
    State(st): State<AppState>,
    Json(req): Json<AddAccountReq>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let cookie = req
        .cookie
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let userpass = match (req.username.as_deref().map(str::trim), req.password.as_deref().map(str::trim)) {
        (Some(u), Some(p)) if !u.is_empty() && !p.is_empty() => Some((u.to_string(), p.to_string())),
        _ => None,
    };
    let refresh = req
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // 显式指定驱动优先，否则按凭据字段推断（旧逻辑）
    let kind = req
        .driver
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            if cookie.is_some() {
                Some("quark".into())
            } else if userpass.is_some() {
                Some("123pan".into())
            } else {
                refresh.clone().map(|_| "aliyundrive_open".into())
            }
        })
        .ok_or((
            StatusCode::BAD_REQUEST,
            "缺少凭据：请按所选网盘类型填写对应参数".to_string(),
        ))?;

    let (driver_kind, cred) = match kind.as_str() {
        "quark" => (
            "quark",
            Credential::Quark {
                cookie: cookie.ok_or((StatusCode::BAD_REQUEST, "夸克需要 cookie".to_string()))?,
            },
        ),
        "quark_uc" | "uc" => (
            "quark_uc",
            Credential::QuarkUC {
                cookie: cookie.ok_or((StatusCode::BAD_REQUEST, "UC 网盘需要 cookie".to_string()))?,
            },
        ),
        "123pan" => {
            let (u, p) = userpass
                .ok_or((StatusCode::BAD_REQUEST, "123 网盘需要账号密码".to_string()))?;
            (
                "123pan",
                Credential::Pan123 {
                    username: u,
                    password: p,
                    access_token: String::new(),
                    platform: "web".into(),
                },
            )
        }
        "aliyundrive_open" | "aliyundrive" | "aliyun" => (
            "aliyundrive_open",
            Credential::AliyundriveOpen {
                refresh_token: refresh.ok_or((
                    StatusCode::BAD_REQUEST,
                    "阿里云盘需要 refresh_token".to_string(),
                ))?,
                access_token: String::new(),
                alipan_type: if req.alipan_type.as_deref() == Some("alipanTV") {
                    "alipanTV".into()
                } else {
                    "default".into()
                },
            },
        ),
        "baidu_netdisk" | "baidu" => (
            "baidu_netdisk",
            Credential::BaiduNetdisk {
                refresh_token: refresh.ok_or((
                    StatusCode::BAD_REQUEST,
                    "百度网盘需要 refresh_token".to_string(),
                ))?,
                access_token: String::new(),
            },
        ),
        "pan115" | "115" => (
            "pan115",
            Credential::Pan115 {
                cookie: cookie.ok_or((
                    StatusCode::BAD_REQUEST,
                    "115 网盘需要 cookie（含 UID/CID/SEID）".to_string(),
                ))?,
            },
        ),
        "thunder" | "xunlei" => {
            let (u, p) = userpass
                .ok_or((StatusCode::BAD_REQUEST, "迅雷网盘需要账号密码".to_string()))?;
            (
                "thunder",
                Credential::Thunder {
                    username: u,
                    password: p,
                    refresh_token: String::new(),
                    captcha_token: String::new(),
                    device_id: String::new(),
                },
            )
        }
        "lanzou" | "lanzouyun" => {
            let account = req.username.clone().unwrap_or_default().trim().to_string();
            let password = req.password.clone().unwrap_or_default();
            let cookie = req.cookie.clone().unwrap_or_default().trim().to_string();
            if account.is_empty() && cookie.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "蓝奏云需要账号密码或 cookie".to_string(),
                ));
            }
            ("lanzou", Credential::Lanzou { cookie, account, password })
        }
        "pan139" | "139yun" | "mobile" => {
            let auth = req
                .authorization
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or((StatusCode::BAD_REQUEST, "移动云盘需要 139 Authorization".to_string()))?;
            ("pan139", Credential::Yun139 {
                authorization: auth.to_string(),
                drive_type: "personal_new".into(),
            })
        }
        "cloud189" | "189" | "tianyi" => {
            let (u, p) = userpass
                .ok_or((StatusCode::BAD_REQUEST, "天翼云盘需要账号密码".to_string()))?;
            ("cloud189", Credential::Cloud189 {
                username: u,
                password: p,
                cookie: String::new(),
            })
        }
        "local" => {
            let root_path = req
                .root_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or((StatusCode::BAD_REQUEST, "本机存储需要挂载目录路径".to_string()))?;
            ("local", Credential::Local { root_path: root_path.to_string() })
        }
        "webdav" => {
            let url = req
                .url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or((StatusCode::BAD_REQUEST, "WebDAV 需要服务器地址".to_string()))?;
            ("webdav", Credential::Webdav {
                url: url.to_string(),
                username: req.username.clone().unwrap_or_default(),
                password: req.password.clone().unwrap_or_default(),
                root_path: req
                    .root_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("/")
                    .to_string(),
            })
        }
        "123pan_share" | "123share" => {
            let share_key = req
                .share_key
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or((StatusCode::BAD_REQUEST, "123 分享需要 shareKey".to_string()))?;
            ("123pan_share", Credential::Pan123Share {
                share_key: share_key.to_string(),
                share_pwd: req.share_pwd.clone().unwrap_or_default(),
                access_token: req.access_token.clone().unwrap_or_default(),
            })
        }
        "weiyun" => {
            let cookies = req
                .cookies
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or((StatusCode::BAD_REQUEST, "腾讯微云需要登录 cookie".to_string()))?;
            ("weiyun", Credential::Weiyun {
                cookies: cookies.to_string(),
                root_folder_id: String::new(),
            })
        }
        "onedrive" | "onedrive_app" => {
            let rt = refresh.ok_or((
                StatusCode::BAD_REQUEST,
                "OneDrive 需要 refresh_token".to_string(),
            ))?;
            ("onedrive", Credential::Onedrive {
                region: req
                    .region
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("global")
                    .to_string(),
                is_sharepoint: req.is_sharepoint,
                site_id: req.site_id.clone().unwrap_or_default(),
                root_path: req
                    .root_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("/")
                    .to_string(),
                refresh_token: rt,
                access_token: String::new(),
            })
        }
        "google_drive" | "google" | "googledrive" => {
            let rt = refresh.ok_or((
                StatusCode::BAD_REQUEST,
                "Google Drive 需要 refresh_token".to_string(),
            ))?;
            ("google_drive", Credential::GoogleDrive {
                refresh_token: rt,
                access_token: String::new(),
                root_folder_id: req.root_folder_id.clone().unwrap_or_default(),
            })
        }
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("不支持的网盘类型: {other}"),
            ))
        }
    };

    // 本机存储的根目录即挂载路径；其余驱动默认根 id 见 default_root_fid
    let root_fid = default_root_fid(driver_kind, &cred);

    let id = uuid::Uuid::new_v4().to_string();
    let driver = Driver::new(&id, &cred, st.store.clone())
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let acc = Account {
        id: id.clone(),
        name: if req.name.trim().is_empty() {
            driver_display_name(driver_kind).to_string()
        } else {
            req.name.trim().to_string()
        },
        cred,
        root_fid,
        server_proxy: req.server_proxy,
    };
    {
        let mut data = st.store.data.lock().unwrap();
        data.accounts.push(acc.clone());
        st.store
            .save(&data)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("保存配置失败: {e}")))?;
    }
    st.drivers.lock().unwrap().insert(id.clone(), Arc::new(driver));

    Ok(Json(json!({ "id": id, "name": acc.name, "driver": driver_kind })))
}

/// GET /api/accounts/{id}/secret —— 返回该账号的凭据明细，仅用于编辑表单回填
///
/// 注意：此接口会返回 cookie / 密码 / refresh_token 等敏感信息，
/// 仅应由已登录的面板管理员在编辑账号时调用（受 /api/accounts 路由已有的鉴权保护）。
pub(crate) async fn get_account_secret(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let data = st.store.data.lock().unwrap();
    let acc = data
        .accounts
        .iter()
        .find(|a| a.id == id)
        .ok_or((StatusCode::NOT_FOUND, "账号不存在".to_string()))?;

    let mut out = json!({
        "id": acc.id,
        "name": acc.name,
        "server_proxy": acc.server_proxy,
    });

    match &acc.cred {
        Credential::Quark { cookie } => {
            out["driver"] = json!("quark");
            out["cookie"] = json!(cookie);
        }
        Credential::QuarkUC { cookie } => {
            out["driver"] = json!("quark_uc");
            out["cookie"] = json!(cookie);
        }
        Credential::Pan123 { username, password, .. } => {
            out["driver"] = json!("123pan");
            out["username"] = json!(username);
            out["password"] = json!(password);
        }
        Credential::AliyundriveOpen { refresh_token, alipan_type, .. } => {
            out["driver"] = json!("aliyundrive_open");
            out["refresh_token"] = json!(refresh_token);
            out["alipan_type"] = json!(alipan_type);
        }
        Credential::BaiduNetdisk { refresh_token, .. } => {
            out["driver"] = json!("baidu_netdisk");
            out["refresh_token"] = json!(refresh_token);
        }
        Credential::Pan115 { cookie } => {
            out["driver"] = json!("pan115");
            out["cookie"] = json!(cookie);
        }
        Credential::Thunder { username, password, .. } => {
            out["driver"] = json!("thunder");
            out["username"] = json!(username);
            out["password"] = json!(password);
        }
        Credential::Lanzou { cookie, account, password } => {
            out["driver"] = json!("lanzou");
            out["cookie"] = json!(cookie);
            out["username"] = json!(account);
            out["password"] = json!(password);
        }
        Credential::Yun139 { authorization, drive_type } => {
            out["driver"] = json!("pan139");
            out["authorization"] = json!(authorization);
            out["drive_type"] = json!(drive_type);
        }
        Credential::Cloud189 { username, password, .. } => {
            out["driver"] = json!("cloud189");
            out["username"] = json!(username);
            out["password"] = json!(password);
        }
        Credential::Local { root_path } => {
            out["driver"] = json!("local");
            out["root_path"] = json!(root_path);
        }
        Credential::Webdav { url, username, password, root_path } => {
            out["driver"] = json!("webdav");
            out["url"] = json!(url);
            out["username"] = json!(username);
            out["password"] = json!(password);
            out["root_path"] = json!(root_path);
        }
        Credential::Pan123Share { share_key, share_pwd, access_token } => {
            out["driver"] = json!("123pan_share");
            out["share_key"] = json!(share_key);
            out["share_pwd"] = json!(share_pwd);
            out["access_token"] = json!(access_token);
        }
        Credential::Weiyun { cookies, root_folder_id } => {
            out["driver"] = json!("weiyun");
            out["cookies"] = json!(cookies);
            out["root_folder_id"] = json!(root_folder_id);
        }
        Credential::Onedrive { region, is_sharepoint, site_id, root_path, refresh_token, .. } => {
            out["driver"] = json!("onedrive");
            out["region"] = json!(region);
            out["is_sharepoint"] = json!(is_sharepoint);
            out["site_id"] = json!(site_id);
            out["root_path"] = json!(root_path);
            out["refresh_token"] = json!(refresh_token);
        }
        Credential::GoogleDrive { refresh_token, root_folder_id, .. } => {
            out["driver"] = json!("google_drive");
            out["refresh_token"] = json!(refresh_token);
            out["root_folder_id"] = json!(root_folder_id);
        }
    }

    Ok(Json(out))
}

/// DELETE /api/accounts/{id}
pub(crate) async fn del_account(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    {
        let mut data = st.store.data.lock().unwrap();
        let before = data.accounts.len();
        data.accounts.retain(|a| a.id != id);
        if data.accounts.len() == before {
            return Err((StatusCode::NOT_FOUND, "账号不存在".into()));
        }
        st.store
            .save(&data)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("保存配置失败: {e}")))?;
    }
    st.drivers.lock().unwrap().remove(&id);
    // 账号已删除，清空其全部目录缓存
    st.list_cache.invalidate_account(&id);
    Ok(Json(json!({ "ok": true })))
}

// ---------- 文件浏览 / 下载 ----------

#[derive(Deserialize)]
pub(crate) struct FilesQuery {
    pub(crate) account: String,
    #[serde(default)]
    pub(crate) fid: Option<String>,
    /// refresh=true 时跳过缓存，直接请求网盘并用结果覆盖缓存
    #[serde(default)]
    pub(crate) refresh: Option<bool>,
}

/// GET /api/files?account=&fid=&refresh=
///
/// 缓存策略（参考 OpenList）：
/// - 默认先查内存缓存（TTL 10 分钟），命中直接返回，不请求网盘
/// - refresh=true 跳过缓存，强制请求网盘并覆盖缓存（前端刷新按钮）
/// - 空结果不缓存，避免刚上传的文件在缓存期内不可见
pub(crate) async fn list_files(
    State(st): State<AppState>,
    Query(q): Query<FilesQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let driver = st
        .get_driver(&q.account)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let fid = q.fid.unwrap_or_else(|| "0".into());
    let refresh = q.refresh.unwrap_or(false);
    let cache_key = format!("{}:{}", q.account, fid);

    // 命中缓存直接返回（refresh 请求跳过）
    if !refresh {
        if let Some(cached) = st.list_cache.get(&cache_key) {
            let mut entries = cached;
            sort_entries(&mut entries);
            return Ok(Json(json!({ "entries": entries })));
        }
    }

    let mut entries = driver
        .list(&fid)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    sort_entries(&mut entries);
    if !entries.is_empty() {
        st.list_cache.set(&cache_key, entries.clone());
    }
    Ok(Json(json!({ "entries": entries })))
}

pub(crate) fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

#[derive(Deserialize)]
pub(crate) struct FileQuery {
    pub(crate) account: String,
    pub(crate) fid: String,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) size: Option<u64>,
    #[serde(default)]
    pub(crate) etag: Option<String>,
    #[serde(default)]
    pub(crate) s3key: Option<String>,
    #[serde(default)]
    pub(crate) ftype: Option<i64>,
    #[serde(default)]
    pub(crate) isdir: Option<bool>,
    /// 驱动私有扩展信息（115 pick_code、百度 fs_id 等），原样透传
    /// query string 中为 JSON 字符串，需自定义反序列化
    #[serde(default, deserialize_with = "deserialize_json_str")]
    pub(crate) extra: Option<serde_json::Value>,
    /// inline=在线播放（Content-Disposition: inline），默认附件下载
    #[serde(default)]
    pub(crate) disp: Option<String>,
}

impl FileQuery {
    fn to_entry(&self) -> Entry {
        Entry {
            fid: self.fid.clone(),
            name: self.name.clone().unwrap_or_default(),
            size: self.size.unwrap_or(0),
            is_dir: self.isdir.unwrap_or(false),
            updated_at: None,
            etag: self.etag.clone(),
            s3_key_flag: self.s3key.clone(),
            file_type: self.ftype,
            extra: self.extra.clone(),
        }
    }
}

/// GET /api/download —— 取直链（返回 url + 是否需代理）
///
/// 代理决策优先级：
/// 1. 账号开启了 server_proxy → 强制走本服务中转
/// 2. 驱动返回 proxy=true（如 115 网盘直链与 UA 绑定）→ 强制走本服务中转
/// 3. 其余情况 → 直链 302，客户端直接访问网盘服务器
pub(crate) async fn get_download(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<FileQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // 取账号的 server_proxy 开关
    let account_server_proxy = {
        let data = st.store.data.lock().unwrap();
        data.accounts
            .iter()
            .find(|a| a.id == q.account)
            .map(|a| a.server_proxy)
            .unwrap_or(false)
    };

    let driver = st
        .get_driver(&q.account)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let info = driver
        .download(&q.to_entry())
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    // 需要代理的条件：账号开关 OR 驱动要求（如 115 UA 绑定直链）
    if account_server_proxy || info.proxy {
        // 将 url 重写为本服务的 /api/stream 端点，让客户端通过本服务中转
        let host = headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("127.0.0.1");
        let proto = headers
            .get("x-forwarded-proto")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("http");
        // 构造 /api/stream 查询参数，原样透传文件元信息（用 form_urlencoded 编码）
        let mut qs = url::form_urlencoded::Serializer::new(String::new());
        qs.append_pair("account", &q.account);
        qs.append_pair("fid", &q.fid);
        qs.append_pair("name", q.name.as_deref().unwrap_or(""));
        qs.append_pair("size", &q.size.unwrap_or(0).to_string());
        qs.append_pair("disp", "inline");
        if let Some(ref etag) = q.etag { qs.append_pair("etag", etag); }
        if let Some(ref s3k) = q.s3key { qs.append_pair("s3key", s3k); }
        if let Some(ft) = q.ftype { qs.append_pair("ftype", &ft.to_string()); }
        if let Some(ref extra) = q.extra {
            qs.append_pair("extra", &extra.to_string());
        }
        let stream_url = format!("{proto}://{host}/api/stream?{}", qs.finish());
        Ok(Json(json!({
            "url": stream_url,
            "proxy": true,
        })))
    } else {
        Ok(Json(json!({
            "url": info.url,
            "proxy": false,
        })))
    }
}

/// 共享的流式代理：拉直链 -> 转发 Range -> 回写头 + Content-Disposition
/// 本机存储例外：local_path 非空时直接从磁盘流式读取，不走 reqwest
pub(crate) async fn proxy_stream(
    driver: &Arc<Driver>,
    e: &Entry,
    headers: &HeaderMap,
    disp: &str,
) -> Result<Response, (StatusCode, String)> {
    let info = driver
        .download(e)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("获取直链失败: {e}")))?;

    if let Some(path) = &info.local_path {
        return serve_local_file(path, e, headers, disp).await;
    }

    // 115 等网盘的直链需要携带特定 User-Agent，且可能经过 HTTP 重定向。
    // reqwest 默认重定向时会剥离非标准 header（包括自定义 User-Agent），
    // 导致 115 服务器在跳转后返回 403 / 无效响应。
    // 解决方案：禁用自动重定向，手动跟随，每一跳都附带完整自定义头。
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default();

    // 手动跟随重定向（最多 10 跳），每跳都带上 info.headers
    let range_val = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let mut current_url = info.url.clone();
    let mut redirect_count = 0usize;
    let upstream = loop {
        if redirect_count > 10 {
            return Err((StatusCode::BAD_GATEWAY, "重定向次数超限".into()));
        }
        let mut req = client.get(&current_url);
        for (k, v) in &info.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(ref range) = range_val {
            req = req.header(header::RANGE, range.as_str());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, format!("拉取直链失败: {e}")))?;
        let status = resp.status().as_u16();
        if status == 301 || status == 302 || status == 307 || status == 308 {
            let location = resp
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
                .ok_or_else(|| (StatusCode::BAD_GATEWAY, "重定向缺少 Location".to_string()))?;
            current_url = location;
            redirect_count += 1;
            continue;
        }
        break resp;
    };
    let status = upstream.status();
    let mut resp_builder = Response::builder()
        .status(StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK));
    for h in [
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::CONTENT_TYPE,
        header::ACCEPT_RANGES,
        header::ETAG,
        header::LAST_MODIFIED,
    ] {
        if let Some(v) = upstream.headers().get(&h) {
            resp_builder = resp_builder.header(&h, v);
        }
    }
    // Content-Disposition：inline 在线播放 / attachment 附件下载
    let fname = if e.name.is_empty() { "download" } else { &e.name };
    let encoded: String = fname
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    resp_builder = resp_builder.header(
        header::CONTENT_DISPOSITION,
        format!("{disp}; filename*=UTF-8''{encoded}"),
    );

    let stream = upstream.bytes_stream();
    let body = Body::from_stream(stream);
    resp_builder
        .body(body)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("构建响应失败: {e}")))
}

/// 本机存储流式读取：支持 Range 断点/拖动播放（对齐 proxy_stream 的响应头行为）
async fn serve_local_file(
    path: &str,
    e: &Entry,
    headers: &HeaderMap,
    disp: &str,
) -> Result<Response, (StatusCode, String)> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let file = tokio::fs::File::open(path).await.map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            (StatusCode::NOT_FOUND, "文件不存在".into())
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("打开文件失败: {err}"))
        }
    })?;
    let total = file
        .metadata()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("读取文件信息失败: {e}")))?
        .len();

    // 解析 Range: bytes=start-end / bytes=start- / bytes=-suffix
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_range)
        .map(|(start, end)| (start.min(total.saturating_sub(1)), end.min(total.saturating_sub(1))))
        .filter(|(start, end)| start <= end && *start < total);

    let (status, start, end, content_length) = match range {
        Some((start, end)) => (StatusCode::PARTIAL_CONTENT, start, end, end - start + 1),
        None => (StatusCode::OK, 0u64, total.saturating_sub(1), total),
    };

    let mut file = file;
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("定位文件失败: {e}")))?;
    let stream = tokio_util::io::ReaderStream::with_capacity(
        file.take(content_length),
        64 * 1024,
    );

    let mut resp = Response::builder().status(status);
    if status == StatusCode::PARTIAL_CONTENT {
        resp = resp.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{total}"),
        );
    }
    resp = resp
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, content_length)
        .header(header::CONTENT_TYPE, content_type_by_ext(&e.name));

    // Content-Disposition：inline 在线播放 / attachment 附件下载
    let fname = if e.name.is_empty() { "download" } else { &e.name };
    let encoded: String = fname
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    resp = resp.header(
        header::CONTENT_DISPOSITION,
        format!("{disp}; filename*=UTF-8''{encoded}"),
    );

    resp.body(Body::from_stream(stream))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("构建响应失败: {e}")))
}

/// 解析 "bytes=start-end" 形式的 Range 头（不含多区间）
fn parse_range(s: &str) -> Option<(u64, u64)> {
    let rest = s.strip_prefix("bytes=")?;
    // 仅取第一个区间
    let first = rest.split(',').next()?.trim();
    let (start_s, end_s) = first.split_once('-')?;
    let total_max = u64::MAX;
    match (start_s.trim().parse::<u64>(), end_s.trim().parse::<u64>()) {
        (Ok(start), Ok(end)) if start <= end => Some((start, end)),
        (Ok(start), Err(_)) => Some((start, total_max)),
        // bytes=-suffix：最后 suffix 字节，start 待调用方结合 total 修正（这里先返回 0 占位）
        (Err(_), Ok(suffix)) if suffix > 0 => Some((total_max.saturating_sub(suffix), total_max)),
        _ => None,
    }
}

/// 按扩展名粗略推断 Content-Type（在线播放场景）
fn content_type_by_ext(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "mov" => "video/quicktime",
        "flv" => "video/x-flv",
        "ts" => "video/mp2t",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" | "opus" => "audio/ogg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "md" | "log" | "srt" | "ass" | "vtt" => "text/plain; charset=utf-8",
        "html" | "htm" => "text/html; charset=utf-8",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

/// GET /api/stream —— 面板用流式下载/播放
pub(crate) async fn stream_file(
    State(st): State<AppState>,
    Query(q): Query<FileQuery>,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, String)> {
    let driver = st
        .get_driver(&q.account)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let disp = if q.disp.as_deref() == Some("inline") {
        "inline"
    } else {
        "attachment"
    };
    proxy_stream(&driver, &q.to_entry(), &headers, disp).await
}

/// PUT /api/accounts/{id} —— 编辑已有账号（替换凭据 + 名称 + server_proxy）
///
/// 流程：先删旧驱动缓存，用新凭据重新验证，验证成功后原地替换配置，保持 id 不变。
pub(crate) async fn edit_account(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<AddAccountReq>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // 确认账号存在
    {
        let data = st.store.data.lock().unwrap();
        if !data.accounts.iter().any(|a| a.id == id) {
            return Err((StatusCode::NOT_FOUND, "账号不存在".into()));
        }
    }

    let cookie = req
        .cookie
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let userpass = match (req.username.as_deref().map(str::trim), req.password.as_deref().map(str::trim)) {
        (Some(u), Some(p)) if !u.is_empty() && !p.is_empty() => Some((u.to_string(), p.to_string())),
        _ => None,
    };
    let refresh = req
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let kind = req
        .driver
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or((StatusCode::BAD_REQUEST, "缺少网盘类型".to_string()))?;

    let (driver_kind, cred) = match kind.as_str() {
        "quark" => (
            "quark",
            Credential::Quark {
                cookie: cookie.ok_or((StatusCode::BAD_REQUEST, "夸克需要 cookie".to_string()))?,
            },
        ),
        "quark_uc" | "uc" => (
            "quark_uc",
            Credential::QuarkUC {
                cookie: cookie.ok_or((StatusCode::BAD_REQUEST, "UC 网盘需要 cookie".to_string()))?,
            },
        ),
        "123pan" => {
            let (u, p) = userpass
                .ok_or((StatusCode::BAD_REQUEST, "123 网盘需要账号密码".to_string()))?;
            (
                "123pan",
                Credential::Pan123 {
                    username: u,
                    password: p,
                    access_token: String::new(),
                    platform: "web".into(),
                },
            )
        }
        "aliyundrive_open" | "aliyundrive" | "aliyun" => (
            "aliyundrive_open",
            Credential::AliyundriveOpen {
                refresh_token: refresh.ok_or((
                    StatusCode::BAD_REQUEST,
                    "阿里云盘需要 refresh_token".to_string(),
                ))?,
                access_token: String::new(),
                alipan_type: if req.alipan_type.as_deref() == Some("alipanTV") {
                    "alipanTV".into()
                } else {
                    "default".into()
                },
            },
        ),
        "baidu_netdisk" | "baidu" => (
            "baidu_netdisk",
            Credential::BaiduNetdisk {
                refresh_token: refresh.ok_or((
                    StatusCode::BAD_REQUEST,
                    "百度网盘需要 refresh_token".to_string(),
                ))?,
                access_token: String::new(),
            },
        ),
        "pan115" | "115" => (
            "pan115",
            Credential::Pan115 {
                cookie: cookie.ok_or((
                    StatusCode::BAD_REQUEST,
                    "115 网盘需要 cookie（含 UID/CID/SEID）".to_string(),
                ))?,
            },
        ),
        "thunder" | "xunlei" => {
            let (u, p) = userpass
                .ok_or((StatusCode::BAD_REQUEST, "迅雷网盘需要账号密码".to_string()))?;
            (
                "thunder",
                Credential::Thunder {
                    username: u,
                    password: p,
                    refresh_token: String::new(),
                    captcha_token: String::new(),
                    device_id: String::new(),
                },
            )
        }
        "lanzou" | "lanzouyun" => {
            let account = req.username.clone().unwrap_or_default().trim().to_string();
            let password = req.password.clone().unwrap_or_default();
            let cookie = req.cookie.clone().unwrap_or_default().trim().to_string();
            if account.is_empty() && cookie.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "蓝奏云需要账号密码或 cookie".to_string(),
                ));
            }
            ("lanzou", Credential::Lanzou { cookie, account, password })
        }
        "pan139" | "139yun" | "mobile" => {
            let auth = req
                .authorization
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or((StatusCode::BAD_REQUEST, "移动云盘需要 139 Authorization".to_string()))?;
            ("pan139", Credential::Yun139 {
                authorization: auth.to_string(),
                drive_type: "personal_new".into(),
            })
        }
        "cloud189" | "189" | "tianyi" => {
            let (u, p) = userpass
                .ok_or((StatusCode::BAD_REQUEST, "天翼云盘需要账号密码".to_string()))?;
            ("cloud189", Credential::Cloud189 {
                username: u,
                password: p,
                cookie: String::new(),
            })
        }
        "local" => {
            let root_path = req
                .root_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or((StatusCode::BAD_REQUEST, "本机存储需要挂载目录路径".to_string()))?;
            ("local", Credential::Local { root_path: root_path.to_string() })
        }
        "webdav" => {
            let url = req
                .url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or((StatusCode::BAD_REQUEST, "WebDAV 需要服务器地址".to_string()))?;
            ("webdav", Credential::Webdav {
                url: url.to_string(),
                username: req.username.clone().unwrap_or_default(),
                password: req.password.clone().unwrap_or_default(),
                root_path: req
                    .root_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("/")
                    .to_string(),
            })
        }
        "123pan_share" | "123share" => {
            let share_key = req
                .share_key
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or((StatusCode::BAD_REQUEST, "123 分享需要 shareKey".to_string()))?;
            ("123pan_share", Credential::Pan123Share {
                share_key: share_key.to_string(),
                share_pwd: req.share_pwd.clone().unwrap_or_default(),
                access_token: req.access_token.clone().unwrap_or_default(),
            })
        }
        "weiyun" => {
            let cookies = req
                .cookies
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or((StatusCode::BAD_REQUEST, "腾讯微云需要登录 cookie".to_string()))?;
            ("weiyun", Credential::Weiyun {
                cookies: cookies.to_string(),
                root_folder_id: String::new(),
            })
        }
        "onedrive" | "onedrive_app" => {
            let rt = refresh.ok_or((
                StatusCode::BAD_REQUEST,
                "OneDrive 需要 refresh_token".to_string(),
            ))?;
            ("onedrive", Credential::Onedrive {
                region: req
                    .region
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("global")
                    .to_string(),
                is_sharepoint: req.is_sharepoint,
                site_id: req.site_id.clone().unwrap_or_default(),
                root_path: req
                    .root_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("/")
                    .to_string(),
                refresh_token: rt,
                access_token: String::new(),
            })
        }
        "google_drive" | "google" | "googledrive" => {
            let rt = refresh.ok_or((
                StatusCode::BAD_REQUEST,
                "Google Drive 需要 refresh_token".to_string(),
            ))?;
            ("google_drive", Credential::GoogleDrive {
                refresh_token: rt,
                access_token: String::new(),
                root_folder_id: req.root_folder_id.clone().unwrap_or_default(),
            })
        }
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("不支持的网盘类型: {other}"),
            ))
        }
    };

    // 用新凭据验证（复用 Driver::new，失败则拒绝保存）
    let new_driver = Driver::new(&id, &cred, st.store.clone())
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let new_name = if req.name.trim().is_empty() {
        driver_display_name(driver_kind).to_string()
    } else {
        req.name.trim().to_string()
    };

    // 原地替换配置，id 保持不变
    {
        let mut data = st.store.data.lock().unwrap();
        if let Some(acc) = data.accounts.iter_mut().find(|a| a.id == id) {
            acc.name = new_name.clone();
            // 根目录 fid 随驱动同步更新（本机存储 = 挂载路径，其余 = 驱动默认根 id）
            acc.root_fid = default_root_fid(driver_kind, &cred);
            acc.cred = cred;
            acc.server_proxy = req.server_proxy;
        }
        st.store
            .save(&data)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("保存配置失败: {e}")))?;
    }

    // 替换驱动缓存
    st.drivers.lock().unwrap().insert(id.clone(), Arc::new(new_driver));
    // 凭据已变更，清空该账号全部目录缓存，避免展示旧数据
    st.list_cache.invalidate_account(&id);

    Ok(Json(json!({ "id": id, "name": new_name, "driver": driver_kind })))
}
