//! Словарь ключевых слов. Источник — `spec/keywords.toml`; в коде нет словоформ.

use std::collections::HashMap;
use unicode_normalization::UnicodeNormalization;

/// Язык ключевых слов проекта.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Ru,
    It,
    Scn,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::Ru, Lang::It, Lang::Scn];

    pub fn code(self) -> &'static str {
        match self {
            Lang::Ru => "ru",
            Lang::It => "it",
            Lang::Scn => "scn",
        }
    }

    pub fn from_code(s: &str) -> Option<Lang> {
        match s {
            "ru" => Some(Lang::Ru),
            "it" => Some(Lang::It),
            "scn" => Some(Lang::Scn),
            _ => None,
        }
    }

    /// Нумерация ABI: 0 — ru, 1 — it, 2 — scn.
    pub fn from_index(i: u32) -> Option<Lang> {
        Lang::ALL.get(i as usize).copied()
    }
}

/// Что означает слово или фраза.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Meaning {
    /// Операция или слово выражения: `ctrl.if`, `lit.true`, `cmp.gt`, ...
    Op(String),
    /// Служебное слово конструкции: (`ctrl.if`, `then`).
    Sup(String, String),
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    pub status: String,
    /// `true` — запись из `[[op]]` (одна из 21 операции), `false` — `[[word]]`.
    pub is_op: bool,
    pub forms: HashMap<Lang, Vec<String>>,
    pub support: Vec<(String, HashMap<Lang, Vec<String>>)>,
}

#[derive(Debug, Clone)]
pub struct TypeEntry {
    pub id: String,
    pub forms: HashMap<Lang, Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct Dictionary {
    pub schema_version: u32,
    pub default_lang: Lang,
    pub entries: Vec<Entry>,
    pub types: Vec<TypeEntry>,
    /// Слова манифеста vladx.toml (разделы и поля) — `[[manifest]]`.
    pub manifest: Vec<TypeEntry>,
    phrases: HashMap<Lang, HashMap<Vec<String>, Vec<Meaning>>>,
    max_words: usize,
}

/// Нормализация слова для сравнения: NFC + нижний регистр.
pub fn norm(word: &str) -> String {
    word.nfc().collect::<String>().to_lowercase()
}

fn split_phrase(p: &str) -> Vec<String> {
    p.split_whitespace().map(norm).collect()
}

fn str_list(v: Option<&toml::Value>, ctx: &str) -> Result<Vec<String>, String> {
    match v {
        None => Ok(vec![]),
        Some(toml::Value::Array(a)) => a
            .iter()
            .map(|x| x.as_str().map(|s| s.to_string()).ok_or_else(|| format!("{ctx}: ожидалась строка")))
            .collect(),
        Some(_) => Err(format!("{ctx}: ожидался список строк")),
    }
}

fn lang_forms(t: &toml::Table, ctx: &str) -> Result<HashMap<Lang, Vec<String>>, String> {
    let mut m = HashMap::new();
    for l in Lang::ALL {
        m.insert(l, str_list(t.get(l.code()), &format!("{ctx}.{}", l.code()))?);
    }
    Ok(m)
}

impl Dictionary {
    /// Словарь, вшитый в бинарь (`spec/keywords.toml`).
    pub fn builtin() -> Dictionary {
        Dictionary::from_toml(crate::BUILTIN_KEYWORDS).expect("встроенный словарь корректен")
    }

    pub fn from_toml(src: &str) -> Result<Dictionary, String> {
        let root: toml::Table = src.parse().map_err(|e| format!("словарь не разобран: {e}"))?;
        let schema_version = root.get("schema_version").and_then(|v| v.as_integer()).unwrap_or(1) as u32;
        let default_lang = root
            .get("default_lang")
            .and_then(|v| v.as_str())
            .and_then(Lang::from_code)
            .unwrap_or(Lang::Ru);
        let mut entries = vec![];
        for (section, is_op) in [("op", true), ("word", false)] {
            let Some(arr) = root.get(section) else { continue };
            let arr = arr.as_array().ok_or_else(|| format!("[[{section}]] должен быть списком таблиц"))?;
            for item in arr {
                let t = item.as_table().ok_or_else(|| format!("[[{section}]]: ожидалась таблица"))?;
                let id = t
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| format!("[[{section}]] без id"))?
                    .to_string();
                let status = t.get("status").and_then(|v| v.as_str()).unwrap_or("fixed").to_string();
                let forms = lang_forms(t, &id)?;
                let mut support = vec![];
                if let Some(sup) = t.get("support").and_then(|v| v.as_table()) {
                    for (role, st) in sup {
                        let st = st.as_table().ok_or_else(|| format!("{id}.support.{role}: ожидалась таблица"))?;
                        support.push((role.clone(), lang_forms(st, &format!("{id}.support.{role}"))?));
                    }
                }
                entries.push(Entry { id, status, is_op, forms, support });
            }
        }
        let mut types = vec![];
        if let Some(arr) = root.get("type").and_then(|v| v.as_array()) {
            for item in arr {
                let t = item.as_table().ok_or("[[type]]: ожидалась таблица")?;
                let id = t.get("id").and_then(|v| v.as_str()).ok_or("[[type]] без id")?.to_string();
                types.push(TypeEntry { forms: lang_forms(t, &id)?, id });
            }
        }

        let mut manifest = vec![];
        if let Some(arr) = root.get("manifest").and_then(|v| v.as_array()) {
            for item in arr {
                let t = item.as_table().ok_or("[[manifest]]: ожидалась таблица")?;
                let id = t.get("id").and_then(|v| v.as_str()).ok_or("[[manifest]] без id")?.to_string();
                manifest.push(TypeEntry { forms: lang_forms(t, &id)?, id });
            }
        }

        let mut phrases: HashMap<Lang, HashMap<Vec<String>, Vec<Meaning>>> = HashMap::new();
        let mut max_words = 1;
        for l in Lang::ALL {
            let m = phrases.entry(l).or_default();
            for e in &entries {
                for f in &e.forms[&l] {
                    let k = split_phrase(f);
                    if k.is_empty() {
                        continue;
                    }
                    max_words = max_words.max(k.len());
                    m.entry(k).or_default().push(Meaning::Op(e.id.clone()));
                }
                for (role, sf) in &e.support {
                    for f in &sf[&l] {
                        let k = split_phrase(f);
                        if k.is_empty() {
                            continue;
                        }
                        max_words = max_words.max(k.len());
                        m.entry(k).or_default().push(Meaning::Sup(e.id.clone(), role.clone()));
                    }
                }
            }
        }
        let d = Dictionary { schema_version, default_lang, entries, types, manifest, phrases, max_words };
        let errs: Vec<String> = d.validate().into_iter().filter(|p| !p.starts_with("предупреждение")).collect();
        if !errs.is_empty() {
            return Err(errs.join("; "));
        }
        Ok(d)
    }

    /// Проверка словаря: дубли словоформ, пустые операции, конфликты служебных слов.
    /// Строки, начинающиеся с «предупреждение», не считаются ошибками.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = vec![];
        let mut seen = std::collections::HashSet::new();
        for e in &self.entries {
            if !seen.insert(e.id.clone()) {
                problems.push(format!("id «{}» повторяется", e.id));
            }
            for l in Lang::ALL {
                if e.forms[&l].is_empty() {
                    let sev = if e.is_op { "" } else { "предупреждение: " };
                    problems.push(format!("{sev}у «{}» нет слов для языка {}", e.id, l.code()));
                }
            }
        }
        for l in Lang::ALL {
            for (phrase, meanings) in &self.phrases[&l] {
                let ops: Vec<&String> = meanings
                    .iter()
                    .filter_map(|m| if let Meaning::Op(id) = m { Some(id) } else { None })
                    .collect();
                if ops.len() > 1 {
                    problems.push(format!(
                        "[{}] словоформа «{}» принадлежит сразу {}",
                        l.code(),
                        phrase.join(" "),
                        ops.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                    ));
                }
                if !ops.is_empty() && ops.len() != meanings.len() {
                    problems.push(format!(
                        "[{}] «{}» одновременно ключевое и служебное слово",
                        l.code(),
                        phrase.join(" ")
                    ));
                }
            }
        }
        problems.sort();
        problems
    }

    /// Самая длинная фраза словаря в начале `words` (слова уже нормализованы).
    pub fn lookup(&self, lang: Lang, words: &[String]) -> Option<(usize, &Vec<Meaning>)> {
        let m = &self.phrases[&lang];
        let upto = words.len().min(self.max_words);
        for n in (1..=upto).rev() {
            if let Some(ms) = m.get(&words[..n]) {
                return Some((n, ms));
            }
        }
        None
    }

    pub fn max_words(&self) -> usize {
        self.max_words
    }

    fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Каноническая (первая) форма операции; для подсказок в сообщениях.
    pub fn form(&self, lang: Lang, id: &str) -> String {
        self.entry(id)
            .and_then(|e| e.forms[&lang].first().cloned())
            .unwrap_or_else(|| id.to_string())
    }

    /// Все формы операции на языке.
    pub fn forms(&self, lang: Lang, id: &str) -> Vec<String> {
        self.entry(id).map(|e| e.forms[&lang].clone()).unwrap_or_default()
    }

    /// Каноническое служебное слово роли, если есть.
    pub fn support(&self, lang: Lang, id: &str, role: &str) -> Option<String> {
        self.entry(id)?
            .support
            .iter()
            .find(|(r, _)| r == role)
            .and_then(|(_, f)| f[&lang].first().cloned())
    }

    /// Нейтральный id типа по слову из подсказки типа.
    pub fn type_id(&self, lang: Lang, word: &str) -> Option<String> {
        let w = norm(word);
        self.types
            .iter()
            .find(|t| t.forms[&lang].iter().any(|f| norm(f) == w) || t.id == w)
            .map(|t| t.id.clone())
    }

    /// Нейтральный id слова манифеста: форма любого языка или сам id.
    pub fn manifest_id(&self, word: &str) -> Option<String> {
        let w = norm(word);
        self.manifest
            .iter()
            .find(|t| t.id == w || Lang::ALL.iter().any(|l| t.forms[l].iter().any(|f| norm(f) == w)))
            .map(|t| t.id.clone())
    }

    /// Слово манифеста на языке проекта (первая форма; нет формы — id).
    pub fn manifest_word(&self, lang: Lang, id: &str) -> String {
        self.manifest
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| t.forms[&lang].first().cloned())
            .unwrap_or_else(|| id.to_string())
    }

    /// Все формы слова манифеста на всех языках (например, имена папки пакетов).
    pub fn manifest_forms(&self, id: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .manifest
            .iter()
            .filter(|t| t.id == id)
            .flat_map(|t| Lang::ALL.iter().flat_map(move |l| t.forms[l].clone()))
            .collect();
        v.dedup();
        v
    }

    /// Имена типов для подсказки в сообщении об ошибке.
    pub fn type_names(&self, lang: Lang) -> Vec<String> {
        self.types.iter().filter_map(|t| t.forms[&lang].first().cloned()).collect()
    }

    /// Все словоформы (для `vladx keywords` и проверок).
    pub fn all_forms(&self, lang: Lang) -> Vec<(String, String)> {
        let mut out = vec![];
        for e in &self.entries {
            for f in &e.forms[&lang] {
                out.push((e.id.clone(), f.clone()));
            }
            for (role, sf) in &e.support {
                for f in &sf[&lang] {
                    out.push((format!("{}.{}", e.id, role), f.clone()));
                }
            }
        }
        out
    }

    pub fn op_count(&self) -> usize {
        self.entries.iter().filter(|e| e.is_op).count()
    }
}
