//! 天翼云盘（189）驱动
//!
//! 移植自 OpenList Go 版 drivers/189（Cloud189）：
//! - 登录：账号密码经 RSA(PKCS1v15) 加密后走 open.e.189.cn 统一登录，
//!   依赖 cookie 会话（session 失效返回 InvalidSessionKey 时自动重登一次）
//! - 列表：/api/open/file/listFiles.action 分页拉取（默认根目录 -11）
//! - 直链：/api/portal/getFileInfo.action 取 downloadUrl 后跟一次 302 拿最终地址
//!
//! 上传、秒传等能力未实现（openlist-rs 仅支持 list + download）。

use super::DownloadInfo;
use crate::config::Entry;
use base64::Engine;
use rand::Rng;
use reqwest::redirect::Policy;
use reqwest::{Client, Method};
use rsa::pkcs8::DecodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
use serde_json::Value;
use std::sync::Arc;

const LOGIN_URL: &str = "https://cloud.189.cn/api/portal/loginUrl.action?redirectURL=https%3A%2F%2Fcloud.189.cn%2Fmain.action";
const LOGIN_OK_URL: &str = "https://cloud.189.cn/web/main";
/// 对齐 Go 版 base.UserAgent（OpenList-4.2.6 drivers/base/client.go）
const UA: &str = "Mozilla/5.0 (Macintosh; Apple macOS 26_1_0) AppleWebKit/537.36 (KHTML, like Gecko) Safari/537.36 Chrome/142.0.0.0 OpenList/425.6.30";
/// API 请求的默认 Referer（对齐 Go 版 Init(): SetHeader("Referer", "https://cloud.189.cn/")）
const API_REFERER: &str = "https://cloud.189.cn/";

/// 天翼云盘 API 的默认根目录 id（对齐 Go 版 DefaultRoot: -11）
const DEFAULT_ROOT: &str = "-11";

pub struct Cloud189 {
    /// 带 cookie 会话的客户端（登录态保存在共享 cookie jar 中）
    http: Client,
    /// 不跟随重定向的客户端（与 http 共享 cookie jar，用于拿 302 直链）
    http_no_redirect: Client,
    username: String,
    password: String,
}

/// 解析 "2024-01-02 15:04:05"（北京时间）-> epoch 毫秒（对齐 Go MustParseCNTime）
fn parse_cn_time(t: &str) -> Option<i64> {
    let b = t.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| std::str::from_utf8(&b[r]).ok()?.parse::<u32>().ok();
    let y = num(0..4)? as i64;
    let m = num(5..7)?;
    let d = num(8..10)?;
    let hh = num(11..13)?;
    let mm = num(14..16)?;
    let ss = num(17..19)?;
    let days = days_from_civil(y, m, d);
    Some((days * 86_400 + hh as i64 * 3600 + mm as i64 * 60 + ss as i64 - 8 * 3600) * 1000)
}

/// Howard Hinnant days_from_civil
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe
}

/// 对齐 Go random()："0." + 17 位宽的随机数
fn random_param() -> String {
    let n: u64 = rand::thread_rng().gen_range(0..100_000_000_000_000_000);
    format!("0.{n:>17}")
}

/// 对齐 Go b64tohex：将 base64 字符串按特殊规则转换为 hex 字符串
/// 注意：不是「base64解码后hex编码」，而是 Go help.go 里的逐6bit映射算法
fn b64tohex(a: &str) -> String {
    const B64MAP: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const BI_RM: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let int2char = |i: usize| -> char { BI_RM[i] as char };
    let mut d = String::new();
    let mut e: u8 = 0;
    let mut c: usize = 0;
    for m in a.chars() {
        if m == '=' { continue; }
        let v = match B64MAP.iter().position(|&b| b == m as u8) {
            Some(pos) => pos,
            None => continue,
        };
        if e == 0 {
            e = 1; d.push(int2char(v >> 2)); c = 3 & v;
        } else if e == 1 {
            e = 2; d.push(int2char((c << 2) | (v >> 4))); c = 15 & v;
        } else if e == 2 {
            e = 3; d.push(int2char(c)); d.push(int2char(v >> 2)); c = 3 & v;
        } else {
            e = 0; d.push(int2char((c << 2) | (v >> 4))); d.push(int2char(15 & v));
        }
    }
    if e == 1 { d.push(int2char(c << 2)); }
    d
}

/// 对齐 Go RsaEncode(data, j_rsakey, hex=true)：
/// 1. 把 j_rsakey 包进 PEM 头尾解析 PKIX 公钥（Go: pem.Decode + x509.ParsePKIXPublicKey）
/// 2. RSA-PKCS1v15 加密
/// 3. base64 编码后走 b64tohex 转换（不是标准 hex::encode）
fn rsa_encrypt_hex(data: &[u8], j_rsakey: &str) -> Result<String, String> {
    let pem_str = format!(
        "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----",
        j_rsakey
    );
    let pub_key = RsaPublicKey::from_public_key_pem(&pem_str)
        .map_err(|e| format!("189 公钥解析失败: {e}"))?;
    let mut rng = rand::thread_rng();
    let encrypted = pub_key
        .encrypt(&mut rng, Pkcs1v15Encrypt, data)
        .map_err(|e| format!("189 RSA 加密失败: {e}"))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&encrypted);
    Ok(b64tohex(&b64))
}

/// 对齐 Go 版 utils.Json.Get(body, "result").ToInt()：
/// 天翼接口的 result 字段可能是字符串 "0" 也可能是整数 0，两者都视为成功
fn json_result(v: &Value) -> Option<i64> {
    match v.get("result")? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

impl Cloud189 {
    pub fn new(username: String, password: String) -> Self {
        // 共享 cookie jar：登录态在两个 client 之间通用（对齐 Go resty.NewWithClient(d.client)）
        // 注意：不要把 Referer 放进 default_headers——reqwest 的 RequestBuilder::header()
        // 是 append 语义（Go resty SetHeaders 是覆盖），会导致登录请求携带两个 Referer，
        // 天翼服务端返回 400 Bad Request。Referer 改为逐请求显式设置。
        let jar = Arc::new(reqwest::cookie::Jar::default());
        let http = Client::builder()
            .cookie_provider(jar.clone())
            .user_agent(UA)
            .build()
            .unwrap();
        let http_no_redirect = Client::builder()
            .cookie_provider(jar)
            .user_agent(UA)
            .redirect(Policy::none())
            .build()
            .unwrap();
        Cloud189 {
            http,
            http_no_redirect,
            username,
            password,
        }
    }

    /// 对齐 Go newLogin()：统一登录换 session cookie
    pub async fn login(&self) -> Result<(), String> {
        // Step 0: 访问 loginUrl，若最终跳到 web/main 说明已登录
        let res = self
            .http
            .get(LOGIN_URL)
            .send()
            .await
            .map_err(|e| format!("189 登录页请求失败: {e}"))?;
        let final_url = res.url().clone();
        if final_url.as_str() == LOGIN_OK_URL {
            return Ok(());
        }
        let q: Vec<(String, String)> = final_url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        let get = |key: &str| -> String {
            q.iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        let (lt, req_id, app_id) = (get("lt"), get("reqId"), get("appId"));
        if lt.is_empty() || app_id.is_empty() {
            return Err(format!("189 登录跳转参数缺失: {final_url}"));
        }
        let login_headers = |r: reqwest::RequestBuilder| -> reqwest::RequestBuilder {
            r.header("lt", &lt)
                .header("reqid", &req_id)
                .header("referer", final_url.as_str())
                .header("origin", "https://open.e.189.cn")
        };
        // 登录接口返回非 JSON 时带上 HTTP 状态与响应片段，便于定位问题（对齐 Go 版 Debug 日志）
        let expect_json = async |res: reqwest::Response, step: &str| -> Result<Value, String> {
            let status = res.status();
            let body = res.text().await.map_err(|e| format!("189 {step} 读取响应失败: {e}"))?;
            serde_json::from_str::<Value>(&body).map_err(|_| {
                format!(
                    "189 {step} 响应非 JSON (HTTP {status}): {}",
                    &body[..body.len().min(200)]
                )
            })
        };

        // Step 1: appConf
        let res = login_headers(
            self.http
                .post("https://open.e.189.cn/api/logbox/oauth2/appConf.do"),
        )
        .form(&[("version", "2.0"), ("appKey", app_id.as_str())])
        .send()
        .await
        .map_err(|e| format!("189 appConf 请求失败: {e}"))?;
        let conf: Value = expect_json(res, "appConf").await?;
        if json_result(&conf).unwrap_or(-1) != 0 {
            let msg = conf.get("msg").and_then(|m| m.as_str()).unwrap_or("unknown");
            return Err(format!("189 appConf 失败: {msg}"));
        }
        let account_type = conf.pointer("/data/accountType").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let return_url = conf.pointer("/data/returnUrl").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let mail_suffix = conf.pointer("/data/mailSuffix").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let param_id = conf.pointer("/data/paramId").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let client_type = conf.pointer("/data/clientType").and_then(|v| v.as_i64()).unwrap_or(0).to_string();
        let is_oauth2 = conf.pointer("/data/isOauth2").and_then(|v| v.as_bool()).unwrap_or(false).to_string();

        // Step 2: encryptConf（取 RSA 公钥与加密前缀）
        let res = login_headers(
            self.http
                .post("https://open.e.189.cn/api/logbox/config/encryptConf.do"),
        )
        .form(&[("appId", app_id.as_str())])
        .send()
        .await
        .map_err(|e| format!("189 encryptConf 请求失败: {e}"))?;
        let enc: Value = expect_json(res, "encryptConf").await?;
        if json_result(&enc).unwrap_or(-1) != 0 {
            return Err(format!("189 获取 encryptConf 失败: {enc}"));
        }
        let pre = enc.pointer("/data/pre").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let pub_key = enc.pointer("/data/pubKey").and_then(|v| v.as_str()).unwrap_or("").to_string();

        // Step 3: RSA 加密账号密码并提交登录
        // 用不跟随重定向的 client 发送，避免登录接口 302 时丢失 JSON 响应体（对齐 Go resty 拿到 body 后解析的行为）
        let user_rsa = rsa_encrypt_hex(self.username.as_bytes(), &pub_key)?;
        let pass_rsa = rsa_encrypt_hex(self.password.as_bytes(), &pub_key)?;
        let res = login_headers(
            self.http_no_redirect
                .post("https://open.e.189.cn/api/logbox/oauth2/loginSubmit.do"),
        )
        .form(&[
            ("version", "v2.0"),
            ("apToken", ""),
            ("appKey", app_id.as_str()),
            ("accountType", account_type.as_str()),
            ("userName", &format!("{pre}{user_rsa}")),
            ("epd", &format!("{pre}{pass_rsa}")),
            ("captchaType", ""),
            ("validateCode", ""),
            ("smsValidateCode", ""),
            ("captchaToken", ""),
            ("returnUrl", return_url.as_str()),
            ("mailSuffix", mail_suffix.as_str()),
            ("dynamicCheck", "FALSE"),
            ("clientType", client_type.as_str()),
            ("cb_SaveName", "3"),
            ("isOauth2", is_oauth2.as_str()),
            ("state", ""),
            ("paramId", param_id.as_str()),
        ])
        .send()
        .await
        .map_err(|e| format!("189 登录请求失败: {e}"))?;

        // 若登录接口直接返回重定向（对齐 Go resty 自动跟随 302 的行为），跟随回调换取 session
        if res.status().is_redirection() {
            let loc = res
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            if loc.is_empty() {
                return Err("189 登录返回重定向但缺少 Location".to_string());
            }
            self.follow_login_redirect(&loc).await?;
            return Ok(());
        }

        let login: Value = expect_json(res, "loginSubmit").await?;
        if json_result(&login).unwrap_or(-1) != 0 {
            let msg = login.get("msg").and_then(|m| m.as_str()).unwrap_or("unknown");
            return Err(format!("189 登录失败: {msg}"));
        }
        // 关键步骤：跟随登录响应里的 toUrl 完成 OAuth 回调跳转，
        // 换取 cloud.189.cn 的有效 session cookie（对齐 Go resty 自动跟随重定向链的行为）。
        // 没有这一步，后续 API 请求会因 session 无效而报 InvalidSessionKey/Bad Request。
        let to_url = login.get("toUrl").and_then(|v| v.as_str()).unwrap_or("");
        if to_url.is_empty() {
            return Err(format!("189 登录成功但未返回 toUrl: {login}"));
        }
        self.follow_login_redirect(to_url).await?;
        Ok(())
    }

    /// 跟随登录回调重定向链（callbackUnify.action → ... → cloud.189.cn），把 session cookie 写入 jar
    async fn follow_login_redirect(&self, start_url: &str) -> Result<(), String> {
        let mut current = reqwest::Url::parse(start_url)
            .map_err(|e| format!("189 登录回调地址无效 ({start_url}): {e}"))?;
        for _ in 0..10 {
            let res = self
                .http_no_redirect
                .get(current.clone())
                .send()
                .await
                .map_err(|e| format!("189 登录回调请求失败 ({current}): {e}"))?;
            if res.status().is_redirection() {
                let loc = res
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                if loc.is_empty() {
                    return Err(format!("189 登录回调重定向缺少 Location: {current}"));
                }
                current = current
                    .join(&loc)
                    .map_err(|e| format!("189 登录回调重定向地址无效 ({loc}): {e}"))?;
                continue;
            }
            return Ok(());
        }
        Err("189 登录回调重定向次数超限".to_string())
    }

    /// 对齐 Go request()：带 noCache 参数，InvalidSessionKey 自动重登一次，res_code 检查
    async fn request(
        &self,
        method: Method,
        url: &str,
        query: Option<&[(&str, &str)]>,
        retried: bool,
    ) -> Result<Value, String> {
        let mut req = self
            .http
            .request(method.clone(), url)
            .header("Accept", "application/json;charset=UTF-8")
            .header("Referer", API_REFERER)
            .query(&[("noCache", random_param())]);
        if let Some(q) = query {
            req = req.query(q);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("189 请求失败: {e}"))?;
        let v: Value = resp
            .json()
            .await
            .map_err(|e| format!("189 响应解析失败: {e}"))?;
        let error_code = v.get("errorCode").and_then(|e| e.as_str()).unwrap_or("");
        if error_code == "InvalidSessionKey" {
            if retried {
                return Err("189 会话失效且重登失败".into());
            }
            self.login().await?;
            return Box::pin(self.request(method, url, query, true)).await;
        }
        let res_code = v.get("res_code").and_then(|c| c.as_i64()).unwrap_or(0);
        if res_code != 0 {
            let msg = v
                .get("res_message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            return Err(format!("189 接口错误(code={res_code}): {msg}"));
        }
        Ok(v)
    }

    /// 对齐 Go Init()：验证凭据（登录）
    pub async fn validate(&self) -> Result<(), String> {
        self.login().await?;
        Ok(())
    }

    fn normalize_fid(&self, fid: &str) -> String {
        if fid.is_empty() || fid == "0" {
            DEFAULT_ROOT.to_string()
        } else {
            fid.to_string()
        }
    }

    /// 对齐 Go getFiles()：listFiles.action 分页（60/页），count=0 停止
    pub async fn list(&self, parent_fid: &str) -> Result<Vec<Entry>, String> {
        let fid = self.normalize_fid(parent_fid);
        let mut files = Vec::new();
        let mut page: u32 = 1;
        loop {
            let page_str = page.to_string();
            let query = [
                ("pageSize", "60"),
                ("pageNum", page_str.as_str()),
                ("mediaType", "0"),
                ("folderId", fid.as_str()),
                ("iconOption", "5"),
                ("orderBy", "lastOpTime"),
                ("descending", "true"),
            ];
            let v = self
                .request(
                    Method::GET,
                    "https://cloud.189.cn/api/open/file/listFiles.action",
                    Some(&query),
                    false,
                )
                .await?;
            let ao = v.get("fileListAO").cloned().unwrap_or(Value::Null);
            let count = ao.get("count").and_then(|c| c.as_u64()).unwrap_or(0);
            if count == 0 {
                break;
            }
            if let Some(list) = ao.get("folderList").and_then(|l| l.as_array()) {
                for f in list {
                    files.push(Entry {
                        fid: f
                            .get("id")
                            .and_then(|i| i.as_i64())
                            .unwrap_or(0)
                            .to_string(),
                        name: f.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                        size: 0,
                        is_dir: true,
                        updated_at: f
                            .get("lastOpTime")
                            .and_then(|t| t.as_str())
                            .and_then(parse_cn_time),
                        etag: None,
                        s3_key_flag: None,
                        file_type: None,
                        extra: None,
                    });
                }
            }
            if let Some(list) = ao.get("fileList").and_then(|l| l.as_array()) {
                for f in list {
                    files.push(Entry {
                        fid: f
                            .get("id")
                            .and_then(|i| i.as_i64())
                            .unwrap_or(0)
                            .to_string(),
                        name: f.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                        size: f.get("size").and_then(|s| s.as_u64()).unwrap_or(0),
                        is_dir: false,
                        updated_at: f
                            .get("lastOpTime")
                            .and_then(|t| t.as_str())
                            .and_then(parse_cn_time),
                        etag: None,
                        s3_key_flag: None,
                        file_type: None,
                        extra: None,
                    });
                }
            }
            page += 1;
        }
        Ok(files)
    }

    /// 对齐 Go Link()：getFileInfo -> downloadUrl -> 跟一次 302 拿最终直链
    pub async fn download(&self, e: &Entry) -> Result<DownloadInfo, String> {
        let v = self
            .request(
                Method::GET,
                "https://cloud.189.cn/api/portal/getFileInfo.action",
                Some(&[("fileId", e.fid.as_str())]),
                false,
            )
            .await?;
        let first_url = v
            .get("downloadUrl")
            .and_then(|u| u.as_str())
            .unwrap_or("");
        if first_url.is_empty() {
            return Err("189 未返回下载直链".into());
        }
        // Go 版 downloadUrl 字段缺协议头，补 "https:"
        let full = if first_url.starts_with("http") {
            first_url.to_string()
        } else {
            format!("https:{first_url}")
        };
        let mut final_url = full.clone();
        if let Ok(res) = self.http_no_redirect.get(&full).send().await {
            if res.status().as_u16() == 302 {
                if let Some(loc) = res.headers().get("location").and_then(|l| l.to_str().ok()) {
                    final_url = loc.to_string();
                }
            }
        }
        let final_url = final_url.replacen("http://", "https://", 1);
        Ok(DownloadInfo {
            url: final_url,
            headers: vec![],
            proxy: false,
            local_path: None,
        })
    }
}
