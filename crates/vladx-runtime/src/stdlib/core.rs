//! `ядро` — нативная часть stdlib, как `internalBinding` в Node: только то,
//! чего не написать на самом VladX (доступ к файлам, сети, времени, случайности)
//! и горячие операции (сортировка, JSON, разбиение строк). Программе ядро не
//! видно: его получают только модули `vladx:…` (их исходники — `stdlib/*.vlstd`).

use crate::interp::{deep_eq, fail, fail_hint, Interp, R};
use crate::value::{type_name, Module, NativeFn, NativeImpl, Value};
use crate::Perm;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::rc::Rc;
use vladx_ast::Span;

pub(crate) fn native(name: &str, min: usize, max: usize, f: NativeImpl) -> Value {
    Value::Native(Rc::new(NativeFn { name: name.to_string(), min, max, f }))
}

/// Модуль `ядро`.
pub fn module() -> Value {
    let fns: Vec<(&str, usize, usize, NativeImpl)> = vec![
        ("тип", 1, 1, k_type),
        ("ошибка", 2, 3, k_error),
        ("длина", 1, 1, g_len),
        ("равны", 2, 2, k_eq),
        // строки
        ("символы", 1, 1, k_chars),
        ("кусок_строки", 2, 3, k_str_slice),
        ("найди_в_строке", 2, 3, k_str_find),
        ("верхний", 1, 1, k_upper),
        ("нижний", 1, 1, k_lower),
        ("пробельный", 1, 1, k_space),
        ("раздели", 2, 2, k_split),
        ("соедини", 2, 2, k_join),
        ("код_символа", 1, 1, k_code),
        ("символ_кода", 1, 1, k_from_code),
        // списки
        ("элемент", 2, 2, k_get),
        ("задай_элемент", 3, 3, k_set),
        ("добавь", 2, 2, k_push),
        ("вынь", 1, 1, k_pop),
        ("кусок_списка", 2, 3, k_list_slice),
        ("отсортируй", 1, 1, k_sort),
        // словари
        ("ключи", 1, 1, k_keys),
        ("значения", 1, 1, k_values),
        ("есть_ключ", 2, 2, k_has),
        ("поле", 2, 2, k_field),
        ("задай_поле", 3, 3, k_set_field),
        ("удали_ключ", 2, 2, k_remove),
        // числа, случайность, время
        ("корень", 1, 1, k_sqrt),
        ("пол", 1, 1, k_floor),
        ("потолок", 1, 1, k_ceil),
        ("округли", 1, 2, k_round),
        ("степень", 2, 2, k_pow),
        ("дробь", 0, 0, k_frac),
        ("миллисекунды", 0, 0, k_ms),
        // json
        ("в_json", 1, 1, k_to_json),
        ("из_json", 1, 1, k_from_json),
        // файлы и сеть (с проверкой разрешений)
        ("прочитай_файл", 1, 1, k_read),
        ("запиши_файл", 2, 2, k_write),
        ("удали_файл", 1, 1, k_delete),
        ("есть_файл", 1, 1, k_exists),
        ("список_файлов", 0, 0, k_list),
        ("получи", 1, 1, k_get_url),
        ("сеть_разрешена", 0, 0, k_net_allowed),
    ];
    let mut m = IndexMap::new();
    for (name, min, max, f) in fns {
        m.insert(name.to_string(), native(&format!("ядро.{name}"), min, max, f));
    }
    Value::Module(Rc::new(Module { name: "ядро".into(), names: vec![], fns: m }))
}

// ───────────── извлечение аргументов ─────────────

fn type_err<T>(f: &str, expected: &str, got: &Value, sp: Span) -> R<T> {
    fail(
        "E-TYPE-001",
        format!("{f}: ожидалась {expected}, получено «{}»", type_name(got.type_id())),
        sp,
    )
}

fn arg_s(f: &str, a: &[Value], i: usize, sp: Span) -> R<Rc<str>> {
    match &a[i] {
        Value::Str(s) => Ok(s.clone()),
        v => type_err(f, "строка", v, sp),
    }
}
fn arg_n(f: &str, a: &[Value], i: usize, sp: Span) -> R<f64> {
    match &a[i] {
        Value::Num(n) => Ok(*n),
        v => type_err(f, "число", v, sp),
    }
}
fn arg_l(f: &str, a: &[Value], i: usize, sp: Span) -> R<Rc<RefCell<Vec<Value>>>> {
    match &a[i] {
        Value::List(l) => Ok(l.clone()),
        v => type_err(f, "список", v, sp),
    }
}
fn arg_m(f: &str, a: &[Value], i: usize, sp: Span) -> R<Rc<RefCell<crate::value::MapObj>>> {
    match &a[i] {
        Value::Map(m) => Ok(m.clone()),
        v => type_err(f, "словарь", v, sp),
    }
}

/// Индекс в границах [0, len] (для «кусок»).
fn clamp_idx(n: f64, len: usize) -> usize {
    if n.is_nan() || n < 0.0 {
        0
    } else {
        (n.floor() as usize).min(len)
    }
}

fn sort_key(v: &Value) -> u8 {
    match v {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Num(_) => 2,
        Value::Str(_) => 3,
        _ => 4,
    }
}
pub fn to_json(v: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::Null | Value::Func(_) | Value::Native(_) | Value::Module(_) => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Num(n) => {
            if n.fract() == 0.0 && n.abs() < 9.0e15 {
                J::from(*n as i64)
            } else {
                serde_json::Number::from_f64(*n).map(J::Number).unwrap_or(J::Null)
            }
        }
        Value::Str(s) => J::String(s.to_string()),
        Value::List(l) => J::Array(l.borrow().iter().map(to_json).collect()),
        Value::Map(m) => {
            let mut o = serde_json::Map::new();
            for (k, x) in &m.borrow().entries {
                if !matches!(x, Value::Func(_) | Value::Native(_)) {
                    o.insert(k.clone(), to_json(x));
                }
            }
            J::Object(o)
        }
        Value::Error(e) => {
            let mut o = serde_json::Map::new();
            o.insert("код".into(), J::String(e.code.clone()));
            o.insert("сообщение".into(), J::String(e.message.clone()));
            J::Object(o)
        }
    }
}

pub fn from_json(j: &serde_json::Value) -> Value {
    use serde_json::Value as J;
    match j {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => Value::Num(n.as_f64().unwrap_or(f64::NAN)),
        J::String(s) => Value::str(s),
        J::Array(a) => Value::list(a.iter().map(from_json).collect()),
        J::Object(o) => Value::map(o.iter().map(|(k, v)| (k.clone(), from_json(v))).collect(), None),
    }
}

fn parse_json(s: &str, sp: Span) -> R<Value> {
    match serde_json::from_str::<serde_json::Value>(s) {
        Ok(j) => Ok(from_json(&j)),
        Err(e) => fail("E-JSON-001", format!("Не разобрать JSON: {e}"), sp),
    }
}

// ───────────── общее ─────────────

fn k_type(_: &mut Interp, a: &[Value], _: Span) -> R<Value> {
    Ok(Value::str(type_name(a[0].type_id())))
}
/// `ядро.ошибка(код, сообщение, подсказка?)` — ошибка исполнения, её ловит «попробуй».
fn k_error(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let code = it.display(&a[0]);
    let msg = it.display(&a[1]);
    match a.get(2) {
        Some(Value::Null) | None => fail(&code, msg, sp),
        Some(h) => {
            let h = it.display(h);
            fail_hint(&code, msg, h, sp)
        }
    }
}
fn k_eq(_: &mut Interp, a: &[Value], _: Span) -> R<Value> {
    Ok(Value::Bool(deep_eq(&a[0], &a[1])))
}
pub(crate) fn g_len(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    match &a[0] {
        Value::Str(s) => Ok(Value::Num(s.chars().count() as f64)),
        Value::List(l) => Ok(Value::Num(l.borrow().len() as f64)),
        Value::Map(m) => Ok(Value::Num(m.borrow().entries.len() as f64)),
        v => type_err("длина", "строка, список или словарь", v, sp),
    }
}

// ───────────── строки ─────────────

fn k_chars(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("символы", a, 0, sp)?;
    Ok(Value::list(s.chars().map(|c| Value::str(c.to_string())).collect()))
}
fn k_str_slice(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("кусок", a, 0, sp)?;
    let chars: Vec<char> = s.chars().collect();
    let from = clamp_idx(arg_n("кусок", a, 1, sp)?, chars.len());
    let to = match a.get(2) {
        Some(Value::Null) | None => chars.len(),
        Some(_) => clamp_idx(arg_n("кусок", a, 2, sp)?, chars.len()),
    };
    if to < from {
        return Ok(Value::str(""));
    }
    Ok(Value::str(chars[from..to].iter().collect::<String>()))
}
/// Индекс (в символах) первого вхождения начиная с `от`, или -1.
fn k_str_find(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("найди", a, 0, sp)?;
    let what = arg_s("найди", a, 1, sp)?;
    let from = match a.get(2) {
        Some(Value::Null) | None => 0,
        Some(_) => clamp_idx(arg_n("найди", a, 2, sp)?, s.chars().count()),
    };
    let start = s.char_indices().nth(from).map(|(b, _)| b).unwrap_or(s.len());
    Ok(Value::Num(match s[start..].find(&*what) {
        Some(b) => (from + s[start..start + b].chars().count()) as f64,
        None => -1.0,
    }))
}
fn k_upper(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    Ok(Value::str(arg_s("верхний", a, 0, sp)?.to_uppercase()))
}
fn k_lower(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    Ok(Value::str(arg_s("нижний", a, 0, sp)?.to_lowercase()))
}
fn k_space(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("пробельный", a, 0, sp)?;
    Ok(Value::Bool(!s.is_empty() && s.chars().all(char::is_whitespace)))
}
fn k_split(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("раздели", a, 0, sp)?;
    let sep = arg_s("раздели", a, 1, sp)?;
    let parts: Vec<Value> = if sep.is_empty() {
        s.chars().map(|c| Value::str(c.to_string())).collect()
    } else {
        s.split(&*sep).map(Value::str).collect()
    };
    Ok(Value::list(parts))
}
fn k_join(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let l = arg_l("соедини", a, 0, sp)?;
    let sep = arg_s("соедини", a, 1, sp)?;
    let l = l.borrow();
    Ok(Value::str(l.iter().map(|v| it.display(v)).collect::<Vec<_>>().join(&sep)))
}
fn k_code(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("код", a, 0, sp)?;
    match s.chars().next() {
        Some(c) => Ok(Value::Num(c as u32 as f64)),
        None => fail("E-VALUE-001", "код: пустая строка", sp),
    }
}
fn k_from_code(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let n = arg_n("из_кода", a, 0, sp)?;
    match char::from_u32(n as u32).filter(|_| n >= 0.0 && n.fract() == 0.0) {
        Some(c) => Ok(Value::str(c.to_string())),
        None => fail("E-VALUE-001", format!("из_кода: {} — не код символа", Interp::fmt_num(n)), sp),
    }
}

// ───────────── списки ─────────────

fn index(l: &[Value], i: f64, sp: Span) -> R<usize> {
    if i < 0.0 || i.fract() != 0.0 || i as usize >= l.len() {
        return fail_hint(
            "E-INDEX-001",
            format!("Индекс {} вне списка длиной {}", Interp::fmt_num(i), l.len()),
            "индексы считаются с 0",
            sp,
        );
    }
    Ok(i as usize)
}
fn k_get(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let l = arg_l("элемент", a, 0, sp)?;
    let i = arg_n("элемент", a, 1, sp)?;
    let l = l.borrow();
    let i = index(&l, i, sp)?;
    Ok(l[i].clone())
}
fn k_set(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let l = arg_l("задай_элемент", a, 0, sp)?;
    let i = arg_n("задай_элемент", a, 1, sp)?;
    let mut l = l.borrow_mut();
    let i = index(&l, i, sp)?;
    l[i] = a[2].clone();
    Ok(Value::Null)
}
fn k_push(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let l = arg_l("добавь", a, 0, sp)?;
    l.borrow_mut().push(a[1].clone());
    Ok(a[0].clone())
}
fn k_pop(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let l = arg_l("вынь", a, 0, sp)?;
    let v = l.borrow_mut().pop();
    Ok(v.unwrap_or(Value::Null))
}
fn k_list_slice(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let l = arg_l("кусок", a, 0, sp)?;
    let l = l.borrow();
    let from = clamp_idx(arg_n("кусок", a, 1, sp)?, l.len());
    let to = match a.get(2) {
        Some(Value::Null) | None => l.len(),
        Some(_) => clamp_idx(arg_n("кусок", a, 2, sp)?, l.len()),
    };
    if to < from {
        return Ok(Value::list(vec![]));
    }
    Ok(Value::list(l[from..to].to_vec()))
}
/// Копия, отсортированная по возрастанию: ничего < логические < числа < строки < остальное.
fn k_sort(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let mut v = arg_l("отсортируй", a, 0, sp)?.borrow().clone();
    v.sort_by(|x, y| match (x, y) {
        (Value::Num(p), Value::Num(q)) => p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal),
        (Value::Str(p), Value::Str(q)) => p.cmp(q),
        (Value::Bool(p), Value::Bool(q)) => p.cmp(q),
        _ => sort_key(x).cmp(&sort_key(y)),
    });
    Ok(Value::list(v))
}

// ───────────── словари ─────────────

fn key(it: &Interp, v: &Value) -> String {
    it.display(v)
}
fn k_keys(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let m = arg_m("ключи", a, 0, sp)?;
    let keys = m.borrow().entries.keys().map(Value::str).collect();
    Ok(Value::list(keys))
}
fn k_values(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let m = arg_m("значения", a, 0, sp)?;
    let vals = m.borrow().entries.values().cloned().collect();
    Ok(Value::list(vals))
}
fn k_has(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let m = arg_m("есть", a, 0, sp)?;
    let k = key(it, &a[1]);
    let has = m.borrow().entries.contains_key(&k);
    Ok(Value::Bool(has))
}
fn k_field(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let m = arg_m("значение", a, 0, sp)?;
    let k = key(it, &a[1]);
    let v = m.borrow().entries.get(&k).cloned();
    Ok(v.unwrap_or(Value::Null))
}
fn k_set_field(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let m = arg_m("задай", a, 0, sp)?;
    let k = key(it, &a[1]);
    m.borrow_mut().entries.insert(k, a[2].clone());
    Ok(a[0].clone())
}
fn k_remove(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let m = arg_m("удали", a, 0, sp)?;
    let k = key(it, &a[1]);
    let removed = m.borrow_mut().entries.shift_remove(&k).is_some();
    Ok(Value::Bool(removed))
}

// ───────────── числа, случайность, время ─────────────

fn k_sqrt(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let x = arg_n("корень", a, 0, sp)?;
    if x < 0.0 {
        return fail("E-MATH-003", "Корень из отрицательного числа", sp);
    }
    Ok(Value::Num(x.sqrt()))
}
fn k_floor(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    Ok(Value::Num(arg_n("пол", a, 0, sp)?.floor()))
}
fn k_ceil(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    Ok(Value::Num(arg_n("потолок", a, 0, sp)?.ceil()))
}
fn k_round(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let x = arg_n("округли", a, 0, sp)?;
    match a.get(1) {
        Some(Value::Null) | None => Ok(Value::Num(x.round())),
        Some(_) => {
            let d = arg_n("округли", a, 1, sp)?.clamp(0.0, 15.0).floor();
            let k = 10f64.powf(d);
            Ok(Value::Num((x * k).round() / k))
        }
    }
}
fn k_pow(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    Ok(Value::Num(arg_n("степень", a, 0, sp)?.powf(arg_n("степень", a, 1, sp)?)))
}
fn k_frac(it: &mut Interp, _: &[Value], _: Span) -> R<Value> {
    Ok(Value::Num(it.random()))
}
fn k_ms(it: &mut Interp, _: &[Value], _: Span) -> R<Value> {
    Ok(Value::Num(it.host.now_ms()))
}

// ───────────── json ─────────────

fn k_to_json(_: &mut Interp, a: &[Value], _: Span) -> R<Value> {
    Ok(Value::str(to_json(&a[0]).to_string()))
}
fn k_from_json(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("json.прочитай", a, 0, sp)?;
    parse_json(&s, sp)
}

// ───────────── файлы и сеть ─────────────

fn k_read(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let p = arg_s("прочитай", a, 0, sp)?;
    it.need(Perm::Files, sp)?;
    match it.host.read(&p) {
        Ok(s) => Ok(Value::str(s)),
        Err(e) => fail("E-FILE-001", e, sp),
    }
}
fn k_write(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let p = arg_s("запиши", a, 0, sp)?;
    let data = it.display(&a[1]);
    it.need(Perm::Files, sp)?;
    match it.host.write(&p, &data) {
        Ok(()) => Ok(Value::Null),
        Err(e) => fail("E-FILE-002", e, sp),
    }
}
fn k_exists(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let p = arg_s("есть", a, 0, sp)?;
    it.need(Perm::Files, sp)?;
    Ok(Value::Bool(it.host.read(&p).is_ok()))
}
fn k_list(it: &mut Interp, _: &[Value], sp: Span) -> R<Value> {
    it.need(Perm::Files, sp)?;
    match it.host.list() {
        Ok(mut v) => {
            v.sort();
            Ok(Value::list(v.into_iter().map(Value::str).collect()))
        }
        Err(e) => fail("E-FILE-001", e, sp),
    }
}
fn k_delete(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let p = arg_s("удали", a, 0, sp)?;
    it.need(Perm::Files, sp)?;
    match it.host.delete(&p) {
        Ok(()) => Ok(Value::Null),
        Err(e) => fail("E-FILE-003", e, sp),
    }
}
fn k_get_url(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let url = arg_s("получи", a, 0, sp)?;
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return fail_hint(
            "E-NET-001",
            format!("Адрес «{url}» должен начинаться с http:// или https://"),
            "например: https://example.com",
            sp,
        );
    }
    it.need(Perm::Net, sp)?;
    it.host.get(&url).map(Value::str).or_else(|e| fail("E-NET-001", e, sp))
}
fn k_net_allowed(it: &mut Interp, _: &[Value], _: Span) -> R<Value> {
    Ok(Value::Bool(it.opts.net))
}

// ───────────── глобальные функции ─────────────

pub(crate) fn g_str(it: &mut Interp, a: &[Value], _: Span) -> R<Value> {
    Ok(Value::str(it.display(&a[0])))
}
pub(crate) fn g_num(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    match &a[0] {
        Value::Num(n) => Ok(Value::Num(*n)),
        Value::Bool(b) => Ok(Value::Num(if *b { 1.0 } else { 0.0 })),
        Value::Str(s) => match s.trim().parse::<f64>() {
            Ok(n) if !s.trim().is_empty() => Ok(Value::Num(n)),
            _ => fail_hint(
                "E-TYPE-001",
                format!("число: не удалось превратить «{s}» в число"),
                "число пишется цифрами, дробная часть — через точку: 3.5",
                sp,
            ),
        },
        v => type_err("число", "строка или число", v, sp),
    }
}
