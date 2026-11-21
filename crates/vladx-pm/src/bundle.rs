//! Пакет в реестре — один JSON: имя, версия и файлы с содержимым.
//! Целостность — sha256 по отсортированным путям и содержимому.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use vladx_syntax::{Dictionary, Lang};

/// Самый большой пакет, который принимает реестр.
pub const MAX_BYTES: usize = 1 << 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bundle {
    pub name: String,
    pub version: String,
    pub files: BTreeMap<String, String>,
    pub integrity: String,
}

pub fn integrity(files: &BTreeMap<String, String>) -> String {
    let mut h = Sha256::new();
    for (p, c) in files {
        h.update(p.as_bytes());
        h.update([0]);
        h.update(c.as_bytes());
        h.update([0]);
    }
    let d = h.finalize();
    let mut s = String::from("sha256-");
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

impl Bundle {
    pub fn verify(&self) -> Result<(), String> {
        let got = integrity(&self.files);
        if got != self.integrity {
            return Err(format!("пакет {}@{} повреждён: контрольная сумма не совпала", self.name, self.version));
        }
        for p in self.files.keys() {
            safe_path(p)?;
        }
        Ok(())
    }

    pub fn size(&self) -> usize {
        self.files.iter().map(|(p, c)| p.len() + c.len()).sum()
    }
}

/// Путь внутри пакета: без «..», без абсолютных путей и скрытых частей.
pub fn safe_path(p: &str) -> Result<(), String> {
    let bad = p.is_empty()
        || p.starts_with('/')
        || p.contains('\\')
        || p.contains(':')
        || p.split('/').any(|s| s.is_empty() || s == "." || s == ".." || s.starts_with('.'));
    if bad {
        return Err(format!("путь «{p}» в пакете недопустим"));
    }
    Ok(())
}

/// Что попадает в пакет при публикации.
pub fn publishable(path: &str, modules_dirs: &[String]) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.iter().any(|s| s.starts_with('.')) {
        return false;
    }
    if parts.len() > 1 && modules_dirs.iter().any(|m| m == parts[0] || parts[0] == "vladx_modules") {
        return false;
    }
    if parts[0] == "vladx-files" || parts[0] == "target" {
        return false;
    }
    let name = parts.last().copied().unwrap_or("");
    if name == crate::manifest::FILE {
        return parts.len() == 1;
    }
    if name == "vladx.lock" {
        return false;
    }
    let upper = name.to_uppercase();
    if upper.starts_with("LICENSE") || upper.starts_with("LICENCE") || upper.starts_with("README") {
        return true;
    }
    [".vlstd", ".vlhmn", ".md", ".txt", ".json", ".csv"].iter().any(|e| name.ends_with(e))
}

/// Собрать пакет для публикации из файлов проекта (пути — от корня, через «/»).
pub fn pack(files: Vec<(String, String)>, dict: &Dictionary) -> Result<(Bundle, crate::Manifest), String> {
    let mdirs = crate::manifest::modules_dir_names(dict);
    let mut map = BTreeMap::new();
    for (p, c) in files {
        if publishable(&p, &mdirs) {
            safe_path(&p)?;
            map.insert(p, c);
        }
    }
    let src = map.get(crate::manifest::FILE).ok_or("в пакете нет vladx.toml")?;
    let m = crate::manifest::parse(src, dict)?;
    let name = m.name.clone().ok_or("в vladx.toml нет имени пакета (поле «имя»)")?;
    crate::manifest::valid_name(&name)?;
    let version = m.version.clone().ok_or("в vladx.toml нет версии (поле «версия», например \"1.0.0\")")?;
    crate::semver::Version::parse(&version)?;
    let lang = m.lang.unwrap_or(Lang::Ru);
    let main = m.main.clone().unwrap_or_else(|| default_main(dict, lang));
    if !map.contains_key(&main) {
        return Err(format!("точка входа «{main}» не найдена в пакете (поле «главный» в vladx.toml)"));
    }
    for (p, c) in &map {
        if p.ends_with(".vlstd") {
            if let Err(d) = vladx_syntax::parse(c, lang, dict) {
                let at = format!(":{}:{}", d.line, d.col);
                return Err(format!("{p}{at}: {} {}", d.code, d.message));
            }
        }
    }
    let b = Bundle { name, version, integrity: integrity(&map), files: map };
    if b.size() > MAX_BYTES {
        return Err(format!("пакет больше {} КБ — уберите лишние файлы", MAX_BYTES / 1024));
    }
    Ok((b, m))
}

/// Точка входа по умолчанию — как в `vladx init`.
pub fn default_main(_dict: &Dictionary, lang: Lang) -> String {
    match lang {
        Lang::Ru => "главная.vlstd".into(),
        Lang::It => "principale.vlstd".into(),
        Lang::Scn => "principali.vlstd".into(),
    }
}
