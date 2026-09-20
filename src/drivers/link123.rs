//! 123PanLink driver - see local openlist-rs for full source
// Temporary stub - full implementation is in the local workspace.
// Please pull from the conversation artifacts or re-apply the full port.

use super::DownloadInfo;
use crate::config::Entry;

pub struct Pan123Link;

impl Pan123Link {
    pub fn new(
        _origin_urls: String,
        _private_key: String,
        _uid: u64,
        _valid_duration_min: i64,
    ) -> Result<Self, String> {
        Err("123PanLink: full source pending push; see local artifacts".into())
    }
    pub fn validate(&self) -> Result<(), String> { Ok(()) }
    pub async fn list(&self, _: &str) -> Result<Vec<Entry>, String> { Ok(vec![]) }
    pub async fn download(&self, _: &Entry) -> Result<DownloadInfo, String> {
        Err("not implemented".into())
    }
    pub async fn mkdir(&self, _: &str, _: &str) -> Result<(), String> {
        Err("readonly".into())
    }
    pub async fn rename(&self, _: &str, _: &Entry, _: &str) -> Result<(), String> {
        Err("readonly".into())
    }
    pub async fn move_entry(&self, _: &str, _: &Entry, _: &str) -> Result<(), String> {
        Err("readonly".into())
    }
    pub async fn copy(&self, _: &str, _: &Entry, _: &str) -> Result<(), String> {
        Err("readonly".into())
    }
    pub async fn remove(&self, _: &str, _: &Entry) -> Result<(), String> {
        Err("readonly".into())
    }
    pub async fn put(&self, _: &str, _: super::PutInput) -> Result<(), String> {
        Err("readonly".into())
    }
}
