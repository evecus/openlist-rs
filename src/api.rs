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
        _ => "网盘",
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
        "lanzou" | "lanzouyun" => (
            "lanzou",
            Credential::Lanzou {
                cookie: cookie.ok_or((StatusCode::BAD_REQUEST, "蓝奏云需要 cookie".to_string()))?,
            },
        ),
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
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("不支持的网盘类型: {other}"),
            ))
        }
    };

    // 按驱动设置默认根目录 id
    let root_fid = match driver_kind {
        "pan139" => "/",
        "cloud189" => "-11",
        _ => "0",
    };

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
        root_fid: root_fid.into(),
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
        Credential::Lanzou { cookie } => {
            out["driver"] = json!("lanzou");
            out["cookie"] = json!(cookie);
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

    let mut req = reqwest::Client::new().get(&info.url);
    for (k, v) in &info.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    if let Some(range) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        req = req.header(header::RANGE, range);
    }
    let upstream = req
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("拉取直链失败: {e}")))?;
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
        "lanzou" | "lanzouyun" => (
            "lanzou",
            Credential::Lanzou {
                cookie: cookie.ok_or((StatusCode::BAD_REQUEST, "蓝奏云需要 cookie".to_string()))?,
            },
        ),
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
