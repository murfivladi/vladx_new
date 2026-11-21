//! `vladx.toml` — манифест проекта и пакета, как package.json в Node.
//!
//! Разделы и поля пишутся на языке проекта (слова — в `spec/keywords.toml`,
//! раздел `[[manifest]]`) или нейтральными id:
//!
//! ```toml
//! [проект]                    # [progetto] · [pruggettu] · [project]
//! имя = "@влад/строки"
//! версия = "1.2.0"
//! язык = "ru"
//! главный = "главная.vlstd"
//!
//! [зависимости]               # [dipendenze] · [dependencies]
//! @влад/утилиты = "^1.0.0"
//!
//! [скрипты]                   # [script] · [scripts]
//! тест = "vladx run тесты.vlstd"
//! ```
//!
//! Стандарт TOML 1.0 разрешает ключи без кавычек только латиницей; vladx.toml
//! читается свободнее: ключ из любых букв (и имена пакетов с `@` и `/`) можно
//! писать без кавычек — перед разбором такие ключи берутся в кавычки.

use std::collections::BTreeMap;
use vladx_syntax::{Dictionary, Lang};

pub const FILE: &str = "vladx.toml";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Manifest {
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub license: Option<String>,
    pub lang: Option<Lang>,
    pub main: Option<String>,
    pub registry: Option<String>,
    pub files: Option<bool>,
    pub net: Option<bool>,
    pub restrict_files: Option<bool>,
    pub dependencies: BTreeMap<String, String>,
    pub scripts: BTreeMap<String, String>,
}

fn bare_key_ok(k: &str) -> bool {
    !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Ключи из любых букв → в кавычках (стандартный TOML).
pub fn quote_keys(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 64);
    for line in src.lines() {
        let t = line.trim_start();
        let indent = &line[..line.len() - t.len()];
        if t.starts_with('[') && !t.starts_with("[[") {
            if let Some(end) = t.find(']') {
                let inner = t[1..end].trim();
                if !inner.starts_with('"') && !bare_key_ok(inner) {
                    out.push_str(&format!("{indent}[\"{}\"]{}\n", inner.replace('"', "\\\""), &t[end + 1..]));
                    continue;
                }
            }
        } else if !t.starts_with('#') && !t.starts_with('"') && !t.starts_with('\'') {
            if let Some(eq) = t.find('=') {
                let key = t[..eq].trim();
                if !key.is_empty() && !bare_key_ok(key) && !key.contains(' ') {
                    out.push_str(&format!("{indent}\"{}\" ={}\n", key.replace('"', "\\\""), &t[eq + 1..]));
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Разобрать vladx.toml (разделы и поля — на любом из языков или id).
pub fn parse(src: &str, dict: &Dictionary) -> Result<Manifest, String> {
    let table: toml::Table = quote_keys(src).parse().map_err(|e: toml::de::Error| format!("vladx.toml: {}", e.message()))?;
    let mut m = Manifest::default();
    for (sec_key, sec_val) in &table {
        let Some(sec) = dict.manifest_id(sec_key) else {
            return Err(format!("vladx.toml: неизвестный раздел [{sec_key}]"));
        };
        let Some(t) = sec_val.as_table() else {
            return Err(format!("vladx.toml: [{sec_key}] должен быть разделом"));
        };
        match sec.as_str() {
            "project" => {
                for (k, v) in t {
                    let id = dict.manifest_id(k).ok_or_else(|| format!("vladx.toml: неизвестное поле «{k}» в [{sec_key}]"))?;
                    let s = v.as_str().map(|s| s.to_string());
                    let need = |s: Option<String>| s.ok_or_else(|| format!("vladx.toml: «{k}» — строка в кавычках"));
                    match id.as_str() {
                        "name" => m.name = Some(need(s)?),
                        "version" => m.version = Some(need(s)?),
                        "description" => m.description = Some(need(s)?),
                        "author" => m.author = Some(need(s)?),
                        "license" => m.license = Some(need(s)?),
                        "main" => m.main = Some(need(s)?),
                        "registry" => m.registry = Some(need(s)?),
                        "lang" => {
                            let l = need(s)?;
                            m.lang = Some(Lang::from_code(&l).ok_or_else(|| format!("vladx.toml: неизвестный язык «{l}» (ru, it, scn)"))?);
                        }
                        _ => return Err(format!("vladx.toml: поле «{k}» не относится к [{sec_key}]")),
                    }
                }
            }
            "permissions" => {
                for (k, v) in t {
                    let id = dict.manifest_id(k).ok_or_else(|| format!("vladx.toml: неизвестное поле «{k}» в [{sec_key}]"))?;
                    let b = v.as_bool().ok_or_else(|| format!("vladx.toml: «{k}» — true или false"))?;
                    match id.as_str() {
                        "files" => m.files = Some(b),
                        "net" => m.net = Some(b),
                        "restrict_files" => m.restrict_files = Some(b),
                        _ => return Err(format!("vladx.toml: поле «{k}» не относится к [{sec_key}]")),
                    }
                }
            }
            "dependencies" => {
                for (k, v) in t {
                    let r = v.as_str().ok_or_else(|| format!("vladx.toml: версия «{k}» — строка, например \"^1.0.0\""))?;
                    crate::semver::Range::parse(r).map_err(|e| format!("vladx.toml: «{k}» = \"{r}\": {e}"))?;
                    m.dependencies.insert(k.clone(), r.to_string());
                }
            }
            "scripts" => {
                for (k, v) in t {
                    let c = v.as_str().ok_or_else(|| format!("vladx.toml: скрипт «{k}» — строка с командой"))?;
                    m.scripts.insert(k.clone(), c.to_string());
                }
            }
            _ => return Err(format!("vladx.toml: [{sec_key}] — не раздел")),
        }
    }
    Ok(m)
}

fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Ключ для записи: буквы любого языка — без кавычек (так vladx.toml и читается).
fn key(s: &str) -> String {
    if s.chars().any(|c| c.is_whitespace() || c == '"' || c == '=' || c == '#' || c == '[' || c == ']') {
        q(s)
    } else {
        s.to_string()
    }
}

/// Записать манифест на языке проекта.
pub fn render(m: &Manifest, lang: Lang, dict: &Dictionary) -> String {
    let w = |id: &str| dict.manifest_word(lang, id);
    let mut s = format!("[{}]\n", w("project"));
    let fields: [(&str, &Option<String>); 7] = [
        ("name", &m.name),
        ("version", &m.version),
        ("description", &m.description),
        ("author", &m.author),
        ("license", &m.license),
        ("main", &m.main),
        ("registry", &m.registry),
    ];
    for (id, v) in fields.iter().take(2) {
        if let Some(v) = v {
            s.push_str(&format!("{} = {}\n", w(id), q(v)));
        }
    }
    if let Some(l) = m.lang {
        s.push_str(&format!("{} = {}\n", w("lang"), q(l.code())));
    }
    for (id, v) in fields.iter().skip(2) {
        if let Some(v) = v {
            s.push_str(&format!("{} = {}\n", w(id), q(v)));
        }
    }
    if m.files.is_some() || m.net.is_some() || m.restrict_files.is_some() {
        s.push_str(&format!("\n[{}]\n", w("permissions")));
        for (id, v) in [("files", m.files), ("net", m.net), ("restrict_files", m.restrict_files)] {
            if let Some(b) = v {
                s.push_str(&format!("{} = {b}\n", w(id)));
            }
        }
    }
    s.push_str(&format!("\n[{}]\n", w("dependencies")));
    for (k, v) in &m.dependencies {
        s.push_str(&format!("{} = {}\n", key(k), q(v)));
    }
    if !m.scripts.is_empty() {
        s.push_str(&format!("\n[{}]\n", w("scripts")));
        for (k, v) in &m.scripts {
            s.push_str(&format!("{} = {}\n", key(k), q(v)));
        }
    }
    s
}

/// Папка установленных пакетов на языке проекта (`пакеты`, `pacchetti`).
pub fn modules_dir(dict: &Dictionary, lang: Lang) -> String {
    dict.manifest_word(lang, "modules_dir")
}

/// Все имена папки пакетов (на всех языках) — так рантайм находит пакеты.
pub fn modules_dir_names(dict: &Dictionary) -> Vec<String> {
    let mut v = dict.manifest_forms("modules_dir");
    v.dedup();
    v
}

/// Имя пакета: буквы любого языка, цифры, `-`, `_`, `.`; область — `@автор/имя`.
pub fn valid_name(name: &str) -> Result<(), String> {
    let seg_ok = |s: &str| {
        let mut cs = s.chars();
        cs.next().is_some_and(|c| c.is_alphanumeric())
            && s.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
            && s.chars().count() <= 60
    };
    let ok = match name.strip_prefix('@') {
        Some(rest) => matches!(rest.split_once('/'), Some((scope, n)) if seg_ok(scope) && seg_ok(n)),
        None => seg_ok(name),
    };
    if !ok {
        return Err(format!(
            "имя пакета «{name}»: буквы, цифры, «-», «_», «.»; область автора — @автор/имя"
        ));
    }
    if name.chars().any(|c| c.is_uppercase()) {
        return Err(format!("имя пакета «{name}» пишется строчными буквами"));
    }
    Ok(())
}

fn unquote_key(k: &str) -> String {
    let k = k.trim();
    if k.len() >= 2 && ((k.starts_with('"') && k.ends_with('"')) || (k.starts_with('\'') && k.ends_with('\''))) {
        k[1..k.len() - 1].replace("\\\"", "\"")
    } else {
        k.to_string()
    }
}

/// Заголовок раздела в строке, если это `[раздел]`.
fn header(line: &str) -> Option<String> {
    let t = line.trim();
    if t.starts_with('[') && !t.starts_with("[[") {
        let end = t.find(']')?;
        return Some(unquote_key(&t[1..end]));
    }
    None
}

/// Поставить (`Some`) или убрать (`None`) зависимость, не трогая остальной текст
/// и комментарии. Нет раздела зависимостей — он добавляется на языке проекта.
pub fn set_dependency(src: &str, dict: &Dictionary, lang: Lang, name: &str, range: Option<&str>) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 3);
    let mut in_deps = false;
    let mut found_section = false;
    let mut done = false;
    let mut last_entry: Option<usize> = None; // индекс в out последней строки раздела
    let entry = range.map(|r| format!("{} = {}", key(name), q(r)));
    for line in &lines {
        if let Some(h) = header(line) {
            if in_deps && !done {
                if let (Some(e), Some(i)) = (&entry, last_entry) {
                    out.insert(i + 1, e.clone());
                    done = true;
                }
            }
            in_deps = dict.manifest_id(&h).as_deref() == Some("dependencies");
            if in_deps {
                found_section = true;
                out.push(line.to_string());
                last_entry = Some(out.len() - 1);
                continue;
            }
        } else if in_deps {
            let t = line.trim();
            if let Some(eq) = t.find('=').filter(|_| !t.starts_with('#')) {
                if unquote_key(&t[..eq]) == name {
                    if let Some(e) = &entry {
                        if !done {
                            out.push(e.clone());
                            last_entry = Some(out.len() - 1);
                            done = true;
                        }
                    } else {
                        done = true;
                    }
                    continue;
                }
                out.push(line.to_string());
                last_entry = Some(out.len() - 1);
                continue;
            }
        }
        out.push(line.to_string());
    }
    if !done {
        if let Some(e) = &entry {
            if in_deps {
                let i = last_entry.unwrap_or(out.len() - 1);
                out.insert(i + 1, e.clone());
            } else if !found_section {
                if out.last().is_some_and(|l| !l.trim().is_empty()) {
                    out.push(String::new());
                }
                out.push(format!("[{}]", dict.manifest_word(lang, "dependencies")));
                out.push(e.clone());
            }
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}
