//! vladpm — менеджер пакетов VladX, как npm.
//!
//! Здесь нет сети и диска: реестр и файлы приходят через трейты [`Registry`]
//! и [`Fs`]. Один и тот же код работает в CLI (`vladpm`, `vladx pm`) и в
//! браузерной площадке (WASM).

pub mod bundle;
pub mod install;
pub mod lock;
pub mod manifest;
pub mod resolve;
pub mod semver;

pub use bundle::Bundle;
pub use install::{add, install, remove, Report};
pub use lock::{Lock, LockEntry};
pub use manifest::Manifest;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Реестр по умолчанию — тот же сайт, где площадка.
pub const DEFAULT_REGISTRY: &str = "https://vladinc.ru/vladx";

/// Сведения о версии пакета в реестре.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct VersionMeta {
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
    #[serde(default)]
    pub integrity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(default)]
    pub published: u64,
}

/// Карточка пакета: все версии (как packument в npm).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Packument {
    pub name: String,
    #[serde(default)]
    pub versions: BTreeMap<String, VersionMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub owners: Vec<String>,
}

impl Packument {
    /// Последняя версия без предрелиза (или последний предрелиз, если других нет).
    pub fn latest(&self) -> Option<String> {
        let parsed: Vec<(semver::Version, &String)> =
            self.versions.keys().filter_map(|v| semver::Version::parse(v).ok().map(|p| (p, v))).collect();
        parsed
            .iter()
            .filter(|(p, _)| p.pre.is_empty())
            .max_by(|a, b| a.0.cmp(&b.0))
            .or_else(|| parsed.iter().max_by(|a, b| a.0.cmp(&b.0)))
            .map(|(_, v)| v.to_string())
    }
}

pub trait Registry {
    fn packument(&mut self, name: &str) -> Result<Packument, String>;
    fn fetch(&mut self, name: &str, version: &str) -> Result<Bundle, String>;
}

/// Файлы проекта; пути — от корня проекта, через «/».
pub trait Fs {
    fn read(&mut self, path: &str) -> Option<String>;
    fn write(&mut self, path: &str, content: &str) -> Result<(), String>;
    /// Удалить папку со всем содержимым (нет папки — не ошибка).
    fn remove_dir(&mut self, path: &str) -> Result<(), String>;
}

/// Файлы в памяти — для тестов.
#[derive(Debug, Default, Clone)]
pub struct MemFs {
    pub files: BTreeMap<String, String>,
}

impl Fs for MemFs {
    fn read(&mut self, path: &str) -> Option<String> {
        self.files.get(path).cloned()
    }
    fn write(&mut self, path: &str, content: &str) -> Result<(), String> {
        self.files.insert(path.to_string(), content.to_string());
        Ok(())
    }
    fn remove_dir(&mut self, path: &str) -> Result<(), String> {
        let pre = format!("{path}/");
        self.files.retain(|k, _| !k.starts_with(&pre));
        Ok(())
    }
}

/// Реестр в памяти — для тестов и офлайн-проверок.
#[derive(Debug, Default, Clone)]
pub struct MemRegistry {
    pub packages: BTreeMap<String, (Packument, BTreeMap<String, Bundle>)>,
    pub fetches: usize,
}

impl MemRegistry {
    pub fn publish(&mut self, b: Bundle, m: &Manifest) {
        let e = self.packages.entry(b.name.clone()).or_insert_with(|| (Packument { name: b.name.clone(), ..Packument::default() }, BTreeMap::new()));
        e.0.versions.insert(
            b.version.clone(),
            VersionMeta {
                dependencies: m.dependencies.clone(),
                integrity: b.integrity.clone(),
                description: m.description.clone(),
                lang: m.lang.map(|l| l.code().to_string()),
                ..VersionMeta::default()
            },
        );
        e.1.insert(b.version.clone(), b);
    }
}

impl Registry for MemRegistry {
    fn packument(&mut self, name: &str) -> Result<Packument, String> {
        self.packages.get(name).map(|p| p.0.clone()).ok_or_else(|| format!("пакет «{name}» не найден в реестре"))
    }
    fn fetch(&mut self, name: &str, version: &str) -> Result<Bundle, String> {
        self.fetches += 1;
        self.packages
            .get(name)
            .and_then(|p| p.1.get(version).cloned())
            .ok_or_else(|| format!("{name}@{version} не найден в реестре"))
    }
}
