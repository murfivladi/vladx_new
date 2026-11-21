//! Стандартная библиотека — как в Node: модули написаны на самом VladX
//! (`crates/vladx-runtime/stdlib/*.vlstd`, вшиты в бинарь, как `lib/*.js` в Node),
//! а на Rust — только `ядро` (доступ к платформе и горячие операции, см. `core.rs`).
//!
//! Подключение — явное: `импортируй числа` или `из "vladx:числа" возьми корень`.
//! Имена модулей и функций — из словаря (`[[stdlib]]`): на любом языке, все формы — синонимы.
//! Глобальны только `длина`, `размер`, `строка`, `число` (как `console` в Node).

pub mod core;

use crate::value::{Env, Value};
use std::rc::Rc;

pub use self::core::{from_json, to_json};

/// Исходники модулей stdlib: нейтральный id → текст на ru.
pub const SOURCES: &[(&str, &str)] = &[
    ("math", include_str!("../../stdlib/math.vlstd")),
    ("text", include_str!("../../stdlib/text.vlstd")),
    ("list", include_str!("../../stdlib/list.vlstd")),
    ("map", include_str!("../../stdlib/map.vlstd")),
    ("random", include_str!("../../stdlib/random.vlstd")),
    ("time", include_str!("../../stdlib/time.vlstd")),
    ("json", include_str!("../../stdlib/json.vlstd")),
    ("fs", include_str!("../../stdlib/fs.vlstd")),
    ("net", include_str!("../../stdlib/net.vlstd")),
    ("path", include_str!("../../stdlib/path.vlstd")),
    ("events", include_str!("../../stdlib/events.vlstd")),
    ("assert", include_str!("../../stdlib/assert.vlstd")),
];

pub fn source(id: &str) -> Option<&'static str> {
    SOURCES.iter().find(|(i, _)| *i == id).map(|(_, s)| *s)
}

/// Глобальные функции (есть везде без подключения).
pub const GLOBAL_FNS: &[&str] = &["длина", "размер", "строка", "число"];

/// Прежние глобальные модули: работают без подключения, но с предупреждением
/// W-DEPRECATED-001 — пора писать `импортируй …`.
pub const DEPRECATED_GLOBALS: &[&str] = &["numeri", "testo", "liste", "mappe", "caso", "tempo", "json", "file", "rete"];

/// Имена, которые среда даёт программе (для статических проверок).
pub const GLOBAL_NAMES: &[&str] = &[
    "numeri", "testo", "liste", "mappe", "caso", "tempo", "json", "file", "rete", "длина", "размер", "строка", "число",
];

/// То же для исходников stdlib: им доступно ещё и `ядро`.
pub const STDLIB_GLOBALS: &[&str] = &["длина", "размер", "строка", "число", "ядро"];

/// Глобальные функции в окружение программы или модуля.
pub fn install(g: &Rc<Env>) {
    g.define("длина", self::core::native("длина", 1, 1, self::core::g_len), false);
    g.define("размер", self::core::native("размер", 1, 1, self::core::g_len), false);
    g.define("строка", self::core::native("строка", 1, 1, self::core::g_str), false);
    g.define("число", self::core::native("число", 1, 1, self::core::g_num), false);
}

/// Модуль `vladx:…` в виде значения: имена функций на всех языках.
pub fn module_value(name: &str, shown: Vec<String>, exports: &Rc<Env>) -> Value {
    let mut v: Vec<(String, Value)> = exports.vars.borrow().iter().map(|(k, b)| (k.clone(), b.value.clone())).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    let fns = v.into_iter().collect();
    Value::Module(Rc::new(crate::value::Module { name: name.to_string(), names: shown, fns }))
}
