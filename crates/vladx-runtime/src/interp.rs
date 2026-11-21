//! Интерпретатор дерева AST.

use crate::value::{type_name, AssignErr, Closure, Env, ErrorVal, Value};
use crate::{Host, Options, Perm};
use indexmap::IndexMap;
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use vladx_ast::{BinOp, Block, Diagnostic, Expr, Param, Span, Stmt, StringPart, UnOp};

/// Управление потоком: всё, что прерывает обычное исполнение.
pub enum Ctl {
    Break,
    Continue,
    Return(Value),
    Throw(Value, Span),
    Fail(Diagnostic),
}

pub type R<T> = Result<T, Ctl>;

pub fn fail<T>(code: &str, msg: impl Into<String>, sp: Span) -> R<T> {
    Err(Ctl::Fail(Diagnostic::error(code, msg, sp)))
}

pub fn fail_hint<T>(code: &str, msg: impl Into<String>, hint: impl Into<String>, sp: Span) -> R<T> {
    Err(Ctl::Fail(Diagnostic::error(code, msg, sp).with_hint(hint)))
}

/// Слова языка программы, нужные рантайму (печать значений, сообщения).
#[derive(Clone, Debug)]
pub struct Words {
    pub true_: String,
    pub false_: String,
    pub null: String,
    pub let_: String,
    pub mutable: String,
    pub assign: String,
}

pub struct Interp<'h> {
    pub(crate) host: &'h mut dyn Host,
    pub(crate) opts: Options,
    pub(crate) words: Words,
    steps: u64,
    depth: u32,
    pub(crate) stdout: Vec<String>,
    pub(crate) warnings: Vec<Diagnostic>,
    warned: HashSet<(String, u32, u32)>,
    pub(crate) globals: Rc<Env>,
    cur: Span,
    rng: u64,
    asked: HashSet<Perm>,
    closure_envs: Vec<Weak<Env>>,
}

const MAX_WARNINGS: usize = 100;

impl<'h> Interp<'h> {
    pub fn new(host: &'h mut dyn Host, opts: Options, words: Words) -> Self {
        let seed = opts.seed.unwrap_or_else(|| host.now_ms().to_bits() ^ 0x9E37_79B9_7F4A_7C15) | 1;
        let globals = Env::new(None, None);
        let mut me = Interp {
            host,
            opts,
            words,
            steps: 0,
            depth: 0,
            stdout: vec![],
            warnings: vec![],
            warned: HashSet::new(),
            globals,
            cur: Span::new(1, 1, 1),
            rng: seed,
            asked: HashSet::new(),
            closure_envs: vec![],
        };
        crate::stdlib::install(&mut me);
        me
    }

    /// Исполнить тело программы.
    pub fn run(&mut self, body: &[Stmt]) -> Result<(), Diagnostic> {
        let g = self.globals.clone();
        let r = self.exec_stmts(body, &g);
        match r {
            Ok(()) | Err(Ctl::Return(_)) | Err(Ctl::Break) | Err(Ctl::Continue) => Ok(()),
            Err(Ctl::Fail(d)) => Err(d),
            Err(Ctl::Throw(v, sp)) => {
                let shown = self.display(&v);
                Err(Diagnostic::error("E-UNCAUGHT", format!("необработанная ошибка: {shown}"), sp).with_hint(
                    "оберните код в попытку и поймайте ошибку, чтобы обработать её".to_string(),
                ))
            }
        }
    }

    /// Разорвать циклы ссылок (окружение ↔ замыкание), чтобы память освобождалась.
    pub fn cleanup(&mut self) {
        for w in self.closure_envs.drain(..) {
            if let Some(e) = w.upgrade() {
                e.vars.borrow_mut().clear();
            }
        }
        self.globals.vars.borrow_mut().clear();
    }

    // ───────────── служебное ─────────────

    pub(crate) fn sp(&mut self, s: Option<Span>) -> Span {
        if let Some(s) = s {
            self.cur = s;
        }
        self.cur
    }

    fn tick(&mut self, sp: Span) -> R<()> {
        self.steps += 1;
        if self.steps > self.opts.max_steps {
            return fail_hint(
                "E-LIMIT-001",
                format!("Превышен лимит шагов ({}). Похоже на бесконечный цикл", self.opts.max_steps),
                "проверьте, что условие цикла когда-нибудь становится ложным",
                sp,
            );
        }
        if self.steps.is_multiple_of(65_536) {
            if let Some(msg) = self.host.should_stop() {
                return fail("E-LIMIT-003", msg, sp);
            }
        }
        Ok(())
    }

    pub(crate) fn warn(&mut self, code: &str, msg: String, sp: Span) {
        if self.warnings.len() >= MAX_WARNINGS || !self.warned.insert((code.to_string(), sp.line, sp.col)) {
            return;
        }
        let d = Diagnostic::warning(code, msg, sp);
        self.host.warn(&format!("{}:{}  {}  {}", d.line, d.col, d.code, d.message));
        self.warnings.push(d);
    }

    pub(crate) fn out(&mut self, line: String) {
        self.host.say(&line);
        self.stdout.push(line);
    }

    pub(crate) fn random(&mut self) -> f64 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        let r = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (r >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Проверка разрешения. Если доступ сужен — один раз спросить человека.
    pub(crate) fn need(&mut self, kind: Perm, sp: Span) -> R<()> {
        let granted = match kind {
            Perm::Files => self.opts.files,
            Perm::Net => self.opts.net,
        };
        if granted {
            return Ok(());
        }
        if self.asked.insert(kind) && self.host.ask_permission(kind) {
            match kind {
                Perm::Files => self.opts.files = true,
                Perm::Net => self.opts.net = true,
            }
            return Ok(());
        }
        match kind {
            Perm::Files => fail_hint(
                "E-FILE-001",
                "доступ к файлам запрещён: нужно разрешение человека на доступ к файлам",
                "включите разрешение «файлы» (в CLI — уберите --no-files)",
                sp,
            ),
            Perm::Net => fail_hint(
                "E-NET-001",
                "сеть запрещена: нужно разрешение человека на доступ к сети",
                "включите разрешение «сеть» (в CLI — уберите --no-net)",
                sp,
            ),
        }
    }

    // ───────────── печать значений ─────────────

    pub fn fmt_num(n: f64) -> String {
        if n == 0.0 {
            "0".to_string()
        } else if n.is_nan() {
            "не число".to_string()
        } else if n.is_infinite() {
            if n > 0.0 { "бесконечность" } else { "-бесконечность" }.to_string()
        } else {
            format!("{n}")
        }
    }

    pub fn display(&self, v: &Value) -> String {
        match v {
            Value::Str(s) => s.to_string(),
            _ => self.repr(v, 0),
        }
    }

    pub fn repr(&self, v: &Value, depth: usize) -> String {
        if depth > 20 {
            return "…".into();
        }
        match v {
            Value::Null => self.words.null.clone(),
            Value::Bool(true) => self.words.true_.clone(),
            Value::Bool(false) => self.words.false_.clone(),
            Value::Num(n) => Self::fmt_num(*n),
            Value::Str(s) => {
                if depth == 0 {
                    s.to_string()
                } else {
                    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
                }
            }
            Value::List(l) => {
                let items: Vec<String> = l.borrow().iter().map(|x| self.repr(x, depth + 1)).collect();
                format!("[{}]", items.join(", "))
            }
            Value::Map(m) => {
                let m = m.borrow();
                let items: Vec<String> =
                    m.entries.iter().map(|(k, x)| format!("{}: {}", k, self.repr(x, depth + 1))).collect();
                format!("{{{}}}", items.join(", "))
            }
            Value::Func(c) => format!("<функция {}>", c.name.as_deref().unwrap_or("без имени")),
            Value::Native(n) => format!("<функция {}>", n.name),
            Value::Module(m) => format!("<модуль {}>", m.name),
            Value::Error(e) => format!("{} ({})", e.message, e.code),
        }
    }

    fn short(&self, v: &Value) -> String {
        let s = self.repr(v, 1);
        if s.chars().count() > 40 {
            format!("{}…", s.chars().take(40).collect::<String>())
        } else {
            s
        }
    }

    // ───────────── инструкции ─────────────

    pub(crate) fn exec_stmts(&mut self, body: &[Stmt], env: &Rc<Env>) -> R<()> {
        // Функции поднимаются: их можно вызывать до объявления.
        for s in body {
            if let Stmt::FunctionDecl { name, params, body: fb, .. } = s {
                let f = self.closure(Some(name.clone()), params, fb, env);
                env.define(name, f, false);
            }
        }
        for s in body {
            self.exec(s, env)?;
        }
        Ok(())
    }

    fn exec_block(&mut self, b: &Block, env: &Rc<Env>) -> R<()> {
        let inner = Env::new(Some(env.clone()), None);
        self.exec_stmts(&b.body, &inner)
    }

    fn closure(&mut self, name: Option<String>, params: &[Param], body: &Rc<Block>, env: &Rc<Env>) -> Value {
        if !self.closure_envs.last().is_some_and(|w| w.as_ptr() == Rc::as_ptr(env)) {
            self.closure_envs.push(Rc::downgrade(env));
        }
        Value::Func(Rc::new(Closure { name, params: params.to_vec(), body: body.clone(), env: env.clone() }))
    }

    fn exec(&mut self, s: &Stmt, env: &Rc<Env>) -> R<()> {
        let sp = self.sp(s.span());
        self.tick(sp)?;
        match s {
            Stmt::Declare { name, mutable, value, .. } => {
                let v = self.eval(value, env)?;
                if env.has_own(name) {
                    return fail("E-NAME-001", format!("Имя «{name}» уже объявлено в этом блоке"), sp);
                }
                env.define(name, v, *mutable);
            }
            Stmt::Assign { target, value, .. } => {
                let v = self.eval(value, env)?;
                match target {
                    Expr::Identifier { name, span } => {
                        let sp = self.sp(*span);
                        self.assign_var(env, name, v, sp)?;
                    }
                    Expr::Member { object, property, span } => {
                        let o = self.eval(object, env)?;
                        let sp = self.sp(*span);
                        self.set_member(&o, property, v, sp)?;
                    }
                    _ => return fail("E-TYPE-004", format!("Слева от «{}» должно быть имя или поле", self.words.assign), sp),
                }
            }
            Stmt::Say { value, .. } => {
                let v = self.eval(value, env)?;
                let line = self.display(&v);
                self.out(line);
            }
            Stmt::Ask { prompt, name, .. } => {
                let p = self.eval(prompt, env)?;
                let p = self.display(&p);
                let answer = match self.host.input(&p) {
                    Some(a) => a,
                    // Ответа ещё нет: остановиться и дать человеку ответить (см. Options::interactive).
                    None if self.opts.interactive => return fail(crate::ASK_WAIT, p, sp),
                    None => String::new(),
                };
                self.stdout.push(format!("{p} {answer}"));
                let v = Value::str(&answer);
                match env.mutability(name) {
                    Some(_) => self.assign_var(env, name, v, sp)?,
                    None => env.define(name, v, true),
                }
            }
            Stmt::If { test, then, else_, .. } => {
                if self.eval(test, env)?.truthy() {
                    self.exec_block(then, env)?;
                } else if let Some(e) = else_ {
                    self.exec_block(e, env)?;
                }
            }
            Stmt::While { test, body, .. } => loop {
                self.tick(sp)?;
                if !self.eval(test, env)?.truthy() {
                    break;
                }
                match self.exec_block(body, env) {
                    Ok(()) | Err(Ctl::Continue) => {}
                    Err(Ctl::Break) => break,
                    Err(e) => return Err(e),
                }
            },
            Stmt::Repeat { times, body, .. } => {
                let tv = self.eval(times, env)?;
                let n = match tv {
                    Value::Num(n) => n,
                    other => self.num_of(&other, sp)?,
                };
                if n < 0.0 {
                    return fail_hint("E-VALUE-001", "Число повторов не может быть отрицательным", "укажите 0 или больше", sp);
                }
                let n = n.floor() as u64;
                for _ in 0..n {
                    self.tick(sp)?;
                    match self.exec_block(body, env) {
                        Ok(()) | Err(Ctl::Continue) => {}
                        Err(Ctl::Break) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
            Stmt::ForEach { item, iter, body, .. } => {
                let coll = self.eval(iter, env)?;
                let items: Vec<Value> = match &coll {
                    Value::List(l) => l.borrow().clone(),
                    Value::Str(s) => s.chars().map(|c| Value::str(c.to_string())).collect(),
                    Value::Map(m) => m.borrow().entries.keys().map(Value::str).collect(),
                    other => {
                        return fail_hint(
                            "E-TYPE-006",
                            format!("По «{}» нельзя пройти циклом: нужен список, строка или словарь", self.short(other)),
                            format!("сейчас это {}", type_name(other.type_id())),
                            sp,
                        )
                    }
                };
                for it in items {
                    self.tick(sp)?;
                    let inner = Env::new(Some(env.clone()), None);
                    inner.define(item, it, false);
                    match self.exec_stmts(&body.body, &inner) {
                        Ok(()) | Err(Ctl::Continue) => {}
                        Err(Ctl::Break) => break,
                        Err(e) => return Err(e),
                    }
                }
            }
            Stmt::Break { .. } => return Err(Ctl::Break),
            Stmt::Continue { .. } => return Err(Ctl::Continue),
            Stmt::FunctionDecl { .. } => {} // уже поднята в exec_stmts
            Stmt::Return { value, .. } => {
                let v = match value {
                    Some(e) => self.eval(e, env)?,
                    None => Value::Null,
                };
                return Err(Ctl::Return(v));
            }
            Stmt::Try { body, error_name, handler, .. } => {
                let caught = match self.exec_block(body, env) {
                    Err(Ctl::Throw(v, _)) => v,
                    Err(Ctl::Fail(d)) if !d.code.starts_with("E-LIMIT") && d.code != crate::ASK_WAIT => {
                        Value::Error(Rc::new(ErrorVal { code: d.code, message: d.message }))
                    }
                    other => return other,
                };
                let inner = Env::new(Some(env.clone()), None);
                inner.define(error_name, caught, false);
                self.exec_stmts(&handler.body, &inner)?;
            }
            Stmt::Throw { value, .. } => {
                let v = self.eval(value, env)?;
                return Err(Ctl::Throw(v, sp));
            }
            Stmt::Import { module, path, .. } => {
                if let Some(m) = module {
                    if !matches!(self.globals.get(m), Some(Value::Module(_))) {
                        return fail_hint(
                            "E-MODULE-001",
                            format!("Модуль «{m}» не найден"),
                            "модули stdlib: numeri, testo, liste, mappe, caso, tempo, json, file, rete",
                            sp,
                        );
                    }
                }
                if let Some(p) = path {
                    return fail_hint(
                        "E-MODULE-001",
                        format!("Модуль «{p}» не найден: импорт файлов проекта пока не поддерживается"),
                        "в v0.1 доступны только модули стандартной библиотеки",
                        sp,
                    );
                }
            }
            Stmt::ExprStmt { expr, .. } => {
                self.eval(expr, env)?;
            }
        }
        Ok(())
    }

    fn assign_var(&mut self, env: &Rc<Env>, name: &str, v: Value, sp: Span) -> R<()> {
        match env.assign(name, v) {
            Ok(()) => Ok(()),
            Err(AssignErr::NotFound) => fail("E-NAME-004", format!("Имя «{name}» не объявлено"), sp),
            Err(AssignErr::Immutable) => fail_hint(
                "E-NAME-002",
                format!("«{name}» объявлено через «{}» и не может меняться", self.words.let_),
                format!("объявите через «{}», чтобы значение можно было менять", self.words.mutable),
                sp,
            ),
        }
    }

    // ───────────── выражения ─────────────

    pub(crate) fn eval(&mut self, e: &Expr, env: &Rc<Env>) -> R<Value> {
        let sp = self.sp(e.span());
        Ok(match e {
            Expr::NumberLit { value, .. } => Value::Num(*value),
            Expr::StringLit { value, parts, .. } => match parts {
                None => Value::str(value),
                Some(ps) => {
                    let mut s = String::new();
                    for p in ps {
                        match p {
                            StringPart::Text { value } => s.push_str(value),
                            StringPart::Hole { expr } => {
                                let v = self.eval(expr, env)?;
                                self.cur = sp;
                                s.push_str(&self.display(&v));
                            }
                        }
                    }
                    Value::str(s)
                }
            },
            Expr::BoolLit { value, .. } => Value::Bool(*value),
            Expr::NullLit { .. } => Value::Null,
            Expr::ListLit { items, .. } => {
                let mut v = Vec::with_capacity(items.len());
                for i in items {
                    v.push(self.eval(i, env)?);
                }
                Value::list(v)
            }
            Expr::MapLit { entries, .. } => {
                let mut m = IndexMap::new();
                for en in entries {
                    let v = self.eval(&en.value, env)?;
                    m.insert(en.key.clone(), v);
                }
                Value::map(m, None)
            }
            Expr::ObjectExpr { proto, entries, .. } => {
                let p = self.eval(proto, env)?;
                if !matches!(p, Value::Map(_)) {
                    return fail(
                        "E-TYPE-001",
                        format!("Прототипом может быть только объект, а получено «{}»", self.short(&p)),
                        sp,
                    );
                }
                let mut m = IndexMap::new();
                for en in entries {
                    let v = self.eval(&en.value, env)?;
                    m.insert(en.key.clone(), v);
                }
                Value::map(m, Some(p))
            }
            Expr::Identifier { name, .. } => match env.get(name) {
                Some(v) => v,
                None => return fail("E-NAME-004", format!("Имя «{name}» не объявлено"), sp),
            },
            Expr::SelfExpr { .. } => match env.this() {
                Some(v) => v,
                None => return fail("E-NAME-005", "«это» можно использовать только внутри метода объекта", sp),
            },
            Expr::FunctionExpr { params, body, .. } => self.closure(None, params, body, env),
            Expr::Unary { op, operand, .. } => {
                let v = self.eval(operand, env)?;
                match op {
                    UnOp::Not => Value::Bool(!v.truthy()),
                    UnOp::Neg => Value::Num(-self.num_of(&v, sp)?),
                }
            }
            Expr::Binary { op: BinOp::And, left, right, .. } => {
                let l = self.eval(left, env)?;
                if !l.truthy() {
                    Value::Bool(false)
                } else {
                    Value::Bool(self.eval(right, env)?.truthy())
                }
            }
            Expr::Binary { op: BinOp::Or, left, right, .. } => {
                let l = self.eval(left, env)?;
                if l.truthy() {
                    Value::Bool(true)
                } else {
                    Value::Bool(self.eval(right, env)?.truthy())
                }
            }
            Expr::Binary { op, left, right, .. } => {
                let l = self.eval(left, env)?;
                let r = self.eval(right, env)?;
                self.sp(Some(sp));
                self.binary(*op, l, r, sp)?
            }
            Expr::Member { object, property, .. } => {
                let o = self.eval(object, env)?;
                self.get_member(&o, property, sp)?
            }
            Expr::Call { callee, args, .. } => {
                let (f, this, what) = match &**callee {
                    Expr::Member { object, property, .. } => {
                        let o = self.eval(object, env)?;
                        let f = self.get_member_for_call(&o, property, sp)?;
                        let this = if matches!(o, Value::Map(_)) { Some(o) } else { None };
                        (f, this, format!("{}.{}", expr_name(object), property))
                    }
                    other => (self.eval(other, env)?, None, expr_name(other)),
                };
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    vals.push((a.name.clone(), self.eval(&a.value, env)?));
                }
                self.cur = sp;
                self.call(f, vals, this, sp, &what)?
            }
        })
    }

    pub(crate) fn call(&mut self, f: Value, args: Vec<(Option<String>, Value)>, this: Option<Value>, sp: Span, what: &str) -> R<Value> {
        match f {
            Value::Func(c) => {
                self.tick(sp)?;
                if self.depth >= self.opts.max_depth {
                    return fail_hint(
                        "E-LIMIT-002",
                        format!("Слишком глубокая рекурсия (предел {})", self.opts.max_depth),
                        "проверьте условие выхода из рекурсии",
                        sp,
                    );
                }
                self.depth += 1;
                let r = self.call_closure(&c, args, this, sp);
                self.depth -= 1;
                match r {
                    Ok(()) | Err(Ctl::Break) | Err(Ctl::Continue) => Ok(Value::Null),
                    Err(Ctl::Return(v)) => Ok(v),
                    Err(e) => Err(e),
                }
            }
            Value::Native(n) => {
                if args.iter().any(|(name, _)| name.is_some()) {
                    return fail("E-ARGS-003", "Встроенные функции не принимают именованные аргументы", sp);
                }
                let vals: Vec<Value> = args.into_iter().map(|(_, v)| v).collect();
                if vals.len() > n.max {
                    return fail(
                        "E-ARGS-001",
                        format!("Лишние аргументы: ожидалось {}, передано {} ({})", n.max, vals.len(), n.name),
                        sp,
                    );
                }
                if vals.len() < n.min {
                    return fail(
                        "E-ARGS-002",
                        format!("Не хватает аргумента: «{}» ждёт {}, передано {}", n.name, n.min, vals.len()),
                        sp,
                    );
                }
                (n.f)(self, &vals, sp)
            }
            other => fail(
                "E-TYPE-003",
                format!("Нельзя вызвать «{what}»: это не функция, а {}", type_name(other.type_id())),
                sp,
            ),
        }
    }

    fn call_closure(&mut self, c: &Rc<Closure>, args: Vec<(Option<String>, Value)>, this: Option<Value>, sp: Span) -> R<()> {
        let env = Env::new(Some(c.env.clone()), this);
        let mut positional = vec![];
        let mut named: Vec<(String, Value)> = vec![];
        for (n, v) in args {
            match n {
                Some(n) => named.push((n, v)),
                None => positional.push(v),
            }
        }
        let fname = c.name.clone().unwrap_or_else(|| "функция".into());
        if positional.len() > c.params.len() {
            return fail(
                "E-ARGS-001",
                format!("Лишние аргументы: ожидалось {}, передано {} («{fname}»)", c.params.len(), positional.len()),
                sp,
            );
        }
        for (n, _) in &named {
            if !c.params.iter().any(|p| &p.name == n) {
                return fail("E-ARGS-001", format!("Лишние аргументы: у «{fname}» нет параметра «{n}»"), sp);
            }
        }
        let mut positional = positional.into_iter();
        for p in &c.params {
            let from_pos = positional.next();
            let from_named = named.iter().position(|(n, _)| n == &p.name).map(|i| named.remove(i).1);
            let v = match (from_pos, from_named) {
                (Some(_), Some(_)) => {
                    return fail("E-ARGS-001", format!("Лишние аргументы: «{}» передан дважды", p.name), sp);
                }
                (Some(v), None) | (None, Some(v)) => v,
                (None, None) => match &p.default {
                    Some(d) => self.eval(d, &env)?,
                    None => {
                        return fail_hint(
                            "E-ARGS-002",
                            format!("Не хватает аргумента «{}»", p.name),
                            format!("передайте его: {fname}(…, {} = значение)", p.name),
                            sp,
                        )
                    }
                },
            };
            if let Some(t) = &p.type_hint {
                if v.type_id() != t && t != "any" {
                    return fail(
                        "E-TYPE-002",
                        format!(
                            "Аргумент «{}» ожидает тип «{}», получено «{}»",
                            p.name,
                            type_name(t),
                            type_name(v.type_id())
                        ),
                        sp,
                    );
                }
            }
            env.define(&p.name, v, true);
        }
        self.exec_stmts(&c.body.body, &env)
    }

    fn lookup_map(&self, v: &Value, prop: &str) -> Option<Value> {
        let mut cur = v.clone();
        for _ in 0..100 {
            let next = match &cur {
                Value::Map(m) => {
                    let m = m.borrow();
                    if let Some(x) = m.entries.get(prop) {
                        return Some(x.clone());
                    }
                    m.proto.clone()
                }
                _ => None,
            };
            cur = next?;
        }
        None
    }

    pub(crate) fn get_member(&mut self, o: &Value, prop: &str, sp: Span) -> R<Value> {
        match o {
            Value::Null => fail(
                "E-NULL-001",
                format!("Обращение к полю «{prop}» у «{}»", self.words.null),
                sp,
            ),
            Value::Map(_) => Ok(self.lookup_map(o, prop).unwrap_or(Value::Null)),
            Value::Module(m) => match m.fns.get(prop) {
                Some(f) => Ok(f.clone()),
                None => fail_hint(
                    "E-MODULE-002",
                    format!("В модуле «{}» нет функции «{prop}»", m.name),
                    format!("есть: {}", m.fns.keys().cloned().collect::<Vec<_>>().join(", ")),
                    sp,
                ),
            },
            Value::Error(e) => Ok(match prop {
                "код" => Value::str(&e.code),
                "сообщение" => Value::str(&e.message),
                _ => Value::Null,
            }),
            other => fail(
                "E-PROP-001",
                format!("У значения «{}» нет метода «{prop}»", self.short(other)),
                sp,
            ),
        }
    }

    fn get_member_for_call(&mut self, o: &Value, prop: &str, sp: Span) -> R<Value> {
        if let Value::Map(_) = o {
            return match self.lookup_map(o, prop) {
                Some(v) => Ok(v),
                None => fail("E-PROP-001", format!("У значения «{}» нет метода «{prop}»", self.short(o)), sp),
            };
        }
        self.get_member(o, prop, sp)
    }

    fn set_member(&mut self, o: &Value, prop: &str, v: Value, sp: Span) -> R<()> {
        match o {
            Value::Map(m) => {
                m.borrow_mut().entries.insert(prop.to_string(), v);
                Ok(())
            }
            Value::Null => fail("E-NULL-001", format!("Обращение к полю «{prop}» у «{}»", self.words.null), sp),
            other => fail(
                "E-PROP-001",
                format!("У значения «{}» нет поля «{prop}», которое можно изменить", self.short(other)),
                sp,
            ),
        }
    }

    // ───────────── арифметика и сравнения ─────────────

    /// Число для арифметики с правилами приведения §4.3.
    pub(crate) fn num_of(&mut self, v: &Value, sp: Span) -> R<f64> {
        match v {
            Value::Num(n) => Ok(*n),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Null => {
                self.warn("W-COERCE-003", format!("«{}» в арифметике считается нулём", self.words.null), sp);
                Ok(0.0)
            }
            Value::Str(s) => match s.trim().parse::<f64>() {
                Ok(n) if !s.trim().is_empty() => {
                    self.warn("W-COERCE-002", format!("Строка «{s}» приведена к числу {}", Self::fmt_num(n)), sp);
                    Ok(n)
                }
                _ => fail("E-TYPE-005", format!("Строку «{s}» нельзя превратить в число"), sp),
            },
            other => fail(
                "E-TYPE-005",
                format!("Арифметика невозможна со значением типа «{}»", type_name(other.type_id())),
                sp,
            ),
        }
    }

    fn binary(&mut self, op: BinOp, l: Value, r: Value, sp: Span) -> R<Value> {
        use BinOp::*;
        match op {
            Add => match (&l, &r) {
                (Value::Num(a), Value::Num(b)) => Ok(Value::Num(a + b)),
                (Value::Str(a), Value::Str(b)) => Ok(Value::str(format!("{a}{b}"))),
                (Value::Str(_), other) | (other, Value::Str(_)) => {
                    self.warn(
                        "W-COERCE-001",
                        format!("{} + строка: значение приведено к строке", type_name(other.type_id())),
                        sp,
                    );
                    Ok(Value::str(format!("{}{}", self.display(&l), self.display(&r))))
                }
                (Value::List(a), Value::List(b)) => {
                    let mut v = a.borrow().clone();
                    v.extend(b.borrow().iter().cloned());
                    Ok(Value::list(v))
                }
                _ => {
                    let a = self.num_of(&l, sp)?;
                    let b = self.num_of(&r, sp)?;
                    Ok(Value::Num(a + b))
                }
            },
            Sub | Mul | Div | Mod => {
                let a = self.num_of(&l, sp)?;
                let b = self.num_of(&r, sp)?;
                Ok(Value::Num(match op {
                    Sub => a - b,
                    Mul => a * b,
                    Div => {
                        if b == 0.0 {
                            return fail("E-MATH-001", "Деление на ноль", sp);
                        }
                        a / b
                    }
                    _ => {
                        if b == 0.0 {
                            return fail("E-MATH-002", "Остаток от деления на ноль", sp);
                        }
                        a % b
                    }
                }))
            }
            Eq => Ok(Value::Bool(self.equals(&l, &r, sp))),
            Ne => Ok(Value::Bool(!self.equals(&l, &r, sp))),
            Gt | Lt | Ge | Le => {
                let ord = self.compare(&l, &r, sp)?;
                use std::cmp::Ordering::*;
                Ok(Value::Bool(match (op, ord) {
                    (_, None) => false,
                    (Gt, Some(o)) => o == Greater,
                    (Lt, Some(o)) => o == Less,
                    (Ge, Some(o)) => o != Less,
                    (_, Some(o)) => o != Greater,
                }))
            }
            And | Or => unreachable!("обрабатываются с коротким замыканием"),
        }
    }

    fn str_num_eq(&mut self, s: &str, n: f64, sp: Span) -> bool {
        self.warn("W-COERCE-004", format!("Сравнение строки «{s}» с числом: приведено к числу"), sp);
        matches!(s.trim().parse::<f64>(), Ok(x) if x == n && !s.trim().is_empty())
    }

    pub(crate) fn equals(&mut self, a: &Value, b: &Value, sp: Span) -> bool {
        match (a, b) {
            (Value::Num(n), Value::Str(s)) | (Value::Str(s), Value::Num(n)) => {
                let s = s.clone();
                self.str_num_eq(&s, *n, sp)
            }
            _ => deep_eq(a, b),
        }
    }

    fn compare(&mut self, a: &Value, b: &Value, sp: Span) -> R<Option<std::cmp::Ordering>> {
        match (a, b) {
            (Value::Num(x), Value::Num(y)) => Ok(x.partial_cmp(y)),
            (Value::Str(x), Value::Str(y)) => Ok(Some(x.cmp(y))),
            (Value::Num(x), Value::Str(s)) | (Value::Str(s), Value::Num(x)) => {
                self.warn("W-COERCE-004", format!("Сравнение строки «{s}» с числом: приведено к числу"), sp);
                let y: f64 = match s.trim().parse() {
                    Ok(y) => y,
                    Err(_) => return fail("E-TYPE-005", format!("Строку «{s}» нельзя сравнить с числом"), sp),
                };
                if matches!(a, Value::Num(_)) {
                    Ok(x.partial_cmp(&y))
                } else {
                    Ok(y.partial_cmp(x))
                }
            }
            _ => fail(
                "E-TYPE-005",
                format!(
                    "Нельзя сравнить «{}» и «{}»",
                    type_name(a.type_id()),
                    type_name(b.type_id())
                ),
                sp,
            ),
        }
    }
}

pub fn deep_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Num(x), Value::Num(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::List(x), Value::List(y)) => {
            Rc::ptr_eq(x, y) || {
                let (x, y) = (x.borrow(), y.borrow());
                x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| deep_eq(p, q))
            }
        }
        (Value::Map(x), Value::Map(y)) => {
            Rc::ptr_eq(x, y) || {
                let (x, y) = (x.borrow(), y.borrow());
                x.entries.len() == y.entries.len()
                    && x.entries.iter().all(|(k, v)| y.entries.get(k).is_some_and(|w| deep_eq(v, w)))
            }
        }
        (Value::Func(x), Value::Func(y)) => Rc::ptr_eq(x, y),
        (Value::Native(x), Value::Native(y)) => Rc::ptr_eq(x, y),
        (Value::Module(x), Value::Module(y)) => Rc::ptr_eq(x, y),
        (Value::Error(x), Value::Error(y)) => x.code == y.code && x.message == y.message,
        _ => false,
    }
}

fn expr_name(e: &Expr) -> String {
    match e {
        Expr::Identifier { name, .. } => name.clone(),
        Expr::Member { object, property, .. } => format!("{}.{}", expr_name(object), property),
        Expr::SelfExpr { .. } => "это".into(),
        Expr::Call { callee, .. } => format!("{}(…)", expr_name(callee)),
        _ => "значение".into(),
    }
}
