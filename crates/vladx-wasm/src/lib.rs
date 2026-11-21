//! VladX в WebAssembly. Только C-ABI: файл один и грузится «как есть».
//!
//! Экспорты: `vx_alloc`, `vx_free`, `vx_run`, `vx_parse`, `vx_result_ptr`,
//! `vx_result_len`, `vx_set_input`, `vx_set_time`, `vx_set_permissions`,
//! `vx_set_keywords`, `vx_version`, `vx_reset_host`, `vx_run_ast`, `vx_format`,
//! `vx_set_llm`, `vx_translate`, `vx_set_interactive`, `vx_tokens`, `vx_check`.
//!
//! Импорты (`env.*`): `vx_host_read`, `vx_host_write`, `vx_host_delete`,
//! `vx_host_list`, `vx_host_get` — возвращают `u64 = (ptr << 32) | len` на буфер
//! `"ok\n…"` или `"err\n…"`, выделенный через `vx_alloc`; ядро освобождает его.
//! `vx_host_ask(kind)` — просьба о доступе (1 — файлы, 2 — сеть), 1 = разрешено.
//! `vx_host_say(строка)` — вывод по ходу выполнения; `vx_host_input(вопрос) → u64` — ответ человека
//! (`"ok\n<ответ>"`; только в режиме 2); `vx_host_now() → f64` — время в мс (0 — взять vx_set_time).
//! `vx_host_llm(json)` — диалог перевода `.vlhmn` (`{"messages": [...]}`) серверу ИИ
//! (`vladx-llm.php` или `vladx serve` → /llm); ответ — `"ok\n<текст модели>"` или `"err\n…"`.
//! `vx_host_http(адрес)` — GET к реестру пакетов напрямую (не сеть программы): `"ok\n<тело>"`.
//! `vx_pm(json)` — менеджер пакетов в хранилище страницы (как `vladpm install/add/remove/update`).

use std::cell::RefCell;
use std::collections::VecDeque;
use vladx_runtime::{Host, Options, Perm};
use vladx_syntax::{Dictionary, Lang};

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
extern "C" {
    fn vx_host_read(p: *const u8, l: usize) -> u64;
    fn vx_host_write(pp: *const u8, pl: usize, dp: *const u8, dl: usize) -> u64;
    fn vx_host_delete(p: *const u8, l: usize) -> u64;
    fn vx_host_list() -> u64;
    fn vx_host_get(p: *const u8, l: usize) -> u64;
    fn vx_host_ask(kind: u32) -> u32;
    fn vx_host_llm(p: *const u8, l: usize) -> u64;
    fn vx_host_say(p: *const u8, l: usize);
    fn vx_host_input(p: *const u8, l: usize) -> u64;
    fn vx_host_now() -> f64;
    fn vx_host_http(p: *const u8, l: usize) -> u64;
}

// Вне WASM (cargo test/build на хосте) — заглушки: хоста нет.
#[cfg(not(target_arch = "wasm32"))]
mod stubs {
    pub unsafe fn vx_host_read(_: *const u8, _: usize) -> u64 { 0 }
    pub unsafe fn vx_host_write(_: *const u8, _: usize, _: *const u8, _: usize) -> u64 { 0 }
    pub unsafe fn vx_host_delete(_: *const u8, _: usize) -> u64 { 0 }
    pub unsafe fn vx_host_list() -> u64 { 0 }
    pub unsafe fn vx_host_get(_: *const u8, _: usize) -> u64 { 0 }
    pub unsafe fn vx_host_ask(_: u32) -> u32 { 0 }
    pub unsafe fn vx_host_llm(_: *const u8, _: usize) -> u64 { 0 }
    pub unsafe fn vx_host_say(_: *const u8, _: usize) {}
    pub unsafe fn vx_host_input(_: *const u8, _: usize) -> u64 { 0 }
    pub unsafe fn vx_host_now() -> f64 { 0.0 }
    pub unsafe fn vx_host_http(_: *const u8, _: usize) -> u64 { 0 }
}
#[cfg(not(target_arch = "wasm32"))]
use stubs::*;

struct State {
    dict: Dictionary,
    result: Vec<u8>,
    input: VecDeque<String>,
    time_ms: f64,
    perms: u32,
    /// Модель на сервере ИИ (входит в ключ кэша переводов).
    llm_model: String,
    /// Площадка: «спроси» без ответа останавливает программу (см. Options::interactive).
    /// «спроси»: 0 — ответы только из очереди; 1 — без ответа остановиться
    /// (повторный запуск с ответами); 2 — ждать ответа через импорт vx_host_input
    /// (страница с SharedArrayBuffer: программа реально стоит, как в Node).
    ask_mode: u32,
    /// Папка главной программы в хранилище: от неё считаются пути импорта.
    main_dir: String,
    /// Кэш переводов .vlhmn на время жизни страницы.
    hmn_cache: std::collections::HashMap<String, String>,
}

thread_local! {
    static ST: RefCell<State> = RefCell::new(State {
        dict: Dictionary::builtin(),
        result: Vec::new(),
        input: VecDeque::new(),
        time_ms: 0.0,
        perms: 3,
        llm_model: String::new(),
        ask_mode: 0,
        main_dir: String::new(),
        hmn_cache: std::collections::HashMap::new(),
    });
}

fn set_result(s: String) {
    ST.with(|st| st.borrow_mut().result = s.into_bytes());
}

unsafe fn read_str(ptr: *const u8, len: usize) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    String::from_utf8_lossy(std::slice::from_raw_parts(ptr, len)).into_owned()
}

/// Ответ хоста: `(ptr << 32) | len` → `Ok(тело)` / `Err(сообщение)`.
fn take_reply(packed: u64) -> Result<String, String> {
    let ptr = (packed >> 32) as usize as *mut u8;
    let len = (packed & 0xffff_ffff) as usize;
    if ptr.is_null() {
        return Err("хост не ответил (страница не подключила файлы и сеть)".into());
    }
    let s = unsafe {
        let s = read_str(ptr, len);
        vx_free(ptr, len);
        s
    };
    if let Some(rest) = s.strip_prefix("ok\n") {
        Ok(rest.to_string())
    } else if s == "ok" {
        Ok(String::new())
    } else {
        Err(s.strip_prefix("err\n").unwrap_or(&s).to_string())
    }
}

struct WebHost;

impl Host for WebHost {
    fn read(&mut self, path: &str) -> Result<String, String> {
        take_reply(unsafe { vx_host_read(path.as_ptr(), path.len()) })
    }
    fn write(&mut self, path: &str, data: &str) -> Result<(), String> {
        take_reply(unsafe { vx_host_write(path.as_ptr(), path.len(), data.as_ptr(), data.len()) }).map(|_| ())
    }
    fn delete(&mut self, path: &str) -> Result<(), String> {
        take_reply(unsafe { vx_host_delete(path.as_ptr(), path.len()) }).map(|_| ())
    }
    fn list(&mut self) -> Result<Vec<String>, String> {
        let s = take_reply(unsafe { vx_host_list() })?;
        Ok(s.lines().filter(|l| !l.is_empty()).map(|l| l.to_string()).collect())
    }
    fn get(&mut self, url: &str) -> Result<String, String> {
        take_reply(unsafe { vx_host_get(url.as_ptr(), url.len()) })
    }
    fn ask_permission(&mut self, kind: Perm) -> bool {
        let k = match kind {
            Perm::Files => 1,
            Perm::Net => 2,
        };
        unsafe { vx_host_ask(k) == 1 }
    }
    fn input(&mut self, prompt: &str) -> Option<String> {
        if let Some(a) = ST.with(|st| st.borrow_mut().input.pop_front()) {
            return Some(a);
        }
        if ST.with(|st| st.borrow().ask_mode) == 2 {
            return take_reply(unsafe { vx_host_input(prompt.as_ptr(), prompt.len()) }).ok();
        }
        None
    }
    fn say(&mut self, line: &str) {
        // Вывод по ходу выполнения, как console.log.
        unsafe { vx_host_say(line.as_ptr(), line.len()) }
    }
    fn now_ms(&mut self) -> f64 {
        // Время даёт страница: живое (как Date.now) или зафиксированное на сеанс повторов.
        let t = unsafe { vx_host_now() };
        if t > 0.0 { t } else { ST.with(|st| st.borrow().time_ms) }
    }
}

// ───────────── экспорты ─────────────

#[no_mangle]
pub extern "C" fn vx_alloc(len: usize) -> *mut u8 {
    let mut v: Vec<u8> = Vec::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr` получен из `vx_alloc(len)` и ещё не освобождён.
#[no_mangle]
pub unsafe extern "C" fn vx_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
    }
}

#[no_mangle]
pub extern "C" fn vx_result_ptr() -> *const u8 {
    ST.with(|st| st.borrow().result.as_ptr())
}

#[no_mangle]
pub extern "C" fn vx_result_len() -> usize {
    ST.with(|st| st.borrow().result.len())
}

#[no_mangle]
pub extern "C" fn vx_version() -> u32 {
    vladx_ast::SCHEMA_VERSION
}

/// Ответы для «спроси»: по одному на строку.
///
/// # Safety
/// `ptr..ptr+len` — валидный буфер UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vx_set_input(ptr: *const u8, len: usize) {
    let s = read_str(ptr, len);
    ST.with(|st| st.borrow_mut().input = s.lines().map(|l| l.to_string()).collect());
}

/// Текущее время хоста (местное, мс): в WASM своих часов нет.
#[no_mangle]
pub extern "C" fn vx_set_time(ms: f64) {
    ST.with(|st| st.borrow_mut().time_ms = ms);
}

/// Бит 0 — файлы, бит 1 — сеть. По умолчанию 3 (всё открыто).
#[no_mangle]
pub extern "C" fn vx_set_permissions(bits: u32) {
    ST.with(|st| st.borrow_mut().perms = bits);
}

/// Сбросить ввод перед запуском.
#[no_mangle]
pub extern "C" fn vx_reset_host() {
    ST.with(|st| st.borrow_mut().input.clear());
}

/// Подменить словарь на лету (TOML). 1 — принят, 0 — ошибка (текст в результате).
///
/// # Safety
/// `ptr..ptr+len` — валидный буфер UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vx_set_keywords(ptr: *const u8, len: usize) -> u32 {
    let s = read_str(ptr, len);
    match Dictionary::from_toml(&s) {
        Ok(d) => {
            ST.with(|st| st.borrow_mut().dict = d);
            set_result("{\"ok\":true}".into());
            1
        }
        Err(e) => {
            set_result(serde_json::json!({"ok": false, "diagnostics": [{"code": "E-DICT-001", "message": e, "line": 0, "col": 0, "len": 0}]}).to_string());
            0
        }
    }
}

fn lang_of(i: u32) -> Lang {
    Lang::from_index(i).unwrap_or(Lang::Ru)
}

fn diag_json(d: &vladx_ast::Diagnostic) -> serde_json::Value {
    let mut v = serde_json::json!({"code": d.code, "message": d.message, "line": d.line, "col": d.col, "len": d.len});
    if let Some(h) = &d.hint {
        v["hint"] = h.clone().into();
    }
    if let Some(f) = &d.file {
        v["file"] = f.clone().into();
    }
    v
}

/// Полный цикл: разбор + статика + исполнение. Результат — JSON
/// `{ok, stdout, warnings, diagnostics}` в буфере результата. Возврат: 1/0.
///
/// # Safety
/// `src_ptr..src_ptr+src_len` — валидный буфер UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vx_run(src_ptr: *const u8, src_len: usize, lang: u32, max_steps: u32) -> u32 {
    let src = read_str(src_ptr, src_len);
    let (dict, opts) = run_setup(max_steps);
    let r = vladx_runtime::run_source(&src, lang_of(lang), &dict, &mut WebHost, opts);
    set_result(result_json(&r, None));
    r.ok as u32
}

/// Только разбор: при успехе в результате AST (JSON), иначе `{ok:false, diagnostics}`.
///
/// # Safety
/// `src_ptr..src_ptr+src_len` — валидный буфер UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vx_parse(src_ptr: *const u8, src_len: usize, lang: u32) -> u32 {
    let src = read_str(src_ptr, src_len);
    let dict = ST.with(|st| st.borrow().dict.clone());
    match vladx_syntax::parse(&src, lang_of(lang), &dict) {
        Ok(p) => {
            set_result(p.to_json_pretty());
            1
        }
        Err(d) => {
            set_result(serde_json::json!({"ok": false, "diagnostics": [diag_json(&d)]}).to_string());
            0
        }
    }
}

fn run_setup(max_steps: u32) -> (Dictionary, Options) {
    ST.with(|st| {
        let st = st.borrow();
        let opts = Options {
            max_steps: if max_steps == 0 { 3_000_000 } else { max_steps as u64 },
            files: st.perms & 1 != 0,
            net: st.perms & 2 != 0,
            interactive: st.ask_mode == 1,
            main_dir: st.main_dir.clone(),
            seed: Some(st.time_ms.to_bits() | 1),
            ..Options::default()
        };
        (st.dict.clone(), opts)
    })
}

fn result_json(r: &vladx_runtime::RunResult, extra: Option<(&str, &str)>) -> String {
    // «спроси» ждёт ответа — это не ошибка, а пауза: {"waiting": {"prompt": …}}.
    let waiting = r.diagnostics.first().filter(|d| d.code == vladx_runtime::ASK_WAIT);
    let diags: Vec<serde_json::Value> = if waiting.is_some() { vec![] } else { r.diagnostics.iter().map(diag_json).collect() };
    let mut json = serde_json::json!({
        "ok": r.ok,
        "stdout": r.stdout,
        "warnings": r.warnings.iter().map(|w| format!("{}:{}  {}  {}", w.line, w.col, w.code, w.message)).collect::<Vec<_>>(),
        "diagnostics": diags,
    });
    if let Some(w) = waiting {
        json["waiting"] = serde_json::json!({ "prompt": w.message });
    }
    if let Some((k, v)) = extra {
        json[k] = v.into();
    }
    json.to_string()
}

/// Исполнить готовый AST (JSON, например перевод `.vlhmn` от сервера).
/// Ядро не доверяет входу: схема → печать → разбор → сравнение → статика,
/// и только потом исполнение. В результате есть `vlstd` — текст программы,
/// к строкам которого относятся позиции ошибок.
///
/// # Safety
/// `ptr..ptr+len` — валидный буфер UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vx_run_ast(ptr: *const u8, len: usize, max_steps: u32) -> u32 {
    let json = read_str(ptr, len);
    let (dict, opts) = run_setup(max_steps);
    let checked = match vladx_syntax::validate::validate_json(&json, &dict, vladx_runtime::GLOBAL_NAMES) {
        Ok(c) => c,
        Err(ds) => {
            let r = vladx_runtime::RunResult { ok: false, diagnostics: ds, ..Default::default() };
            set_result(result_json(&r, None));
            return 0;
        }
    };
    let r = vladx_runtime::run_program(&checked.program, &dict, &mut WebHost, opts);
    set_result(result_json(&r, Some(("vlstd", &checked.text))));
    r.ok as u32
}

/// Форматирование и перевод кода: язык `from` → язык `to` (0 ru, 1 it, 2 scn).
/// Результат — текст или `{ok:false, diagnostics}`.
///
/// # Safety
/// `ptr..ptr+len` — валидный буфер UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vx_format(ptr: *const u8, len: usize, from: u32, to: u32) -> u32 {
    let src = read_str(ptr, len);
    let dict = ST.with(|st| st.borrow().dict.clone());
    match vladx_syntax::format_source(&src, lang_of(from), lang_of(to), &dict) {
        Ok(text) => {
            set_result(text);
            1
        }
        Err(d) => {
            set_result(serde_json::json!({"ok": false, "diagnostics": [diag_json(&d)]}).to_string());
            0
        }
    }
}

// ───────────── .vlhmn в браузере ─────────────

struct WebLlm;

impl vladx_hmn::Llm for WebLlm {
    fn model(&self) -> String {
        ST.with(|st| st.borrow().llm_model.clone())
    }
    fn chat(&mut self, messages: &[vladx_hmn::Msg]) -> Result<String, String> {
        let msgs: Vec<serde_json::Value> =
            messages.iter().map(|m| serde_json::json!({"role": m.role, "content": m.content})).collect();
        let body = serde_json::json!({ "messages": msgs }).to_string();
        take_reply(unsafe { vx_host_llm(body.as_ptr(), body.len()) })
    }
}

struct WebCache;

impl vladx_hmn::Cache for WebCache {
    fn get(&mut self, key: &str) -> Option<String> {
        ST.with(|st| st.borrow().hmn_cache.get(key).cloned())
    }
    fn put(&mut self, key: &str, value: &str) {
        ST.with(|st| st.borrow_mut().hmn_cache.insert(key.to_string(), value.to_string()));
    }
}

/// Модель сервера ИИ (страница узнаёт её запросом GET к серверу).
///
/// # Safety
/// `ptr..ptr+len` — валидный буфер UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vx_set_llm(ptr: *const u8, len: usize) {
    let m = read_str(ptr, len);
    ST.with(|st| st.borrow_mut().llm_model = m);
}

/// Перевести `.vlhmn` в проверенный AST. Ключа в браузере нет: запросы к модели
/// уходят через импорт `vx_host_llm` на сервер с ключом. Результат — JSON
/// `{ok, program, vlstd, confidence, questions, llm_calls, cached, model, diagnostics}`.
///
/// # Safety
/// `ptr..ptr+len` — валидный буфер UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vx_translate(ptr: *const u8, len: usize, lang: u32) -> u32 {
    let text = read_str(ptr, len);
    let dict = ST.with(|st| st.borrow().dict.clone());
    let mut llm = WebLlm;
    let model = vladx_hmn::Llm::model(&llm);
    let r = vladx_hmn::translate(&text, lang_of(lang), &dict, vladx_runtime::GLOBAL_NAMES, &mut llm, &mut WebCache);
    let json = match r {
        Ok(t) => serde_json::json!({
            "ok": true,
            "program": vladx_ast::json_without_spans(&t.checked.program),
            "vlstd": t.checked.text,
            "confidence": t.confidence,
            "questions": t.questions,
            "llm_calls": t.llm_calls,
            "cached": t.llm_calls == 0 && t.prose_segments > 0,
            "model": model,
        }),
        Err(ds) => serde_json::json!({"ok": false, "diagnostics": ds.iter().map(diag_json).collect::<Vec<_>>()}),
    };
    let ok = json["ok"] == serde_json::Value::Bool(true);
    set_result(json.to_string());
    ok as u32
}

// ───────────── редактор: подсветка и живая проверка ─────────────

/// Режим «спроси»: 0 — только очередь ответов, 1 — остановка и повтор, 2 — ждать ответа (импорт vx_host_input).
#[no_mangle]
pub extern "C" fn vx_set_interactive(mode: u32) {
    ST.with(|st| st.borrow_mut().ask_mode = mode.min(2));
}

/// Разметка для подсветки: `{"t": [[строка, колонка, длина, вид], …], "e": ошибка | null}`.
///
/// # Safety
/// `ptr..ptr+len` — валидный буфер UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vx_tokens(ptr: *const u8, len: usize, lang: u32) -> u32 {
    let src = read_str(ptr, len);
    let dict = ST.with(|st| st.borrow().dict.clone());
    let (toks, err) = vladx_syntax::lexer::highlight(&src, lang_of(lang), &dict, vladx_runtime::GLOBAL_NAMES);
    let json = serde_json::json!({ "t": toks, "e": err.as_ref().map(diag_json) });
    set_result(json.to_string());
    1
}

/// Проверка без запуска (разбор + статика): `{"diagnostics": [...]}` с ошибками и предупреждениями.
///
/// # Safety
/// `ptr..ptr+len` — валидный буфер UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vx_check(ptr: *const u8, len: usize, lang: u32) -> u32 {
    let src = read_str(ptr, len);
    let dict = ST.with(|st| st.borrow().dict.clone());
    let (_, diags) = vladx_syntax::analyze(&src, lang_of(lang), &dict, vladx_runtime::GLOBAL_NAMES);
    let list: Vec<serde_json::Value> = diags
        .iter()
        .map(|d| {
            let mut v = diag_json(d);
            v["severity"] = (if d.is_error() { "error" } else { "warning" }).into();
            v
        })
        .collect();
    set_result(serde_json::json!({ "diagnostics": list }).to_string());
    diags.iter().all(|d| !d.is_error()) as u32
}

/// Папка главной программы в хранилище ("" — корень): от неё считаются `из "путь" возьми`.
///
/// # Safety
/// `ptr..ptr+len` — валидный буфер UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vx_set_main_dir(ptr: *const u8, len: usize) {
    let d = read_str(ptr, len);
    ST.with(|st| st.borrow_mut().main_dir = d);
}

// ───────────── vladpm в площадке ─────────────

/// Файлы хранилища страницы (OPFS или сервер) для менеджера пакетов.
struct WebFs;

impl vladx_pm::Fs for WebFs {
    fn read(&mut self, path: &str) -> Option<String> {
        WebHost.read(path).ok()
    }
    fn write(&mut self, path: &str, content: &str) -> Result<(), String> {
        WebHost.write(path, content)
    }
    fn remove_dir(&mut self, path: &str) -> Result<(), String> {
        let pre = format!("{path}/");
        for f in WebHost.list()?.into_iter().filter(|f| f.starts_with(&pre)) {
            WebHost.delete(&f)?;
        }
        Ok(())
    }
}

/// Менеджер пакетов: `{"op": "install"|"add"|"remove"|"update", "args": […],
/// "registry": "https://…", "lang": 0}`. Нет vladx.toml — создаётся на языке `lang`.
/// Результат — `{"ok": true, "report": {...}, "summary": "..."}` или `{"ok": false, "error": "..."}`.
///
/// # Safety
/// `ptr..ptr+len` — валидный буфер UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vx_pm(ptr: *const u8, len: usize) -> u32 {
    let req: serde_json::Value = serde_json::from_str(&read_str(ptr, len)).unwrap_or_default();
    let s = |k: &str| req.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let args: Vec<String> = req
        .get("args")
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let lang = Lang::from_index(req.get("lang").and_then(|v| v.as_u64()).unwrap_or(0) as u32).unwrap_or(Lang::Ru);
    let registry = { let r = s("registry"); if r.is_empty() { vladx_pm::DEFAULT_REGISTRY.to_string() } else { r } };
    let dict = ST.with(|st| st.borrow().dict.clone());
    let mut fs = WebFs;
    let mut reg = vladx_pm::GetRegistry {
        url: vladx_pm::endpoint(&registry),
        get: |u: &str| take_reply(vx_host_http(u.as_ptr(), u.len())),
    };
    let r = (|| -> Result<vladx_pm::Report, String> {
        use vladx_pm::Fs;
        if fs.read(vladx_pm::manifest::FILE).is_none() {
            let m = vladx_pm::Manifest {
                name: Some("площадка".into()),
                version: Some("1.0.0".into()),
                lang: Some(lang),
                ..vladx_pm::Manifest::default()
            };
            fs.write(vladx_pm::manifest::FILE, &vladx_pm::manifest::render(&m, lang, &dict))?;
        }
        match s("op").as_str() {
            "add" => vladx_pm::add(&mut fs, &mut reg, &dict, &args),
            "remove" => vladx_pm::remove(&mut fs, &mut reg, &dict, &args),
            "update" => {
                let names: std::collections::BTreeSet<String> = args.iter().cloned().collect();
                let all = names.is_empty();
                vladx_pm::install(&mut fs, &mut reg, &dict, &names, all)
            }
            _ => vladx_pm::install(&mut fs, &mut reg, &dict, &Default::default(), false),
        }
    })();
    let ok = r.is_ok();
    set_result(match r {
        Ok(rep) => serde_json::json!({ "ok": true, "summary": rep.summary(), "report": rep }).to_string(),
        Err(e) => serde_json::json!({ "ok": false, "error": e }).to_string(),
    });
    ok as u32
}

/// Разобрать vladx.toml (для панели «Пакеты»): `{"ok": true, "name", "lang", "dependencies", "scripts"}`.
///
/// # Safety
/// `ptr..ptr+len` — валидный буфер UTF-8.
#[no_mangle]
pub unsafe extern "C" fn vx_manifest(ptr: *const u8, len: usize) -> u32 {
    let src = read_str(ptr, len);
    let dict = ST.with(|st| st.borrow().dict.clone());
    let r = vladx_pm::manifest::parse(&src, &dict);
    let ok = r.is_ok();
    set_result(match r {
        Ok(m) => serde_json::json!({
            "ok": true,
            "name": m.name,
            "version": m.version,
            "lang": m.lang.map(|l| l.code()),
            "modules_dir": vladx_pm::manifest::modules_dir(&dict, m.lang.unwrap_or(Lang::Ru)),
            "dependencies": m.dependencies,
            "scripts": m.scripts,
        })
        .to_string(),
        Err(e) => serde_json::json!({ "ok": false, "error": e }).to_string(),
    });
    ok as u32
}
