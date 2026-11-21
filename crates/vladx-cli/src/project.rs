//! Проект VladX: `vladx.toml` (как package.json в Node) — язык, точка входа, права.
//!
//! ```toml
//! [project]
//! name = "мой-проект"
//! lang = "ru"             # ru | it | scn — тогда --lang не нужен
//! main = "главная.vlstd"  # vladx run без имени файла
//!
//! [permissions]           # необязательно; по умолчанию открыто всё
//! files = true
//! net = true
//! restrict_files = false
//! ```
//!
//! Корень проекта — папка с `vladx.toml`: от неё считаются пути `file.*`, а модули
//! (`из "путь" возьми …`) — от папки импортирующего файла.

use std::path::{Path, PathBuf};
use vladx_syntax::{Dictionary, Lang};

pub const FILE: &str = "vladx.toml";

#[derive(Debug, Default)]
pub struct Project {
    pub root: PathBuf,
    pub lang: Option<Lang>,
    pub main: Option<String>,
    pub files: Option<bool>,
    pub net: Option<bool>,
    pub restrict_files: Option<bool>,
}

pub fn load(path: &Path) -> Result<Project, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{} не прочитан: {e}", path.display()))?;
    let t: toml::Table = src.parse().map_err(|e| format!("{}: {e}", path.display()))?;
    let p = t.get("project").and_then(|v| v.as_table());
    let perm = t.get("permissions").and_then(|v| v.as_table());
    let s = |tb: Option<&toml::Table>, k: &str| tb.and_then(|x| x.get(k)).and_then(|v| v.as_str()).map(|s| s.to_string());
    let b = |k: &str| perm.and_then(|x| x.get(k)).and_then(|v| v.as_bool());
    let lang = match s(p, "lang") {
        Some(l) => Some(Lang::from_code(&l).ok_or_else(|| format!("{}: неизвестный язык «{l}» (ru, it, scn)", path.display()))?),
        None => None,
    };
    Ok(Project {
        root: path.parent().map(|d| d.to_path_buf()).unwrap_or_default(),
        lang,
        main: s(p, "main"),
        files: b("files"),
        net: b("net"),
        restrict_files: b("restrict_files"),
    })
}

/// Ближайший `vladx.toml` вверх от папки.
pub fn find(start: &Path) -> Option<Result<Project, String>> {
    let start = std::fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    let mut dir = Some(start.as_path());
    while let Some(d) = dir {
        let f = d.join(FILE);
        if f.is_file() {
            return Some(load(&f));
        }
        dir = d.parent();
    }
    None
}

// ───────────── vladx init ─────────────

const MODULE_RU: &str = "// Модуль проекта: всё, что объявлено здесь, можно взять через «из … возьми».
// Имена с «_» в начале — внутренние.

функция приветствие(имя) {
  верни \"Привет, \" + имя + \"!\"
}

пусть _секрет будет \"не видно снаружи\"
";

const MAIN_RU: &str = "// Точка входа проекта: vladx run (без имени файла) запускает её.
из \"утилиты\" возьми приветствие

спроси \"Как тебя зовут?\" и сохрани в имя
скажи приветствие(имя)
";

const MODULE_IT: &str = "// Modulo del progetto: tutto ciò che è dichiarato qui si prende con «da … prendi».
// I nomi che iniziano con «_» restano interni.

funzione saluto(nome) {
  rendi \"Ciao, \" + nome + \"!\"
}

sia _segreto = \"invisibile da fuori\"
";

const MAIN_IT: &str = "// Punto di ingresso: vladx run (senza nome di file) lo esegue.
da \"utili\" prendi saluto

chiedi \"Come ti chiami?\" e salva in nome
dì saluto(nome)
";

/// Создать проект: vladx.toml, точка входа и пример модуля на языке проекта.
pub fn init(dir: &Path, lang: Lang, dict: &Dictionary) -> Result<Vec<PathBuf>, String> {
    if dir.join(FILE).exists() {
        return Err(format!("{} уже есть в {}", FILE, dir.display()));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("папка {} не создана: {e}", dir.display()))?;
    let name = std::fs::canonicalize(dir)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "vladx-проект".into());
    // Исходники пишутся на ru/it, для scn — переводятся форматтером из it.
    let (main_name, module_name, main_src, module_src) = match lang {
        Lang::Ru => ("главная.vlstd", "утилиты.vlstd", MAIN_RU.to_string(), MODULE_RU.to_string()),
        Lang::It => ("principale.vlstd", "utili.vlstd", MAIN_IT.to_string(), MODULE_IT.to_string()),
        Lang::Scn => (
            "principali.vlstd",
            "utili.vlstd",
            vladx_syntax::format_source(MAIN_IT, Lang::It, Lang::Scn, dict).map_err(|d| d.message)?,
            vladx_syntax::format_source(MODULE_IT, Lang::It, Lang::Scn, dict).map_err(|d| d.message)?,
        ),
    };
    let toml = format!(
        "[project]\nname = \"{name}\"\nlang = \"{}\"\nmain = \"{main_name}\"\n\n# По умолчанию программе открыто всё, как в Node. Сузить:\n# [permissions]\n# files = true\n# net = true\n# restrict_files = false\n",
        lang.code()
    );
    let mut made = vec![];
    for (f, text) in [(FILE, toml), (main_name, main_src), (module_name, module_src)] {
        let p = dir.join(f);
        std::fs::write(&p, text).map_err(|e| format!("{} не записан: {e}", p.display()))?;
        made.push(p);
    }
    Ok(made)
}
