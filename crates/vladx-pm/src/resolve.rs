//! Разрешение зависимостей: одна версия на имя — наибольшая, подходящая всем
//! диапазонам; версии из vladx.lock держатся, пока подходят.

use crate::lock::{Lock, LockEntry};
use crate::semver::{Range, Version};
use crate::{Packument, Registry};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub struct Resolver<'a> {
    pub registry: &'a mut dyn Registry,
    cache: HashMap<String, Packument>,
}

/// Кто чего требует: (диапазон, кто требует).
type Wants = BTreeMap<String, Vec<(String, String)>>;

impl<'a> Resolver<'a> {
    pub fn new(registry: &'a mut dyn Registry) -> Self {
        Resolver { registry, cache: HashMap::new() }
    }

    pub fn packument(&mut self, name: &str) -> Result<&Packument, String> {
        if !self.cache.contains_key(name) {
            let p = self.registry.packument(name)?;
            self.cache.insert(name.to_string(), p);
        }
        Ok(&self.cache[name])
    }

    /// `root` — зависимости проекта, `lock` — прежние точные версии,
    /// `unlock` — имена, для которых lock не учитывается (vladpm update).
    pub fn resolve(
        &mut self,
        root: &BTreeMap<String, String>,
        lock: &Lock,
        unlock: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, LockEntry>, String> {
        let mut chosen: BTreeMap<String, LockEntry> = BTreeMap::new();
        for _round in 0..200 {
            let mut wants: Wants = BTreeMap::new();
            for (n, r) in root {
                wants.entry(n.clone()).or_default().push((r.clone(), "проект".into()));
            }
            for (pn, e) in &chosen {
                for (n, r) in &e.dependencies {
                    wants.entry(n.clone()).or_default().push((r.clone(), format!("{pn}@{}", e.version)));
                }
            }
            let mut next: BTreeMap<String, LockEntry> = BTreeMap::new();
            for (name, ranges) in &wants {
                crate::manifest::valid_name(name)?;
                let parsed: Vec<Range> = ranges
                    .iter()
                    .map(|(r, who)| Range::parse(r).map_err(|e| format!("{who}: «{name}» = \"{r}\": {e}")))
                    .collect::<Result<_, _>>()?;
                let ok = |v: &str| Version::parse(v).map(|v| parsed.iter().all(|r| r.matches(&v))).unwrap_or(false);
                // 1) уже выбранная версия; 2) версия из lock; 3) наибольшая из реестра
                if let Some(e) = chosen.get(name).filter(|e| ok(&e.version)) {
                    next.insert(name.clone(), e.clone());
                    continue;
                }
                if !unlock.contains(name) {
                    if let Some(e) = lock.packages.get(name).filter(|e| ok(&e.version)) {
                        next.insert(name.clone(), e.clone());
                        continue;
                    }
                }
                let p = self.packument(name)?;
                let best = p
                    .versions
                    .iter()
                    .filter(|(v, _)| ok(v))
                    .filter_map(|(v, m)| Version::parse(v).ok().map(|pv| (pv, v, m)))
                    .max_by(|a, b| a.0.cmp(&b.0));
                match best {
                    Some((_, v, m)) => {
                        next.insert(
                            name.clone(),
                            LockEntry { version: v.clone(), integrity: m.integrity.clone(), dependencies: m.dependencies.clone() },
                        );
                    }
                    None => {
                        let all: Vec<String> = ranges.iter().map(|(r, who)| format!("\"{r}\" ({who})")).collect();
                        let have: Vec<&String> = p.versions.keys().collect();
                        return Err(if have.is_empty() {
                            format!("у пакета «{name}» нет опубликованных версий")
                        } else if ranges.len() > 1 {
                            format!(
                                "конфликт версий «{name}»: нужна версия, подходящая всем: {}; в реестре: {}",
                                all.join(", "),
                                have.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                            )
                        } else {
                            format!(
                                "у «{name}» нет версии под {}; в реестре: {}",
                                all.join(", "),
                                have.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                            )
                        });
                    }
                }
            }
            if next == chosen {
                return Ok(chosen);
            }
            chosen = next;
        }
        Err("зависимости не сходятся: слишком много перестановок версий".into())
    }
}
