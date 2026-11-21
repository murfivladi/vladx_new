//! Интерпретатор VladX.
//!
//! Внешний мир доступен только через [`Host`]: файлы, сеть, ввод, время.
//! Одно и то же ядро работает в CLI (диск + HTTPS) и в браузере (WASM:
//! хранилище страницы + HTTP). По умолчанию открыто всё, как в Node.

pub mod interp;
pub mod stdlib;
pub mod value;

pub use interp::{Interp, Words};
pub use stdlib::GLOBAL_NAMES;
pub use value::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use vladx_ast::{Diagnostic, Program};
use vladx_syntax::{Dictionary, Lang};

/// Вид доступа к внешнему миру.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Perm {
    Files,
    Net,
}

/// Канал во внешний мир. Пять основных операций + служебные.
/// Ошибки — строки на языке проекта; ядро превращает их в E-FILE-*/E-NET-*.
pub trait Host {
    fn read(&mut self, path: &str) -> Result<String, String>;
    fn write(&mut self, path: &str, data: &str) -> Result<(), String>;
    fn delete(&mut self, path: &str) -> Result<(), String>;
    fn list(&mut self) -> Result<Vec<String>, String>;
    fn get(&mut self, url: &str) -> Result<String, String>;

    /// Доступ сужен, а программа его просит: спросить человека.
    /// `true` — разрешил, программа продолжается.
    fn ask_permission(&mut self, _kind: Perm) -> bool {
        false
    }
    /// Ответ на «спроси».
    fn input(&mut self, _prompt: &str) -> Option<String> {
        None
    }
    /// Строка вывода (для потокового вывода в CLI).
    fn say(&mut self, _line: &str) {}
    /// Предупреждение W-*.
    fn warn(&mut self, _line: &str) {}
    /// Текущее время хоста в мс (местное). В WASM своих часов нет.
    fn now_ms(&mut self) -> f64 {
        0.0
    }
    /// Исходник модуля проекта для `из "путь" возьми …` (путь — от корня проекта).
    /// Это код, а не данные программы: разрешение «файлы» не требуется, как у require в Node.
    fn read_code(&mut self, path: &str) -> Result<String, String> {
        self.read(path)
    }
    /// Мягкая остановка (лимит времени). `Some(сообщение)` — остановить.
    fn should_stop(&mut self) -> Option<String> {
        None
    }
}

/// Настройки исполнения.
#[derive(Debug, Clone)]
pub struct Options {
    pub max_steps: u64,
    pub max_depth: u32,
    /// Разрешение «файлы» (по умолчанию — да).
    pub files: bool,
    /// Разрешение «сеть» (по умолчанию — да).
    pub net: bool,
    /// Зерно случайностей (для воспроизводимых тестов).
    pub seed: Option<u64>,
    /// Интерактивный ввод (площадка): «спроси» без готового ответа останавливает
    /// программу кодом [`ASK_WAIT`]; страница спрашивает человека и запускает
    /// программу заново со всеми ответами (исполнение детерминировано).
    pub interactive: bool,
    /// Папка главной программы относительно корня проекта ("" — корень):
    /// от неё считаются пути `из "…" возьми`.
    pub main_dir: String,
}

/// Код остановки «программа ждёт ответ на спроси» (не ошибка, не ловится «попробуй»).
pub const ASK_WAIT: &str = "E-ASK-WAIT";

impl Default for Options {
    fn default() -> Self {
        Options { max_steps: 10_000_000, max_depth: 1000, files: true, net: true, seed: None, interactive: false, main_dir: String::new() }
    }
}

/// Итог запуска (тот же формат уходит в JSON `vx_run`).
#[derive(Debug, Clone, Default)]
pub struct RunResult {
    pub ok: bool,
    pub stdout: Vec<String>,
    pub warnings: Vec<Diagnostic>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn words_for(dict: &Dictionary, lang: Lang) -> Words {
    Words {
        true_: dict.form(lang, "lit.true"),
        false_: dict.form(lang, "lit.false"),
        null: dict.form(lang, "lit.null"),
        let_: dict.form(lang, "var.declare"),
        mutable: dict.form(lang, "var.mutable"),
        assign: dict.form(lang, "var.assign"),
    }
}

/// Полный цикл: разбор → статические проверки → исполнение.
pub fn run_source(src: &str, lang: Lang, dict: &Dictionary, host: &mut dyn Host, opts: Options) -> RunResult {
    let (prog, diags) = vladx_syntax::analyze(src, lang, dict, GLOBAL_NAMES);
    let (errors, warnings): (Vec<_>, Vec<_>) = diags.into_iter().partition(|d| d.is_error());
    match prog {
        Some(p) if errors.is_empty() => {
            let mut r = run_program(&p, dict, host, opts);
            let mut w = warnings;
            w.extend(r.warnings);
            r.warnings = w;
            r
        }
        _ => RunResult { ok: false, stdout: vec![], warnings, diagnostics: errors },
    }
}

/// Исполнить готовый AST (например, полученный из `.vlhmn`).
pub fn run_program(p: &Program, dict: &Dictionary, host: &mut dyn Host, opts: Options) -> RunResult {
    let lang = Lang::from_code(&p.lang).unwrap_or(dict.default_lang);
    let mut it = Interp::new(host, opts, words_for(dict, lang));
    it.set_project(dict.clone(), lang);
    let r = it.run(&p.body);
    it.cleanup();
    let stdout = std::mem::take(&mut it.stdout);
    let warnings = std::mem::take(&mut it.warnings);
    match r {
        Ok(()) => RunResult { ok: true, stdout, warnings, diagnostics: vec![] },
        Err(d) => RunResult { ok: false, stdout, warnings, diagnostics: vec![d] },
    }
}

/// Сверка stdlib со словарём (см. [`Interp::stdlib_problems`]). Пусто — всё сходится.
pub fn stdlib_problems(dict: &Dictionary) -> Vec<String> {
    let mut host = MemHost::default();
    let mut it = Interp::new(&mut host, Options::default(), words_for(dict, Lang::Ru));
    it.set_project(dict.clone(), Lang::Ru);
    let p = it.stdlib_problems();
    it.cleanup();
    p
}

/// Хост в памяти: для тестов и встраивания. Файлы — словарь, сеть — заготовки.
#[derive(Debug, Default)]
pub struct MemHost {
    pub files: BTreeMap<String, String>,
    /// url → ответ (Ok — тело, Err — ошибка хоста).
    pub net: HashMap<String, Result<String, String>>,
    pub inputs: VecDeque<String>,
    /// Ответ человека на просьбу о доступе.
    pub grant_on_ask: bool,
    /// Сколько раз спрашивали разрешение.
    pub asks: Vec<Perm>,
    pub time_ms: f64,
    /// Ошибка, которую хост вернёт на запись (для проверки доставки ошибок).
    pub fail_writes: Option<String>,
}

impl Host for MemHost {
    fn read(&mut self, path: &str) -> Result<String, String> {
        self.files.get(path).cloned().ok_or_else(|| format!("файл «{path}» не найден"))
    }
    fn write(&mut self, path: &str, data: &str) -> Result<(), String> {
        if let Some(e) = &self.fail_writes {
            return Err(e.clone());
        }
        self.files.insert(path.to_string(), data.to_string());
        Ok(())
    }
    fn delete(&mut self, path: &str) -> Result<(), String> {
        self.files.remove(path).map(|_| ()).ok_or_else(|| format!("файл «{path}» не найден"))
    }
    fn list(&mut self) -> Result<Vec<String>, String> {
        Ok(self.files.keys().cloned().collect())
    }
    fn get(&mut self, url: &str) -> Result<String, String> {
        self.net.get(url).cloned().unwrap_or_else(|| Err(format!("запрос не удался: нет ответа для {url}")))
    }
    fn ask_permission(&mut self, kind: Perm) -> bool {
        self.asks.push(kind);
        self.grant_on_ask
    }
    fn input(&mut self, _prompt: &str) -> Option<String> {
        self.inputs.pop_front()
    }
    fn now_ms(&mut self) -> f64 {
        self.time_ms
    }
}
