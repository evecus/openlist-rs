use crate::api::{proxy_stream, sort_entries};
use crate::config::{Account, Entry};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

// ============================================================
// OpenList 官方 API 兼容层（AList 协议，供 NovaTV / TVBox 等客户端）
// 响应统一 HTTP 200 + {"code":..,"message":..,"data":..}
// 路径方案：根目录下为各账号文件夹（账号备注名），进入即浏览对应网盘
// ============================================================

// 对齐 OpenList internal/conf: UNKNOWN=0 FOLDER=1 VIDEO=2 AUDIO=3 TEXT=4 IMAGE=5
const T_UNKNOWN: i64 = 0;
const T_FOLDER: i64 = 1;
const T_VIDEO: i64 = 2;
const T_AUDIO: i64 = 3;
const T_TEXT: i64 = 4;
const T_IMAGE: i64 = 5;
// 对齐 internal/bootstrap/data/setting.go 默认扩展名
const VIDEO_EXTS: &[&str] = &["mp4", "mkv", "avi", "mov", "rmvb", "webm", "flv", "m3u8"];
const AUDIO_EXTS: &[&str] = &["mp3", "flac", "ogg", "m4a", "wav", "opus", "wma"];
const IMAGE_EXTS: &[&str] =
    &["jpg", "tiff", "jpeg", "png", "gif", "bmp", "svg", "ico", "swf", "webp", "avif"];
const TEXT_EXTS: &str = "txt,htm,html,xml,java,properties,sql,js,md,json,conf,ini,vue,php,py,bat,gitignore,css,go,csv,c,h,sh,vtt,srt,ass,ssa";

fn obj_type(name: &str, is_dir: bool) -> i64 {
    if is_dir {
        return T_FOLDER;
    }
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    if VIDEO_EXTS.contains(&ext.as_str()) {
        return T_VIDEO;
    }
    if AUDIO_EXTS.contains(&ext.as_str()) {
        return T_AUDIO;
    }
    if IMAGE_EXTS.contains(&ext.as_str()) {
        return T_IMAGE;
    }
    if TEXT_EXTS.split(',').any(|e| e == ext) {
        return T_TEXT;
    }
    T_UNKNOWN
}

fn normalize_path(p: &str) -> String {
    let mut s = p.trim().to_string();
    if !s.starts_with('/') {
        s.insert(0, '/');
    }
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    s
}

/// 对齐 Go utils.EncodePath: 按段百分号编码，保留 /
fn path_encode(p: &str) -> String {
    p.split('/')
        .map(|seg| {
            let mut out = String::new();
            for b in seg.bytes() {
                match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'@'
                    | b'!' | b'(' | b')' | b'\'' | b'*' => out.push(b as char),
                    _ => out.push_str(&format!("%{b:02X}")),
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// unix 毫秒 -> RFC3339（东八区），对齐 Go time.Time JSON 格式
fn rfc3339_cst(ms: i64) -> String {
    let secs = (ms / 1000) + 8 * 3600;
    let rem = (secs as u64) % 86400;
    let days = secs.div_euclid(86400);
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}+08:00")
}

fn obj_resp_json(e: &Entry) -> Value {
    let modified = e.updated_at.map(rfc3339_cst);
    json!({
        "name": e.name,
        "size": e.size,
        "is_dir": e.is_dir,
        "modified": modified,
        "created": modified,
        "sign": "",
        "thumb": "",
        "type": obj_type(&e.name, e.is_dir),
        "hashinfo": "",
        "hash_info": null,
    })
}

fn compat_err(msg: impl Into<String>, code: i64) -> Response {
    (
        StatusCode::OK,
        Json(json!({ "code": code, "message": msg.into(), "data": null })),
    )
        .into_response()
}

// ---------- 路径索引（AppState 的兼容层方法） ----------

impl AppState {
    /// 把目录 children 注册到路径索引
    fn index_children(&self, parent: &str, account_id: &str, children: &[Entry]) {
        let mut idx = self.index.lock().unwrap();
        for c in children {
            let key = format!("{parent}/{}", c.name);
            idx.insert(key, (account_id.to_string(), c.clone()));
        }
    }

    /// 解析路径 -> (account_id, Entry)。索引未命中时逐级向下走并注册。
    pub(crate) async fn resolve_path(&self, path: &str) -> Result<(String, Entry), String> {
        let path = normalize_path(path);
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if segs.is_empty() {
            return Err("根目录没有对应文件".into());
        }
        // 第一段 = 账号名
        let accounts: Vec<Account> = self.store.data.lock().unwrap().accounts.clone();
        let Some(acc) = accounts.iter().find(|a| a.name == segs[0]) else {
            return Err(format!("找不到网盘账号: {}", segs[0]));
        };
        let acc_id = acc.id.clone();
        let mut entry = Entry {
            fid: acc.root_fid.clone(),
            name: segs[0].to_string(),
            is_dir: true,
            ..Default::default()
        };
        {
            let mut idx = self.index.lock().unwrap();
            idx.insert(format!("/{}", segs[0]), (acc_id.clone(), entry.clone()));
        }
        let mut prefix = format!("/{}", segs[0]);
        for seg in &segs[1..] {
            if !entry.is_dir {
                return Err(format!("路径不存在: {prefix}/{seg}"));
            }
            let full = format!("{prefix}/{seg}");
            // 索引命中直接复用
            if let Some(hit) = self.index.lock().unwrap().get(&full).cloned() {
                prefix = full;
                entry = hit.1;
                continue;
            }
            // 未命中：列出父目录并注册
            let driver = self.get_driver(&acc_id).await?;
            let children = driver.list(&entry.fid).await?;
            self.index_children(&prefix, &acc_id, &children);
            let hit = self.index.lock().unwrap().get(&full).cloned();
            let Some(hit) = hit else {
                return Err(format!("路径不存在: {full}"));
            };
            prefix = full;
            entry = hit.1;
        }
        Ok((acc_id, entry))
    }

    /// 列目录（含虚拟根：根目录下是各账号文件夹）
    async fn compat_list_dir(&self, path: &str) -> Result<Vec<Entry>, String> {
        let path = normalize_path(path);
        if path == "/" {
            let accounts: Vec<Account> = self.store.data.lock().unwrap().accounts.clone();
            return Ok(accounts
                .iter()
                .map(|a| Entry {
                    fid: format!("virtual:{}", a.id),
                    name: a.name.clone(),
                    is_dir: true,
                    size: 0,
                    ..Default::default()
                })
                .collect());
        }
        let (acc_id, entry) = self.resolve_path(&path).await?;
        if !entry.is_dir {
            return Err(format!("不是目录: {path}"));
        }
        let driver = self.get_driver(&acc_id).await?;
        let children = driver.list(&entry.fid).await?;
        self.index_children(&path, &acc_id, &children);
        Ok(children)
    }
}

// ---------- 端点 ----------

#[derive(Deserialize)]
#[allow(dead_code)]
pub(crate) struct CompatLoginReq {
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    otp_code: String,
}

/// POST /api/auth/login —— 对齐 OpenList 登录响应
pub(crate) async fn compat_login(
    State(st): State<AppState>,
    Json(req): Json<CompatLoginReq>,
) -> Response {
    let Some(auth) = &st.auth else {
        // 未启用面板鉴权：签发一个会话 token（校验时放行）
        let token = uuid::Uuid::new_v4().to_string();
        st.sessions.lock().unwrap().insert(token.clone());
        return (
            StatusCode::OK,
            Json(json!({ "code": 200, "message": "success", "data": { "token": token } })),
        )
            .into_response();
    };
    if req.username != auth.user || req.password != auth.pass {
        return compat_err("用户名或密码错误", 400);
    }
    let token = uuid::Uuid::new_v4().to_string() + &uuid::Uuid::new_v4().simple().to_string();
    st.sessions.lock().unwrap().insert(token.clone());
    (
        StatusCode::OK,
        Json(json!({ "code": 200, "message": "success", "data": { "token": token } })),
    )
        .into_response()
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(crate) struct CompatListReq {
    path: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    page: i64,
    #[serde(default)]
    per_page: i64,
    #[serde(default)]
    refresh: bool,
}

/// POST /api/fs/list —— 对齐 OpenList FsListSplit
pub(crate) async fn compat_fs_list(
    State(st): State<AppState>,
    Json(req): Json<CompatListReq>,
) -> Response {
    let mut entries = match st.compat_list_dir(&req.path).await {
        Ok(e) => e,
        Err(e) => return compat_err(e, 500),
    };
    // 目录在前、按名称排序（官方由驱动决定顺序，这里保持稳定排序）
    sort_entries(&mut entries);
    let total = entries.len() as i64;
    // 对齐官方 pagination: page 从 1 起，per_page<=0 返回全部
    let per_page = if req.per_page <= 0 { total.max(1) } else { req.per_page };
    let page = if req.page <= 0 { 1 } else { req.page };
    let start = (((page - 1) * per_page).clamp(0, total)) as usize;
    let end = ((page * per_page).clamp(0, total)) as usize;
    let content: Vec<Value> = entries[start..end].iter().map(obj_resp_json).collect();
    (
        StatusCode::OK,
        Json(json!({
            "code": 200,
            "message": "success",
            "data": {
                "content": content,
                "total": total,
                "readme": "",
                "header": "",
                "write": false,
                "provider": "",
            }
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(crate) struct CompatGetReq {
    path: String,
    #[serde(default)]
    password: String,
}

/// POST /api/fs/get —— 对齐 OpenList FsGet，返回含 raw_url
pub(crate) async fn compat_fs_get(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CompatGetReq>,
) -> Response {
    let path = normalize_path(&req.path);
    if path == "/" {
        return compat_err("无法获取根目录信息", 500);
    }
    let (acc_id, entry) = match st.resolve_path(&path).await {
        Ok(v) => v,
        Err(e) => return compat_err(e, 500),
    };
    if entry.is_dir {
        let mut data = obj_resp_json(&entry);
        data["raw_url"] = json!("");
        data["provider"] = json!("");
        data["related"] = json!([]);
        return (
            StatusCode::OK,
            Json(json!({ "code": 200, "message": "success", "data": data })),
        )
            .into_response();
    }
    // 查询账号的 server_proxy 开关
    let server_proxy = {
        let data = st.store.data.lock().unwrap();
        data.accounts
            .iter()
            .find(|a| a.id == acc_id)
            .map(|a| a.server_proxy)
            .unwrap_or(false)
    };

    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("127.0.0.1");
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("http");

    let raw_url = if server_proxy {
        // 开启服务器代理：raw_url 指向本服务 /p 路径，所有流量经本机中转
        format!("{proto}://{host}/p{}", path_encode(&path))
    } else {
        // 关闭服务器代理：从驱动获取真实直链，客户端直接请求网盘服务器
        match st.get_driver(&acc_id).await {
            Ok(driver) => match driver.download(&entry).await {
                Ok(info) => info.url,
                Err(_) => {
                    // 取直链失败时降级为代理路径
                    format!("{proto}://{host}/p{}", path_encode(&path))
                }
            },
            Err(_) => format!("{proto}://{host}/p{}", path_encode(&path)),
        }
    };

    let mut data = obj_resp_json(&entry);
    data["raw_url"] = json!(raw_url);
    data["provider"] = json!("");
    data["related"] = json!([]);
    (
        StatusCode::OK,
        Json(json!({ "code": 200, "message": "success", "data": data })),
    )
        .into_response()
}

/// GET /d/{*path}（下载，attachment）与 /p/{*path}（代理播放，inline）
///
/// 行为由账号 `server_proxy` 开关决定：
/// - true  → 本服务中转（原有行为）
/// - false → 302 跳转真实直链，客户端直接访问网盘服务器
async fn compat_file(
    State(st): State<AppState>,
    Path(raw): Path<String>,
    headers: HeaderMap,
    disp: &'static str,
) -> Response {
    let path = normalize_path(&raw);
    let Ok((acc_id, entry)) = st.resolve_path(&path).await else {
        return (StatusCode::NOT_FOUND, "路径不存在").into_response();
    };
    if entry.is_dir {
        return (StatusCode::BAD_REQUEST, "是目录不是文件").into_response();
    }

    // 读取账号的 server_proxy 开关
    let server_proxy = {
        let data = st.store.data.lock().unwrap();
        data.accounts
            .iter()
            .find(|a| a.id == acc_id)
            .map(|a| a.server_proxy)
            .unwrap_or(false)
    };

    let Ok(driver) = st.get_driver(&acc_id).await else {
        return (StatusCode::BAD_REQUEST, "账号不可用").into_response();
    };

    if server_proxy {
        // 服务器代理模式：流式中转
        match proxy_stream(&driver, &entry, &headers, disp).await {
            Ok(r) => r,
            Err((s, m)) => (s, m).into_response(),
        }
    } else {
        // 直链模式：获取真实 URL 后 302 跳转，客户端直接请求网盘
        match driver.download(&entry).await {
            Ok(info) => Redirect::temporary(&info.url).into_response(),
            Err(e) => (StatusCode::BAD_GATEWAY, format!("获取直链失败: {e}")).into_response(),
        }
    }
}

pub(crate) async fn compat_down(
    State(st): State<AppState>,
    Path(raw): Path<String>,
    headers: HeaderMap,
) -> Response {
    compat_file(State(st), Path(raw), headers, "attachment").await
}

pub(crate) async fn compat_proxy(
    State(st): State<AppState>,
    Path(raw): Path<String>,
    headers: HeaderMap,
) -> Response {
    compat_file(State(st), Path(raw), headers, "inline").await
}
