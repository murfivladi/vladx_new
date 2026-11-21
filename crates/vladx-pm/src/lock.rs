//! `vladx.lock` — точные версии всех пакетов, как package-lock.json.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const FILE: &str = "vladx.lock";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LockEntry {
    pub version: String,
    pub integrity: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Lock {
    pub lockfile_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    #[serde(default)]
    pub packages: BTreeMap<String, LockEntry>,
}

impl Default for Lock {
    fn default() -> Self {
        Lock { lockfile_version: 1, registry: None, packages: BTreeMap::new() }
    }
}

impl Lock {
    pub fn parse(src: &str) -> Result<Lock, String> {
        serde_json::from_str(src).map_err(|e| format!("vladx.lock повреждён: {e}"))
    }

    pub fn render(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }
}
