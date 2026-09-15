//! 腾讯微云驱动（对齐 Go 版 drivers/weiyun + weiyunsdk-go 协议，只读浏览 + 下载）
//!
//! - 授权：www.weiyun.com 登录后的 cookie（支持 QQ / 微信登录）
//! - 协议：POST https://www.weiyun.com/webapp/json/{protocol}/{name}
//!   body = {"req_header": "<json 字符串>", "req_body": "<json 字符串>"}
//!   query = g_tk(wyctoken cookie) + cmd
//! - 列目录：weiyunQdisk / DiskDirList（cmd 2208，count 500 翻页）
//! - 下载：weiyunQdiskClient / DiskFileBatchDownload（cmd 2402），
//!   直链需附带 Set-Cookie 下发的下载 cookie，必须走后端代理

use super::DownloadInfo;
use crate::config::Entry;
use reqwest::{Client, ClientBuilder, redirect};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BASE: &str = "https://www.weiyun.com";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";
const TIMEOUT: Duration = Duration::from_secs(30);
/// cookie 过期错误标记（对齐 ErrCookieExpiration）
const EXPIRED: &str = "微云 cookie 已过期，请重新获取";

pub struct Weiyun {
    http: Client,
    http_no_redirect: Client,
    cookies: Mutex<HashMap<String, String>>,
    cookie_order: Mutex<Vec<String>>,
    root_folder_id: String,
    /// 账号主目录 dir_key（懒加载）
    main_dir_key: Mutex<String>,
    /// 403 重试标记，防止无限循环
    refreshing: Mutex<bool>,
}

impl Weiyun {
    pub fn new(cookies: String, root_folder_id: String) -> Self {
        let (map, order) = parse_cookie_str(&cookies);
        Weiyun {
            http: Client::builder().timeout(TIMEOUT).build().unwrap(),
            http_no_redirect: ClientBuilder::new()
                .timeout(TIMEOUT)
                .redirect(redirect::Policy::none())
                .build()
                .unwrap(),
            cookies: Mutex::new(map),
            cookie_order: Mutex::new(order),
            root_folder_id,
            main_dir_key: Mutex::new(String::new()),
            refreshing: Mutex::new(false),
        }
    }

    fn cookie(&self, name: &str) -> String {
        self.cookies
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .unwrap_or_default()
    }

    fn cookie_header(&self) -> String {
        let map = self.cookies.lock().unwrap();
        let order = self.cookie_order.lock().unwrap();
        order
            .iter()
            .filter_map(|k| map.get(k).map(|v| format!("{k}={v}")))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// 对齐 Go 版 cookie jar：每次响应的 Set-Cookie 滚动并入 cookie 存储，
    /// 微云会经 Set-Cookie 轮换会话字段，丢弃会导致后续请求被网关拒绝
    fn store_cookies(&self, headers: &reqwest::header::HeaderMap) {
        let mut map = self.cookies.lock().unwrap();
        let mut order = self.cookie_order.lock().unwrap();
        for v in headers.get_all(reqwest::header::SET_COOKIE) {
            let Ok(s) = v.to_str() else { continue };
            // 只取第一个 k=v 对（忽略 Path/Domain/Expires 等属性）
            let Some(pair) = s.split(';').next() else { continue };
            let Some((k, val)) = pair.split_once('=') else { continue };
            let (k, val) = (k.trim(), val.trim());
            if k.is_empty() || val.is_empty() {
                continue;
            }
            if !map.contains_key(k) {
                order.push(k.to_string());
            }
            map.insert(k.to_string(), val.to_string());
        }
    }

    /// 对齐 LoginType()
    fn login_type(&self) -> &'static str {
        let wy_uf = self.cookie("wy_uf");
        if wy_uf == "2" {
            if !self.cookie("weiyun_wx_openid").is_empty() {
                return "weixin_openid";
            }
            if !self.cookie("weiyun_qq_openid").is_empty() {
                return "qq_openid";
            }
        }
        if wy_uf == "1" {
            return "weixin";
        }
        if wy_uf == "0" || wy_uf.is_empty() {
            return "qq";
        }
        "unknown"
    }

    /// 对齐 ParseTokenInfo()
    fn token_info(&self) -> Value {
        match self.login_type() {
            "weixin" => json!({
                "token_type": 1,
                "openid": self.cookie("openid"),
                "open_appid": self.cookie("wy_appid"),
                "access_token": self.cookie("access_token"),
                "login_key_type": 192,
                "login_key_value": self.cookie("access_token"),
            }),
            "weixin_openid" | "qq_openid" => json!({
                "token_type": 3,
                "login_key_type": 1540,
            }),
            "qq" => json!({
                "token_type": 0,
                "login_key_type": 27,
                "login_key_value": self.cookie("p_skey"),
                "openid": "",
            }),
            _ => json!({}),
        }
    }

    /// 对齐 RefreshCtoken()：GET /disk，发生 302 即 cookie 过期
    async fn refresh_ctoken(&self) -> Result<(), String> {
        let resp = self
            .http_no_redirect
            .get(format!("{BASE}/disk"))
            .header("user-agent", UA)
            .header("cookie", self.cookie_header())
            .send()
            .await
            .map_err(|e| format!("刷新微云会话失败: {e}"))?;
        self.store_cookies(resp.headers());
        if resp.status().is_redirection() || resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(EXPIRED.to_string());
        }
        Ok(())
    }

    /// 对齐 WeiXinRefreshToken()：微信登录 cookie 过期时刷 access_token，
    /// 依赖 cookie 中的 wy_appid / refresh_token，成功后回写 openid / access_token / refresh_token
    async fn weixin_refresh_token(&self) -> Result<(), String> {
        let (appid, rt) = (self.cookie("wy_appid"), self.cookie("refresh_token"));
        if appid.is_empty() || rt.is_empty() {
            return Err("微信刷新 token 缺少 wy_appid / refresh_token cookie".into());
        }
        let resp = self
            .http
            .get("https://api.weixin.qq.com/sns/oauth2/refresh_token")
            .query(&[
                ("grant_type", "refresh_token"),
                ("appid", appid.as_str()),
                ("refresh_token", rt.as_str()),
            ])
            .header("user-agent", UA)
            .send()
            .await
            .map_err(|e| format!("微信刷新 token 请求失败: {e}"))?;
        let v: Value = resp
            .json()
            .await
            .map_err(|e| format!("微信刷新 token 响应解析失败: {e}"))?;
        let errcode = v.get("errcode").and_then(|x| x.as_i64()).unwrap_or(0);
        if errcode != 0 {
            let errmsg = v
                .get("errmsg")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown");
            return Err(format!("微信刷新 token 失败(errcode={errcode}): {errmsg}"));
        }
        // 对齐 SetCookieValue：回写 cookie 中的同名字段（无则新增）
        let mut map = self.cookies.lock().unwrap();
        let mut order = self.cookie_order.lock().unwrap();
        for key in ["openid", "access_token", "refresh_token"] {
            if let Some(nv) = v.get(key).and_then(|x| x.as_str()) {
                if nv.is_empty() {
                    continue;
                }
                if !map.contains_key(key) {
                    order.push(key.to_string());
                }
                map.insert(key.to_string(), nv.to_string());
            }
        }
        Ok(())
    }

    /// 对齐 request()（默认分支）：403 时刷新会话重试一次
    async fn request(
        &self,
        protocol: &str,
        cmd_name: &str,
        cmd: i64,
        data: Value,
    ) -> Result<Value, String> {
        let body = self.build_body(cmd_name, cmd, &data);
        let resp = self
            .do_request(protocol, cmd_name, cmd, &body)
            .await?;
        match resp {
            Ok(v) => Ok(v),
            Err(e) if e.contains("HTTP 403") => {
                // 单飞刷新后重试一次；微信登录先刷微信 token 再验证（对齐 Request() 403 分支）
                let should_refresh = {
                    let mut flag = self.refreshing.lock().unwrap();
                    if *flag {
                        false
                    } else {
                        *flag = true;
                        true
                    }
                };
                if !should_refresh {
                    return Err(e);
                }
                let r = match self.refresh_ctoken().await {
                    Err(e2) if e2 == EXPIRED
                        && matches!(self.login_type(), "weixin" | "weixin_openid") =>
                    {
                        match self.weixin_refresh_token().await {
                            Ok(()) => self.refresh_ctoken().await,
                            Err(e3) => Err(e3),
                        }
                    }
                    other => other,
                };
                *self.refreshing.lock().unwrap() = false;
                r?;
                let resp = self.do_request(protocol, cmd_name, cmd, &body).await?;
                resp
            }
            Err(e) => Err(e),
        }
    }

    fn build_body(&self, cmd_name: &str, cmd: i64, data: &Value) -> Value {
        let token_info = self.token_info();
        let wx_openid = token_info
            .get("openid")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let wx_openid = if wx_openid.is_empty() {
            token_info.get("minico_openid").cloned().unwrap_or(Value::Null)
        } else {
            json!(wx_openid)
        };
        let seq = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let req_header = json!({
            "seq": seq,
            "cmd": cmd,
            "wx_openid": wx_openid,
            "qq_openid": token_info.get("qq_openid").cloned().unwrap_or(Value::Null),
            "user_flag": token_info.get("token_type").cloned().unwrap_or(Value::Null),
            "env_id": token_info.get("env_id").cloned().unwrap_or(Value::Null),
            "type": 1,
            "appid": 30013,
            "version": 3,
            "major_version": 3,
            "minor_version": 3,
            "fix_version": 3,
        });
        let mut req_body = json!({
            "ReqMsg_body": {
                "ext_req_head": {
                    "token_info": token_info,
                    "language_info": { "language_type": 2052 },
                }
            }
        });
        // json! 宏的 key 需为字面量，动态 cmd_name 的字段运行时插入
        req_body["ReqMsg_body"][format!(".weiyun.{cmd_name}MsgReq_body").as_str()] = data.clone();
        json!({
            "req_header": serde_json::to_string(&req_header).unwrap_or_default(),
            "req_body": serde_json::to_string(&req_body).unwrap_or_default(),
        })
    }

    /// 发送请求并解析 Resp{ret,msg,data:{rsp_header,rsp_body}}
    async fn do_request(
        &self,
        protocol: &str,
        cmd_name: &str,
        cmd: i64,
        body: &Value,
    ) -> Result<Result<Value, String>, String> {
        let url = format!("{BASE}/webapp/json/{protocol}/{cmd_name}");
        let resp = self
            .http
            .post(&url)
            .query(&[
                ("g_tk", self.cookie("wyctoken")),
                ("cmd", cmd.to_string()),
            ])
            .header("user-agent", UA)
            .header("referer", format!("{BASE}/disk"))
            .header("origin", BASE)
            .header("content-type", "application/json")
            .header("cookie", self.cookie_header())
            .json(body)
            .send()
            .await
            .map_err(|e| format!("微云请求失败: {e}"))?;
        let status = resp.status().as_u16();
        self.store_cookies(resp.headers());
        let text = resp.text().await.map_err(|e| format!("读取响应失败: {e}"))?;
        if status != 200 {
            if status == 403 {
                return Ok(Err("HTTP 403（会话可能已过期）".into()));
            }
            return Err(format!(
                "微云服务器返回 HTTP {status}（{cmd_name}）: {}",
                trunc300(&text)
            ));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| format!("微云响应解析失败: {e}: {}", trunc300(&text)))?;
        let ret = v.get("ret").and_then(|x| x.as_i64()).unwrap_or(-1);
        if ret != 0 {
            let msg = v
                .get("msg")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
                .to_string();
            return Ok(Err(format!(
                "微云接口错误({cmd_name} ret={ret}): {msg}；响应: {}",
                trunc300(&text)
            )));
        }
        let retcode = v
            .pointer("/data/rsp_header/retcode")
            .and_then(|x| x.as_i64())
            .unwrap_or(0);
        if retcode != 0 {
            let msg = v
                .pointer("/data/rsp_header/retmsg")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
                .to_string();
            return Ok(Err(format!(
                "微云接口错误({cmd_name} retcode={retcode}): {msg}"
            )));
        }
        let body = v
            .pointer("/data/rsp_body/RspMsg_body")
            .cloned()
            .unwrap_or(json!({}));
        Ok(Ok(body))
    }

    /// 根目录 dir_key：优先凭据里的 root_folder_id，否则账号主目录（懒加载）
    async fn root_dir_key(&self) -> Result<String, String> {
        if !self.root_folder_id.is_empty() {
            return Ok(self.root_folder_id.clone());
        }
        {
            let k = self.main_dir_key.lock().unwrap();
            if !k.is_empty() {
                return Ok(k.clone());
            }
        }
        let info = self.disk_user_info_get().await?;
        let key = info
            .get("main_dir_key")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if key.is_empty() {
            return Err("微云未获取到账号主目录".into());
        }
        *self.main_dir_key.lock().unwrap() = key.clone();
        Ok(key)
    }

    /// weiyunQdiskClient / DiskUserInfoGet（cmd 2201）
    async fn disk_user_info_get(&self) -> Result<Value, String> {
        let data = json!({
            "is_get_upload_flow_flag": false,
            "is_get_high_speed_flow_info": false,
            "is_get_weiyun_flag": false,
            "is_get_space_clean_info": false,
            "is_get_user_reward_info": false,
        });
        self.request("weiyunQdiskClient", "DiskUserInfoGet", 2201, data)
            .await
    }

    /// 对齐 Init()
    pub async fn validate(&self) -> Result<(), String> {
        if self.cookie_header().is_empty() {
            return Err("微云 cookies 不能为空".into());
        }
        self.refresh_ctoken().await?;
        self.disk_user_info_get().await?;
        // 预取主目录，失败即凭据无效
        self.root_dir_key().await?;
        Ok(())
    }

    /// 对齐 List()：DiskDirList（cmd 2208）count 500 翻页
    pub async fn list(&self, parent_fid: &str) -> Result<Vec<Entry>, String> {
        let dir_key = if parent_fid.is_empty() || parent_fid == "0" {
            self.root_dir_key().await?
        } else {
            parent_fid.to_string()
        };
        let mut out = Vec::new();
        let mut start: i64 = 0;
        loop {
            let data = json!({
                "dir_key": dir_key,
                "start": start,
                "count": 500,
                "sort_field": 2,
                "reverse_order": false,
                "get_type": 0,
                "get_abstract_url": false,
                "get_dir_detail_info": false,
            });
            let resp = self
                .request("weiyunQdisk", "DiskDirList", 2208, data)
                .await?;
            let dirs = resp
                .get("dir_list")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
            let files = resp
                .get("file_list")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
            let finish = resp
                .get("finish_flag")
                .and_then(|x| x.as_bool())
                .unwrap_or(true);
            for d in &dirs {
                out.push(Entry {
                    fid: d
                        .get("dir_key")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    name: d
                        .get("dir_name")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    size: 0,
                    is_dir: true,
                    updated_at: json_num_or_str_ms(d.get("dir_mtime")),
                    etag: None,
                    s3_key_flag: None,
                    file_type: None,
                    extra: None,
                });
            }
            for f in &files {
                // 文件下载需要父目录 dir_key，挂到 extra 透传
                let extra = json!({ "pdir_key": dir_key });
                out.push(Entry {
                    fid: f
                        .get("file_id")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    name: f
                        .get("filename")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    size: f.get("file_size").and_then(|x| x.as_u64()).unwrap_or(0),
                    is_dir: false,
                    updated_at: json_num_or_str_ms(f.get("file_mtime")),
                    etag: f
                        .get("file_sha")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string()),
                    s3_key_flag: None,
                    file_type: None,
                    extra: Some(extra),
                });
            }
            start += (dirs.len() + files.len()) as i64;
            if finish || (dirs.is_empty() && files.is_empty()) {
                break;
            }
        }
        Ok(out)
    }

    /// 对齐 Link()：DiskFileBatchDownload（cmd 2402）
    pub async fn download(&self, e: &Entry) -> Result<DownloadInfo, String> {
        if e.is_dir {
            return Err("目录无法下载".into());
        }
        let pdir_key = e
            .extra
            .as_ref()
            .and_then(|x| x.get("pdir_key"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if pdir_key.is_empty() {
            return Err("微云缺少父目录信息（pdir_key）".into());
        }
        let data = json!({
            "file_list": [ { "pdir_key": pdir_key, "file_id": e.fid } ],
            "download_type": 0,
        });
        let resp = self
            .request("weiyunQdiskClient", "DiskFileBatchDownload", 2402, data)
            .await?;
        let item = resp
            .get("file_list")
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .cloned()
            .ok_or("微云未返回下载信息")?;
        let url = item
            .get("download_url")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if url.is_empty() {
            return Err("微云未取到下载直链".into());
        }
        let cookie = format!(
            "{}={}",
            item.get("cookie_name").and_then(|x| x.as_str()).unwrap_or(""),
            item.get("cookie_value").and_then(|x| x.as_str()).unwrap_or("")
        );
        Ok(DownloadInfo {
            url,
            headers: vec![("Cookie".into(), cookie)],
            proxy: true,
            local_path: None,
        })
    }
}

/// 微云的 mtime 可能是数字或数字字符串，统一解析为毫秒
fn json_num_or_str_ms(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// 错误信息里附带的原始响应片段（按字符截断，避免 UTF-8 边界 panic）
fn trunc300(s: &str) -> String {
    s.chars().take(300).collect()
}

/// 解析 "k=v; k2=v2" cookie 串（保留顺序，去重取后值）
fn parse_cookie_str(s: &str) -> (HashMap<String, String>, Vec<String>) {
    let mut map = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for part in s.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        let k = k.trim().to_string();
        let v = v.trim().to_string();
        // 对齐 ClearCookie：去空值
        if k.is_empty() || v.is_empty() {
            continue;
        }
        if !map.contains_key(&k) {
            order.push(k.clone());
        }
        map.insert(k, v);
    }
    (map, order)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cookie() {
        let (map, order) = parse_cookie_str("a=1; b=2; a=3; ; c=");
        assert_eq!(map.get("a").unwrap(), "3");
        assert_eq!(map.get("b").unwrap(), "2");
        assert!(!map.contains_key("c"));
        assert_eq!(order, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn test_token_info_qq() {
        let w = Weiyun::new("wy_uf=0; p_skey=abc; wyctoken=tk".into(), String::new());
        assert_eq!(w.login_type(), "qq");
        let ti = w.token_info();
        assert_eq!(ti["token_type"], 0);
        assert_eq!(ti["login_key_value"], "abc");
        assert_eq!(w.cookie("wyctoken"), "tk");
    }

    #[test]
    fn test_token_info_weixin() {
        let w = Weiyun::new("wy_uf=1; openid=o1; wy_appid=ap; access_token=at".into(), String::new());
        assert_eq!(w.login_type(), "weixin");
        let ti = w.token_info();
        assert_eq!(ti["token_type"], 1);
        assert_eq!(ti["openid"], "o1");
    }

    #[test]
    fn test_build_body() {
        let w = Weiyun::new("wy_uf=0; p_skey=abc; wyctoken=tk".into(), String::new());
        let body = w.build_body("DiskDirList", 2208, &json!({"dir_key": "x"}));
        let header: Value =
            serde_json::from_str(body["req_header"].as_str().unwrap()).unwrap();
        assert_eq!(header["cmd"], 2208);
        assert_eq!(header["appid"], 30013);
        let inner: Value =
            serde_json::from_str(body["req_body"].as_str().unwrap()).unwrap();
        assert_eq!(
            inner["ReqMsg_body"][".weiyun.DiskDirListMsgReq_body"]["dir_key"],
            "x"
        );
    }
}
