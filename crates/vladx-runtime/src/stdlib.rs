//! Стандартная библиотека: 9 модулей + глобальные функции.
//! Нативный Rust — только там, где нужен доступ к платформе или скорость.

use crate::interp::{deep_eq, fail, fail_hint, Interp, R};
use crate::value::{type_name, Env, Module, NativeFn, NativeImpl, Value};
use crate::Perm;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::rc::Rc;
use vladx_ast::Span;

/// Имена, которые среда кладёт в глобальный скоуп (для статических проверок).
pub const GLOBAL_NAMES: &[&str] = &[
    "numeri", "testo", "liste", "mappe", "caso", "tempo", "json", "file", "rete", "длина", "размер", "строка", "число",
];

fn native(name: &str, min: usize, max: usize, f: NativeImpl) -> Value {
    Value::Native(Rc::new(NativeFn { name: name.to_string(), min, max, f }))
}

fn module(globals: &Rc<Env>, name: &str, fns: Vec<(&str, usize, usize, NativeImpl)>) {
    let mut m = IndexMap::new();
    for (fname, min, max, f) in fns {
        m.insert(fname.to_string(), native(&format!("{name}.{fname}"), min, max, f));
    }
    globals.define(name, Value::Module(Rc::new(Module { name: name.to_string(), fns: m })), false);
}

pub fn install(it: &mut Interp) {
    let g = it.globals.clone();
    module(&g, "numeri", vec![
        ("корень", 1, 1, n_sqrt),
        ("модуль", 1, 1, n_abs),
        ("округли", 1, 2, n_round),
        ("пол", 1, 1, n_floor),
        ("потолок", 1, 1, n_ceil),
        ("случайное", 0, 2, n_random),
    ]);
    module(&g, "testo", vec![
        ("длина", 1, 1, t_len),
        ("верхний", 1, 1, t_upper),
        ("нижний", 1, 1, t_lower),
        ("обрежь", 1, 1, t_trim),
        ("раздели", 1, 2, t_split),
        ("соедини", 1, 2, t_join),
        ("содержит", 2, 2, t_contains),
        ("начинается", 2, 2, t_starts),
        ("заканчивается", 2, 2, t_ends),
        ("замени", 3, 3, t_replace),
        ("кусок", 2, 3, t_slice),
        ("символ", 2, 2, t_char),
        ("повтори", 2, 2, t_repeat),
        ("слова", 1, 1, t_words),
    ]);
    module(&g, "liste", vec![
        ("добавь", 2, 2, l_push),
        ("размер", 1, 1, l_len),
        ("возьми", 2, 2, l_get),
        ("сортировка", 1, 1, l_sort),
        ("переверни", 1, 1, l_reverse),
        ("содержит", 2, 2, l_contains),
        ("найди", 2, 2, l_find),
        ("кусок", 2, 3, l_slice),
        ("соедини", 1, 2, l_join),
        ("выбери", 1, 1, c_pick),
    ]);
    module(&g, "mappe", vec![
        ("ключи", 1, 1, m_keys),
        ("значения", 1, 1, m_values),
        ("есть", 2, 2, m_has),
        ("размер", 1, 1, m_len),
        ("удали", 2, 2, m_remove),
    ]);
    module(&g, "caso", vec![
        ("дробь", 0, 0, c_frac),
        ("целое", 2, 2, c_int),
        ("выбери", 1, 1, c_pick),
        ("перемешай", 1, 1, c_shuffle),
    ]);
    module(&g, "tempo", vec![
        ("сейчас", 0, 0, tm_now),
        ("миллисекунды", 0, 0, tm_ms),
        ("часы", 1, 1, tm_clock),
    ]);
    module(&g, "json", vec![("запиши", 1, 1, j_write), ("прочитай", 1, 1, j_read)]);
    module(&g, "file", vec![
        ("прочитай", 1, 1, f_read),
        ("запиши", 2, 2, f_write),
        ("есть", 1, 1, f_exists),
        ("список", 0, 0, f_list),
        ("удали", 1, 1, f_delete),
    ]);
    module(&g, "rete", vec![("получи", 1, 1, r_get), ("получиJson", 1, 1, r_get_json), ("разрешено", 0, 0, r_allowed)]);
    g.define("длина", native("длина", 1, 1, g_len), false);
    g.define("размер", native("размер", 1, 1, g_len), false);
    g.define("строка", native("строка", 1, 1, g_str), false);
    g.define("число", native("число", 1, 1, g_num), false);
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

// ───────────── numeri ─────────────

fn n_sqrt(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let x = arg_n("numeri.корень", a, 0, sp)?;
    if x < 0.0 {
        return fail("E-MATH-003", "Корень из отрицательного числа", sp);
    }
    Ok(Value::Num(x.sqrt()))
}
fn n_abs(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    Ok(Value::Num(arg_n("numeri.модуль", a, 0, sp)?.abs()))
}
fn n_round(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let x = arg_n("numeri.округли", a, 0, sp)?;
    if a.len() == 2 {
        let d = arg_n("numeri.округли", a, 1, sp)?.clamp(0.0, 15.0).floor();
        let k = 10f64.powf(d);
        return Ok(Value::Num((x * k).round() / k));
    }
    Ok(Value::Num(x.round()))
}
fn n_floor(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    Ok(Value::Num(arg_n("numeri.пол", a, 0, sp)?.floor()))
}
fn n_ceil(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    Ok(Value::Num(arg_n("numeri.потолок", a, 0, sp)?.ceil()))
}
fn n_random(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    match a.len() {
        0 => Ok(Value::Num(it.random())),
        2 => c_int(it, a, sp),
        _ => fail("E-ARGS-002", "numeri.случайное: передайте ничего или две границы (от, до)", sp),
    }
}

// ───────────── testo ─────────────

fn t_len(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    Ok(Value::Num(arg_s("testo.длина", a, 0, sp)?.chars().count() as f64))
}
fn t_upper(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    Ok(Value::str(arg_s("testo.верхний", a, 0, sp)?.to_uppercase()))
}
fn t_lower(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    Ok(Value::str(arg_s("testo.нижний", a, 0, sp)?.to_lowercase()))
}
fn t_trim(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    Ok(Value::str(arg_s("testo.обрежь", a, 0, sp)?.trim()))
}
fn t_split(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("testo.раздели", a, 0, sp)?;
    let sep = if a.len() > 1 { arg_s("testo.раздели", a, 1, sp)? } else { Rc::from(" ") };
    let parts: Vec<Value> = if sep.is_empty() {
        s.chars().map(|c| Value::str(c.to_string())).collect()
    } else {
        s.split(&*sep).map(Value::str).collect()
    };
    Ok(Value::list(parts))
}
fn join_list(it: &Interp, l: &[Value], sep: &str) -> String {
    l.iter().map(|v| it.display(v)).collect::<Vec<_>>().join(sep)
}
fn t_join(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let l = arg_l("testo.соедини", a, 0, sp)?;
    let sep = if a.len() > 1 { arg_s("testo.соедини", a, 1, sp)? } else { Rc::from("") };
    let l = l.borrow();
    Ok(Value::str(join_list(it, &l, &sep)))
}
fn t_contains(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("testo.содержит", a, 0, sp)?;
    let sub = arg_s("testo.содержит", a, 1, sp)?;
    Ok(Value::Bool(s.contains(&*sub)))
}
fn t_starts(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("testo.начинается", a, 0, sp)?;
    Ok(Value::Bool(s.starts_with(&*arg_s("testo.начинается", a, 1, sp)?)))
}
fn t_ends(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("testo.заканчивается", a, 0, sp)?;
    Ok(Value::Bool(s.ends_with(&*arg_s("testo.заканчивается", a, 1, sp)?)))
}
fn t_replace(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("testo.замени", a, 0, sp)?;
    let from = arg_s("testo.замени", a, 1, sp)?;
    let to = arg_s("testo.замени", a, 2, sp)?;
    if from.is_empty() {
        return Ok(Value::Str(s));
    }
    Ok(Value::str(s.replace(&*from, &to)))
}
fn t_slice(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("testo.кусок", a, 0, sp)?;
    let chars: Vec<char> = s.chars().collect();
    let from = clamp_idx(arg_n("testo.кусок", a, 1, sp)?, chars.len());
    let to = if a.len() > 2 { clamp_idx(arg_n("testo.кусок", a, 2, sp)?, chars.len()) } else { chars.len() };
    if to < from {
        return Ok(Value::str(""));
    }
    Ok(Value::str(chars[from..to].iter().collect::<String>()))
}
fn t_char(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("testo.символ", a, 0, sp)?;
    let i = arg_n("testo.символ", a, 1, sp)?;
    let n = s.chars().count();
    if i < 0.0 || i.fract() != 0.0 || i as usize >= n {
        return fail("E-INDEX-001", format!("Индекс {} вне строки длиной {n}", Interp::fmt_num(i)), sp);
    }
    Ok(Value::str(s.chars().nth(i as usize).unwrap().to_string()))
}
fn t_repeat(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("testo.повтори", a, 0, sp)?;
    let n = arg_n("testo.повтори", a, 1, sp)?;
    if n < 0.0 {
        return fail("E-VALUE-001", "Число повторов не может быть отрицательным", sp);
    }
    if s.len() as f64 * n > 10_000_000.0 {
        return fail("E-VALUE-002", "testo.повтори: слишком длинная строка получится", sp);
    }
    Ok(Value::str(s.repeat(n.floor() as usize)))
}
fn t_words(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("testo.слова", a, 0, sp)?;
    Ok(Value::list(s.split_whitespace().map(Value::str).collect()))
}

// ───────────── liste ─────────────

fn l_push(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let l = arg_l("liste.добавь", a, 0, sp)?;
    l.borrow_mut().push(a[1].clone());
    Ok(a[0].clone())
}
fn l_len(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    Ok(Value::Num(arg_l("liste.размер", a, 0, sp)?.borrow().len() as f64))
}
fn l_get(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let l = arg_l("liste.возьми", a, 0, sp)?;
    let i = arg_n("liste.возьми", a, 1, sp)?;
    let l = l.borrow();
    if i < 0.0 || i.fract() != 0.0 || i as usize >= l.len() {
        return fail_hint(
            "E-INDEX-001",
            format!("Индекс {} вне списка длиной {}", Interp::fmt_num(i), l.len()),
            "индексы считаются с 0",
            sp,
        );
    }
    Ok(l[i as usize].clone())
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
fn l_sort(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let mut v = arg_l("liste.сортировка", a, 0, sp)?.borrow().clone();
    v.sort_by(|x, y| match (x, y) {
        (Value::Num(p), Value::Num(q)) => p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal),
        (Value::Str(p), Value::Str(q)) => p.cmp(q),
        (Value::Bool(p), Value::Bool(q)) => p.cmp(q),
        _ => sort_key(x).cmp(&sort_key(y)),
    });
    Ok(Value::list(v))
}
fn l_reverse(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let mut v = arg_l("liste.переверни", a, 0, sp)?.borrow().clone();
    v.reverse();
    Ok(Value::list(v))
}
fn l_contains(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let l = arg_l("liste.содержит", a, 0, sp)?;
    let found = l.borrow().iter().any(|x| deep_eq(x, &a[1]));
    Ok(Value::Bool(found))
}
fn l_find(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let l = arg_l("liste.найди", a, 0, sp)?;
    let pos = l.borrow().iter().position(|x| deep_eq(x, &a[1]));
    Ok(Value::Num(pos.map(|p| p as f64).unwrap_or(-1.0)))
}
fn l_slice(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let l = arg_l("liste.кусок", a, 0, sp)?;
    let l = l.borrow();
    let from = clamp_idx(arg_n("liste.кусок", a, 1, sp)?, l.len());
    let to = if a.len() > 2 { clamp_idx(arg_n("liste.кусок", a, 2, sp)?, l.len()) } else { l.len() };
    if to < from {
        return Ok(Value::list(vec![]));
    }
    Ok(Value::list(l[from..to].to_vec()))
}
/// `liste.соедини(список, "разделитель")` → строка; `liste.соедини(а, б)` → новый список.
fn l_join(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let l = arg_l("liste.соедини", a, 0, sp)?;
    match a.get(1) {
        Some(Value::List(other)) => {
            let mut v = l.borrow().clone();
            v.extend(other.borrow().iter().cloned());
            Ok(Value::list(v))
        }
        Some(Value::Str(sep)) => Ok(Value::str(join_list(it, &l.borrow(), sep))),
        None => Ok(Value::str(join_list(it, &l.borrow(), ""))),
        Some(v) => type_err("liste.соедини", "строка-разделитель или список", v, sp),
    }
}

// ───────────── mappe ─────────────

fn m_keys(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let m = arg_m("mappe.ключи", a, 0, sp)?;
    let keys = m.borrow().entries.keys().map(Value::str).collect();
    Ok(Value::list(keys))
}
fn m_values(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let m = arg_m("mappe.значения", a, 0, sp)?;
    let vals = m.borrow().entries.values().cloned().collect();
    Ok(Value::list(vals))
}
fn m_has(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let m = arg_m("mappe.есть", a, 0, sp)?;
    let k = it.display(&a[1]);
    let has = m.borrow().entries.contains_key(&k);
    Ok(Value::Bool(has))
}
fn m_len(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    Ok(Value::Num(arg_m("mappe.размер", a, 0, sp)?.borrow().entries.len() as f64))
}
fn m_remove(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let m = arg_m("mappe.удали", a, 0, sp)?;
    let k = it.display(&a[1]);
    let removed = m.borrow_mut().entries.shift_remove(&k).is_some();
    Ok(Value::Bool(removed))
}

// ───────────── caso ─────────────

fn c_frac(it: &mut Interp, _: &[Value], _: Span) -> R<Value> {
    Ok(Value::Num(it.random()))
}
fn c_int(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let lo = arg_n("caso.целое", a, 0, sp)?.ceil();
    let hi = arg_n("caso.целое", a, 1, sp)?.floor();
    if hi < lo {
        return fail("E-VALUE-002", "Пустой диапазон: верхняя граница меньше нижней", sp);
    }
    let r = it.random();
    Ok(Value::Num((lo + (r * (hi - lo + 1.0)).floor()).min(hi)))
}
fn c_pick(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let l = arg_l("выбери", a, 0, sp)?;
    let n = l.borrow().len();
    if n == 0 {
        return Ok(Value::Null);
    }
    let i = ((it.random() * n as f64) as usize).min(n - 1);
    let v = l.borrow()[i].clone();
    Ok(v)
}
fn c_shuffle(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let mut v = arg_l("caso.перемешай", a, 0, sp)?.borrow().clone();
    for i in (1..v.len()).rev() {
        let j = ((it.random() * (i + 1) as f64) as usize).min(i);
        v.swap(i, j);
    }
    Ok(Value::list(v))
}

// ───────────── tempo ─────────────

fn clock(total_secs: f64) -> String {
    let t = total_secs.max(0.0).floor() as u64;
    format!("{:02}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
}
fn tm_now(it: &mut Interp, _: &[Value], _: Span) -> R<Value> {
    let ms = it.host.now_ms();
    let secs_of_day = (ms / 1000.0).floor().rem_euclid(86_400.0);
    Ok(Value::str(clock(secs_of_day)))
}
fn tm_ms(it: &mut Interp, _: &[Value], _: Span) -> R<Value> {
    Ok(Value::Num(it.host.now_ms()))
}
fn tm_clock(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    Ok(Value::str(clock(arg_n("tempo.часы", a, 0, sp)?)))
}

// ───────────── json ─────────────

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

fn j_write(_: &mut Interp, a: &[Value], _: Span) -> R<Value> {
    Ok(Value::str(to_json(&a[0]).to_string()))
}
fn parse_json(s: &str, sp: Span) -> R<Value> {
    match serde_json::from_str::<serde_json::Value>(s) {
        Ok(j) => Ok(from_json(&j)),
        Err(e) => fail("E-JSON-001", format!("Не разобрать JSON: {e}"), sp),
    }
}
fn j_read(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let s = arg_s("json.прочитай", a, 0, sp)?;
    parse_json(&s, sp)
}

// ───────────── file ─────────────

fn f_read(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let p = arg_s("file.прочитай", a, 0, sp)?;
    it.need(Perm::Files, sp)?;
    match it.host.read(&p) {
        Ok(s) => Ok(Value::str(s)),
        Err(e) => fail("E-FILE-001", e, sp),
    }
}
fn f_write(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let p = arg_s("file.запиши", a, 0, sp)?;
    let data = it.display(&a[1]);
    it.need(Perm::Files, sp)?;
    match it.host.write(&p, &data) {
        Ok(()) => Ok(Value::Null),
        Err(e) => fail("E-FILE-002", e, sp),
    }
}
fn f_exists(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let p = arg_s("file.есть", a, 0, sp)?;
    it.need(Perm::Files, sp)?;
    Ok(Value::Bool(it.host.read(&p).is_ok()))
}
fn f_list(it: &mut Interp, _: &[Value], sp: Span) -> R<Value> {
    it.need(Perm::Files, sp)?;
    match it.host.list() {
        Ok(mut v) => {
            v.sort();
            Ok(Value::list(v.into_iter().map(Value::str).collect()))
        }
        Err(e) => fail("E-FILE-001", e, sp),
    }
}
fn f_delete(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let p = arg_s("file.удали", a, 0, sp)?;
    it.need(Perm::Files, sp)?;
    match it.host.delete(&p) {
        Ok(()) => Ok(Value::Null),
        Err(e) => fail("E-FILE-003", e, sp),
    }
}

// ───────────── rete ─────────────

fn http(it: &mut Interp, f: &str, a: &[Value], sp: Span) -> R<String> {
    let url = arg_s(f, a, 0, sp)?;
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return fail_hint(
            "E-NET-001",
            format!("Адрес «{url}» должен начинаться с http:// или https://"),
            "например: https://example.com",
            sp,
        );
    }
    it.need(Perm::Net, sp)?;
    it.host.get(&url).or_else(|e| fail("E-NET-001", e, sp))
}
fn r_get(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    Ok(Value::str(http(it, "rete.получи", a, sp)?))
}
fn r_get_json(it: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    let body = http(it, "rete.получиJson", a, sp)?;
    parse_json(&body, sp)
}
fn r_allowed(it: &mut Interp, _: &[Value], _: Span) -> R<Value> {
    Ok(Value::Bool(it.opts.net))
}

// ───────────── глобальные ─────────────

fn g_len(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
    match &a[0] {
        Value::Str(s) => Ok(Value::Num(s.chars().count() as f64)),
        Value::List(l) => Ok(Value::Num(l.borrow().len() as f64)),
        Value::Map(m) => Ok(Value::Num(m.borrow().entries.len() as f64)),
        v => type_err("длина", "строка или список", v, sp),
    }
}
fn g_str(it: &mut Interp, a: &[Value], _: Span) -> R<Value> {
    Ok(Value::str(it.display(&a[0])))
}
fn g_num(_: &mut Interp, a: &[Value], sp: Span) -> R<Value> {
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
