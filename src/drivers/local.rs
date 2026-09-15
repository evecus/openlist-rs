//! 本机存储驱动（对齐 Go 版 drivers/local，只读浏览 + 下载）
//!
//! - fid 直接使用本机文件路径（根目录 = 配置的 root_path）
//! - list：read_dir 枚举
//! - download：返回 local_path，由 api 层直接从磁盘流式读取（不走 reqwest）

use super::DownloadInfo;
use crate::config::Entry;
use std::path::{Path, PathBuf};

pub struct Local {
    root_path: String,
}

impl Local {
    pub fn new(root_path: String) -> Self {
        Local { root_path }
    }

    /// 对齐 Init()：校验挂载目录存在且为目录
    pub fn validate(&self) -> Result<(), String> {
        let p = Path::new(&self.root_path);
        if !p.exists() {
            return Err(format!("本机存储路径不存在: {}", self.root_path));
        }
        if !p.is_dir() {
            return Err(format!("本机存储路径不是目录: {}", self.root_path));
        }
        Ok(())
    }

    /// fid("0"/"") -> 根目录；其余 fid 即目录路径
    fn resolve_dir(&self, fid: &str) -> PathBuf {
        if fid.is_empty() || fid == "0" {
            PathBuf::from(&self.root_path)
        } else {
            PathBuf::from(fid)
        }
    }

    pub fn list(&self, parent_fid: &str) -> Result<Vec<Entry>, String> {
        let dir = self.resolve_dir(parent_fid);
        let rd = std::fs::read_dir(&dir).map_err(|e| format!("读取目录 {} 失败: {e}", dir.display()))?;
        let mut out = Vec::new();
        for entry in rd {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            // 元数据失败（如悬空符号链接）直接跳过
            let Ok(meta) = entry.metadata() else { continue };
            let is_dir = meta.is_dir();
            let size = if is_dir { 0 } else { meta.len() };
            let updated_at = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64);
            out.push(Entry {
                fid: path.to_string_lossy().to_string(),
                name,
                size,
                is_dir,
                updated_at,
                etag: None,
                s3_key_flag: None,
                file_type: None,
                extra: None,
            });
        }
        Ok(out)
    }

    pub fn download(&self, e: &Entry) -> Result<DownloadInfo, String> {
        if e.is_dir {
            return Err("目录无法下载".into());
        }
        let p = PathBuf::from(&e.fid);
        if !p.is_file() {
            return Err(format!("文件不存在: {}", e.fid));
        }
        Ok(DownloadInfo {
            url: String::new(),
            headers: vec![],
            proxy: true,
            local_path: Some(e.fid.clone()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_dir() {
        let d = Local::new("/tmp/root".into());
        assert_eq!(d.resolve_dir("0"), PathBuf::from("/tmp/root"));
        assert_eq!(d.resolve_dir(""), PathBuf::from("/tmp/root"));
        assert_eq!(d.resolve_dir("/tmp/root/sub"), PathBuf::from("/tmp/root/sub"));
    }
}
