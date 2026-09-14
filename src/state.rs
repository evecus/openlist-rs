use crate::config::{Entry, Store};
use crate::drivers::Driver;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

pub(crate) struct AuthCfg {
    pub(crate) user: String,
    pub(crate) pass: String,
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) store: Arc<Store>,
    pub(crate) drivers: Arc<Mutex<HashMap<String, Arc<Driver>>>>,
    pub(crate) auth: Option<Arc<AuthCfg>>,
    pub(crate) sessions: Arc<Mutex<HashSet<String>>>,
    /// OpenList 兼容层：归一化路径 -> (账号id, Entry)，浏览时逐步注册
    pub(crate) index: Arc<Mutex<HashMap<String, (String, Entry)>>>,
}

impl AppState {
    pub(crate) fn new(
        config_path: &str,
        web_user: Option<String>,
        web_pass: Option<String>,
    ) -> Self {
        let store = Arc::new(Store::load(config_path));
        // 鉴权：--web-user/env 提供用户名即启用；密码缺省则随机生成打印
        let env_user = std::env::var("OPENLIST_WEB_USER")
            .ok()
            .filter(|s| !s.is_empty());
        let env_pass = std::env::var("OPENLIST_WEB_PASS")
            .ok()
            .filter(|s| !s.is_empty());
        let auth =
            match (web_user.or(env_user), web_pass.or(env_pass)) {
                (Some(user), pass) => {
                    let pass = pass.unwrap_or_else(|| {
                        let generated = format!(
                            "{}{}",
                            uuid::Uuid::new_v4().simple(),
                            uuid::Uuid::new_v4().simple()
                        )[..16]
                            .to_string();
                        println!("未指定 --web-pass，已自动生成面板密码: {generated}");
                        generated
                    });
                    Some(Arc::new(AuthCfg { user, pass }))
                }
                _ => {
                    println!(
                        "未设置面板账号（--web-user/--web-pass 或 OPENLIST_WEB_USER/OPENLIST_WEB_PASS），面板免登录访问"
                    );
                    None
                }
            };
        AppState {
            store,
            drivers: Arc::new(Mutex::new(HashMap::new())),
            auth,
            sessions: Arc::new(Mutex::new(HashSet::new())),
            index: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 按账号 id 惰性构建驱动（首次使用时验证凭据并缓存）
    pub(crate) async fn get_driver(&self, id: &str) -> Result<Arc<Driver>, String> {
        {
            let drivers = self.drivers.lock().unwrap();
            if let Some(d) = drivers.get(id) {
                return Ok(d.clone());
            }
        }
        let cred = {
            let data = self.store.data.lock().unwrap();
            data.accounts
                .iter()
                .find(|a| a.id == id)
                .map(|a| (a.id.clone(), a.cred.clone()))
        };
        let Some((id, cred)) = cred else {
            return Err("账号不存在".into());
        };
        let d = Arc::new(Driver::new(&id, &cred, self.store.clone()).await?);
        self.drivers.lock().unwrap().insert(id, d.clone());
        Ok(d)
    }
}
