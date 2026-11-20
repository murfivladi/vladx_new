//! VladX в WebAssembly. Только C-ABI: файл один и грузится «как есть».
//!
//! Экспорты: `vx_alloc`, `vx_free`, `vx_run`, `vx_parse`, `vx_result_ptr`,
//! `vx_result_len`, `vx_set_input`, `vx_set_time`, `vx_set_permissions`,
//! `vx_set_keywords`, `vx_version`, `vx_reset_host`.
//!
//! Импорты (`env.*`): `vx_host_read`, `vx_host_write`, `vx_host_delete`,
//! `vx_host_list`, `vx_host_get` — возвращают `u64 = (ptr << 32) | len` на буфер
//! `"ok\n…"` или `"err\n…"`, выделенный через `vx_alloc`; ядро освобождает его.
//! `vx_host_ask(kind)` — просьба о доступе (1 — файлы, 2 — сеть), 1 = разрешено.

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
}
#[cfg(not(target_arch = "wasm32"))]
use stubs::*;

struct State {
    dict: Dictionary,
    result: Vec<u8>,
    input: VecDeque<String>,
    time_ms: f64,
    perms: u32,
}

thread_local! {
    static ST: RefCell<State> = RefCell::new(State {
        dict: Dictionary::builtin(),
        result: Vec::new(),
        input: VecDeque::new(),
        time_ms: 0.0,
        perms: 3,
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
    fn input(&mut self, _prompt: &str) -> Option<String> {
        ST.with(|st| st.borrow_mut().input.pop_front())
    }
    fn now_ms(&mut self) -> f64 {
        ST.with(|st| st.borrow().time_ms)
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
    let (dict, perms) = ST.with(|st| {
        let st = st.borrow();
        (st.dict.clone(), st.perms)
    });
    let opts = Options {
        max_steps: if max_steps == 0 { 3_000_000 } else { max_steps as u64 },
        files: perms & 1 != 0,
        net: perms & 2 != 0,
        ..Options::default()
    };
    let r = vladx_runtime::run_source(&src, lang_of(lang), &dict, &mut WebHost, opts);
    let json = serde_json::json!({
        "ok": r.ok,
        "stdout": r.stdout,
        "warnings": r.warnings.iter().map(|w| format!("{}:{}  {}  {}", w.line, w.col, w.code, w.message)).collect::<Vec<_>>(),
        "diagnostics": r.diagnostics.iter().map(diag_json).collect::<Vec<_>>(),
    });
    set_result(json.to_string());
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
