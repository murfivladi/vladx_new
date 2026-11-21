//! Установка: разрешить → скачать недостающее → разложить в `пакеты/<имя>/`
//! → убрать лишнее → записать vladx.lock. Один код для CLI и браузера.

use crate::bundle::Bundle;
use crate::lock::{Lock, LockEntry};
use crate::manifest::{self, Manifest};
use crate::resolve::Resolver;
use crate::{Fs, Registry};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use vladx_syntax::{Dictionary, Lang};

/// Что установлено в папке пакетов (как node_modules/.package-lock.json).
pub const STATE: &str = ".vladpm.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    packages: BTreeMap<String, LockEntry>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Report {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: usize,
    pub warnings: Vec<String>,
    /// Итог: имя → версия.
    pub packages: BTreeMap<String, String>,
}

impl Report {
    pub fn summary(&self) -> String {
        let mut s = String::new();
        for a in &self.added {
            s.push_str(&format!("+ {a}\n"));
        }
        for u in &self.updated {
            s.push_str(&format!("~ {u}\n"));
        }
        for r in &self.removed {
            s.push_str(&format!("- {r}\n"));
        }
        for w in &self.warnings {
            s.push_str(&format!("предупреждение: {w}\n"));
        }
        let n = self.packages.len();
        if self.added.is_empty() && self.updated.is_empty() && self.removed.is_empty() {
            s.push_str(&format!("всё уже установлено ({n} пак.)\n"));
        } else {
            s.push_str(&format!(
                "установлено: {} новых, {} обновлено, {} удалено; всего пакетов: {n}\n",
                self.added.len(),
                self.updated.len(),
                self.removed.len()
            ));
        }
        s
    }
}

pub struct Project {
    pub src: String,
    pub manifest: Manifest,
    pub lang: Lang,
    pub modules_dir: String,
}

pub fn read_project(fs: &mut dyn Fs, dict: &Dictionary) -> Result<Project, String> {
    let src = fs.read(manifest::FILE).ok_or("нет vladx.toml — создайте проект: vladpm init")?;
    let m = manifest::parse(&src, dict)?;
    let lang = m.lang.unwrap_or(Lang::Ru);
    Ok(Project { modules_dir: manifest::modules_dir(dict, lang), src, manifest: m, lang })
}

fn pkg_dir(mdir: &str, name: &str) -> String {
    format!("{mdir}/{name}")
}

/// Установить зависимости из vladx.toml. `unlock` — имена для обновления
/// (пусто — держаться за vladx.lock; `None` в `update_all` — обновить всё).
pub fn install(
    fs: &mut dyn Fs,
    reg: &mut dyn Registry,
    dict: &Dictionary,
    unlock: &BTreeSet<String>,
    update_all: bool,
) -> Result<Report, String> {
    let proj = read_project(fs, dict)?;
    let lock = match fs.read(crate::lock::FILE) {
        Some(s) if !update_all => Lock::parse(&s)?,
        _ => Lock::default(),
    };
    let mut res = Resolver::new(reg);
    let resolved = res.resolve(&proj.manifest.dependencies, &lock, unlock)?;
    // предупреждения о снятых с поддержки версиях
    let mut warnings = vec![];
    for (n, e) in &resolved {
        if let Ok(p) = res.packument(n) {
            if let Some(Some(msg)) = p.versions.get(&e.version).map(|m| m.deprecated.clone()) {
                warnings.push(format!("{n}@{} устарел: {msg}", e.version));
            }
        }
    }
    let mdir = proj.modules_dir.clone();
    let state_path = format!("{mdir}/{STATE}");
    let state: State = fs.read(&state_path).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let mut rep = Report { warnings, ..Report::default() };
    for (name, e) in &resolved {
        let dir = pkg_dir(&mdir, name);
        let have = state.packages.get(name);
        let intact = fs.read(&format!("{dir}/{}", manifest::FILE)).is_some();
        if have.is_some_and(|h| h.version == e.version && h.integrity == e.integrity) && intact {
            rep.unchanged += 1;
            continue;
        }
        let b: Bundle = res.registry.fetch(name, &e.version)?;
        b.verify()?;
        if b.name != *name || b.version != e.version {
            return Err(format!("реестр прислал {}@{} вместо {name}@{}", b.name, b.version, e.version));
        }
        if !e.integrity.is_empty() && b.integrity != e.integrity {
            return Err(format!("{name}@{}: контрольная сумма не совпала с vladx.lock — пакет подменён?", e.version));
        }
        fs.remove_dir(&dir)?;
        for (p, c) in &b.files {
            fs.write(&format!("{dir}/{p}"), c)?;
        }
        match have {
            Some(h) if h.version != e.version => rep.updated.push(format!("{name} {} → {}", h.version, e.version)),
            Some(_) => rep.updated.push(format!("{name}@{}", e.version)),
            None => rep.added.push(format!("{name}@{}", e.version)),
        }
    }
    for (name, h) in &state.packages {
        if !resolved.contains_key(name) {
            fs.remove_dir(&pkg_dir(&mdir, name))?;
            rep.removed.push(format!("{name}@{}", h.version));
        }
    }
    let mut final_lock = Lock { registry: lock.registry.clone(), ..Lock::default() };
    for (n, e) in &resolved {
        let mut e = e.clone();
        if e.integrity.is_empty() {
            if let Ok(p) = res.packument(n) {
                if let Some(m) = p.versions.get(&e.version) {
                    e.integrity = m.integrity.clone();
                }
            }
        }
        rep.packages.insert(n.clone(), e.version.clone());
        final_lock.packages.insert(n.clone(), e);
    }
    let st = State { packages: final_lock.packages.clone() };
    if resolved.is_empty() && state.packages.is_empty() {
        // нечего класть: папку пакетов не создаём
    } else {
        fs.write(&state_path, &(serde_json::to_string_pretty(&st).unwrap_or_default() + "\n"))?;
    }
    fs.write(crate::lock::FILE, &final_lock.render())?;
    Ok(rep)
}

/// `имя@диапазон` → (имя, диапазон). Область `@автор/имя@^1` тоже понимается.
pub fn split_spec(spec: &str) -> (String, Option<String>) {
    let (scope, rest) = if let Some(r) = spec.strip_prefix('@') { ("@", r) } else { ("", spec) };
    match rest.split_once('@') {
        Some((n, r)) if !r.is_empty() => (format!("{scope}{n}"), Some(r.to_string())),
        Some((n, _)) => (format!("{scope}{n}"), None),
        None => (spec.to_string(), None),
    }
}

/// Добавить зависимости (`vladpm add имя[@диапазон]…`): правка vladx.toml
/// с сохранением комментариев, затем установка.
pub fn add(
    fs: &mut dyn Fs,
    reg: &mut dyn Registry,
    dict: &Dictionary,
    specs: &[String],
) -> Result<Report, String> {
    let proj = read_project(fs, dict)?;
    let mut src = proj.src.clone();
    let mut unlock = BTreeSet::new();
    for spec in specs {
        let (name, range) = split_spec(spec);
        manifest::valid_name(&name)?;
        let range = match range {
            Some(r) => {
                crate::semver::Range::parse(&r)?;
                r
            }
            None => {
                let p = reg.packument(&name)?;
                let latest = p.latest().ok_or_else(|| format!("у пакета «{name}» нет опубликованных версий"))?;
                format!("^{latest}")
            }
        };
        src = manifest::set_dependency(&src, dict, proj.lang, &name, Some(&range));
        unlock.insert(name);
    }
    fs.write(manifest::FILE, &src)?;
    install(fs, reg, dict, &unlock, false)
}

/// Убрать зависимости (`vladpm remove имя…`).
pub fn remove(fs: &mut dyn Fs, reg: &mut dyn Registry, dict: &Dictionary, names: &[String]) -> Result<Report, String> {
    let proj = read_project(fs, dict)?;
    let mut src = proj.src.clone();
    for n in names {
        if !proj.manifest.dependencies.contains_key(n) {
            return Err(format!("«{n}» нет в зависимостях vladx.toml"));
        }
        src = manifest::set_dependency(&src, dict, proj.lang, n, None);
    }
    fs.write(manifest::FILE, &src)?;
    install(fs, reg, dict, &BTreeSet::new(), false)
}
