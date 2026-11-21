//! Статические проверки до исполнения: имена, неизменяемость, место
//! для «прерви/продолжи/верни/это». Ошибки собираются все сразу.

use crate::keywords::{Dictionary, Lang};
use std::collections::HashMap;
use vladx_ast::{Block, Diagnostic, Expr, Param, Program, Span, Stmt, StringPart};

struct Scope {
    /// Объявлено к текущему моменту: имя → изменяемое?
    declared: HashMap<String, bool>,
    /// Всё, что объявляется в этом блоке (для ссылок «вперёд» из тел функций).
    all: HashMap<String, bool>,
    fn_depth: u32,
}

struct Checker<'a> {
    dict: &'a Dictionary,
    lang: Lang,
    scopes: Vec<Scope>,
    fn_depth: u32,
    loop_depth: u32,
    diags: Vec<Diagnostic>,
    /// Позиция ближайшего узла с позицией (у дыр интерполяции своей нет).
    cur: Span,
}

/// Проверить программу. `globals` — имена, которые даёт среда исполнения
/// (модули stdlib и глобальные функции).
pub fn check(p: &Program, dict: &Dictionary, lang: Lang, globals: &[&str]) -> Vec<Diagnostic> {
    let mut c = Checker {
        dict,
        lang,
        scopes: vec![],
        fn_depth: 0,
        loop_depth: 0,
        diags: vec![],
        cur: Span::new(1, 1, 1),
    };
    let g: HashMap<String, bool> = globals.iter().map(|n| (n.to_string(), false)).collect();
    c.scopes.push(Scope { declared: g.clone(), all: g, fn_depth: 0 });
    c.block_body(&p.body, &[]);
    c.diags
}

fn collect(body: &[Stmt]) -> HashMap<String, bool> {
    let mut m = HashMap::new();
    for s in body {
        match s {
            Stmt::Declare { name, mutable, .. } => {
                m.insert(name.clone(), *mutable);
            }
            Stmt::FunctionDecl { name, .. } => {
                m.insert(name.clone(), false);
            }
            Stmt::Ask { name, .. } => {
                m.entry(name.clone()).or_insert(true);
            }
            Stmt::Import { module, names, .. } => {
                if let Some(mo) = module {
                    m.insert(mo.clone(), false);
                }
                for n in names {
                    m.insert(n.clone(), false);
                }
            }
            _ => {}
        }
    }
    m
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

impl<'a> Checker<'a> {
    fn w(&self, id: &str) -> String {
        self.dict.form(self.lang, id)
    }

    fn at(&mut self, s: Option<Span>) -> Span {
        if let Some(s) = s {
            self.cur = s;
        }
        self.cur
    }

    fn push(&mut self, body: &[Stmt], pre: &[(String, bool)]) {
        let mut all = collect(body);
        let mut declared: HashMap<String, bool> = pre.iter().cloned().collect();
        for (k, v) in pre {
            all.insert(k.clone(), *v);
        }
        // Функции поднимаются: их можно вызывать до объявления.
        for s in body {
            if let Stmt::FunctionDecl { name, .. } = s {
                declared.insert(name.clone(), false);
            }
        }
        self.scopes.push(Scope { declared, all, fn_depth: self.fn_depth });
    }

    fn block_body(&mut self, body: &[Stmt], pre: &[(String, bool)]) {
        self.push(body, pre);
        let mut fns_seen = std::collections::HashSet::new();
        for s in body {
            if let Stmt::FunctionDecl { name, span, .. } = s {
                if !fns_seen.insert(name.clone()) {
                    let sp = self.at(*span);
                    self.diags.push(Diagnostic::error("E-NAME-001", format!("Имя «{name}» уже объявлено в этом блоке"), sp));
                }
            }
            self.stmt(s);
        }
        self.scopes.pop();
    }

    fn block(&mut self, b: &Block, pre: &[(String, bool)]) {
        self.at(b.span);
        self.block_body(&b.body, pre);
    }

    fn resolve(&self, name: &str) -> Option<bool> {
        for sc in self.scopes.iter().rev() {
            if let Some(m) = sc.declared.get(name) {
                return Some(*m);
            }
            if sc.fn_depth < self.fn_depth {
                if let Some(m) = sc.all.get(name) {
                    return Some(*m);
                }
            }
        }
        None
    }

    fn visible_names(&self) -> Vec<String> {
        let mut v = vec![];
        for sc in &self.scopes {
            v.extend(sc.declared.keys().cloned());
        }
        v
    }

    fn undeclared(&mut self, name: &str, span: Span) {
        let mut d = Diagnostic::error("E-NAME-004", format!("Имя «{name}» не объявлено"), span);
        let mut best: Option<(usize, String)> = None;
        for cand in self.visible_names() {
            let dist = levenshtein(name, &cand);
            if dist <= 2 && dist < name.chars().count() && best.as_ref().is_none_or(|(b, _)| dist < *b) {
                best = Some((dist, cand));
            }
        }
        d = match best {
            Some((_, c)) => d.with_hint(format!("возможно, вы имели в виду «{c}»")),
            None => d.with_hint(format!(
                "объявите его раньше: {} {} {} значение",
                self.w("var.declare"),
                name,
                self.dict.support(self.lang, "var.declare", "be").unwrap_or_else(|| "=".into())
            )),
        };
        self.diags.push(d);
    }

    fn declare_here(&mut self, name: &str, mutable: bool, span: Span, allow_redeclare: bool) {
        let sc = self.scopes.last_mut().expect("есть скоуп");
        if let Some(prev_mut) = sc.declared.get(name) {
            if allow_redeclare {
                if !*prev_mut {
                    let let_w = self.w("var.declare");
                    self.diags.push(
                        Diagnostic::error("E-NAME-002", format!("«{name}» объявлено через «{let_w}» и не может меняться"), span)
                            .with_hint(format!("объявите через «{}», чтобы значение можно было менять", self.w("var.mutable"))),
                    );
                }
                return;
            }
            self.diags.push(
                Diagnostic::error("E-NAME-001", format!("Имя «{name}» уже объявлено в этом блоке"), span)
                    .with_hint(format!("чтобы изменить значение, напишите: {name} {} значение", self.w("var.assign"))),
            );
            return;
        }
        sc.declared.insert(name.to_string(), mutable);
    }

    fn function(&mut self, params: &[Param], body: &Block) {
        let saved_loop = self.loop_depth;
        self.loop_depth = 0;
        self.fn_depth += 1;
        // Параметры: свой скоуп, значения по умолчанию видят предыдущие параметры.
        self.scopes.push(Scope { declared: HashMap::new(), all: HashMap::new(), fn_depth: self.fn_depth });
        for p in params {
            if let Some(d) = &p.default {
                self.expr(d);
            }
            let sc = self.scopes.last_mut().unwrap();
            sc.declared.insert(p.name.clone(), true);
        }
        self.block(body, &[]);
        self.scopes.pop();
        self.fn_depth -= 1;
        self.loop_depth = saved_loop;
    }

    fn stmt(&mut self, s: &Stmt) {
        let sp = self.at(s.span());
        match s {
            Stmt::Declare { name, mutable, value, .. } => {
                self.expr(value);
                self.declare_here(name, *mutable, sp, false);
            }
            Stmt::Assign { target, value, .. } => {
                self.expr(value);
                match target {
                    Expr::Identifier { name, span } => {
                        let sp = self.at(*span);
                        match self.resolve(name) {
                            None => self.undeclared(name, sp),
                            Some(false) => {
                                let let_w = self.w("var.declare");
                                self.diags.push(
                                    Diagnostic::error("E-NAME-002", format!("«{name}» объявлено через «{let_w}» и не может меняться"), sp)
                                        .with_hint(format!(
                                            "объявите через «{}»: {} {} {} значение",
                                            self.w("var.mutable"),
                                            self.w("var.mutable"),
                                            name,
                                            self.w("var.assign")
                                        )),
                                );
                            }
                            Some(true) => {}
                        }
                    }
                    other => self.expr(other),
                }
            }
            Stmt::Say { value, .. } | Stmt::Throw { value, .. } => self.expr(value),
            Stmt::Ask { prompt, name, .. } => {
                self.expr(prompt);
                match self.resolve(name) {
                    Some(false) => self.declare_here(name, true, sp, true),
                    Some(true) => {}
                    None => self.declare_here(name, true, sp, true),
                }
            }
            Stmt::If { test, then, else_, .. } => {
                self.expr(test);
                self.block(then, &[]);
                if let Some(e) = else_ {
                    self.block(e, &[]);
                }
            }
            Stmt::While { test, body, .. } => {
                self.expr(test);
                self.loop_depth += 1;
                self.block(body, &[]);
                self.loop_depth -= 1;
            }
            Stmt::Repeat { times, body, .. } => {
                self.expr(times);
                self.loop_depth += 1;
                self.block(body, &[]);
                self.loop_depth -= 1;
            }
            Stmt::ForEach { item, iter, body, .. } => {
                self.expr(iter);
                self.loop_depth += 1;
                self.block(body, &[(item.clone(), false)]);
                self.loop_depth -= 1;
            }
            Stmt::Break { .. } => {
                if self.loop_depth == 0 {
                    let w = self.w("ctrl.break");
                    self.diags.push(Diagnostic::error("E-CTRL-001", format!("«{w}» можно использовать только внутри цикла"), sp));
                }
            }
            Stmt::Continue { .. } => {
                if self.loop_depth == 0 {
                    let w = self.w("ctrl.continue");
                    self.diags.push(Diagnostic::error("E-CTRL-002", format!("«{w}» можно использовать только внутри цикла"), sp));
                }
            }
            Stmt::FunctionDecl { params, body, .. } => self.function(params, body),
            Stmt::Return { value, .. } => {
                if self.fn_depth == 0 {
                    let w = self.w("fn.return");
                    self.diags.push(Diagnostic::error("E-CTRL-003", format!("«{w}» можно использовать только внутри функции"), sp));
                }
                if let Some(v) = value {
                    self.expr(v);
                }
            }
            Stmt::Try { body, error_name, handler, .. } => {
                self.block(body, &[]);
                self.block(handler, &[(error_name.clone(), false)]);
            }
            Stmt::Import { module, path, names, .. } => {
                if let Some(m) = module {
                    let known = self.dict.stdlib_module(m).is_some() || self.scopes.first().is_some_and(|g| g.declared.contains_key(m));
                    if !known {
                        let list: Vec<String> = self.dict.stdlib_modules().iter().map(|id| self.dict.stdlib_word(self.lang, id)).collect();
                        self.diags.push(
                            Diagnostic::error("E-MODULE-001", format!("Модуль «{m}» не найден"), sp)
                                .with_hint(format!("модули stdlib: {}", list.join(", "))),
                        );
                    } else if self.dict.stdlib_module(m).is_some() {
                        self.declare_here(m, false, sp, false);
                    }
                }
                if path.is_some() {
                    for n in names {
                        self.declare_here(n, false, sp, false);
                    }
                }
            }
            Stmt::ExprStmt { expr, .. } => self.expr(expr),
        }
    }

    fn expr(&mut self, e: &Expr) {
        let sp = self.at(e.span());
        match e {
            Expr::NumberLit { .. } | Expr::BoolLit { .. } | Expr::NullLit { .. } => {}
            Expr::StringLit { parts, .. } => {
                if let Some(ps) = parts {
                    for p in ps {
                        if let StringPart::Hole { expr } = p {
                            self.expr(expr);
                            self.cur = sp;
                        }
                    }
                }
            }
            Expr::ListLit { items, .. } => items.iter().for_each(|i| self.expr(i)),
            Expr::MapLit { entries, .. } => entries.iter().for_each(|en| self.expr(&en.value)),
            Expr::ObjectExpr { proto, entries, .. } => {
                self.expr(proto);
                entries.iter().for_each(|en| self.expr(&en.value));
            }
            Expr::Identifier { name, .. } => {
                if self.resolve(name).is_none() {
                    self.undeclared(name, sp);
                }
            }
            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Unary { operand, .. } => self.expr(operand),
            Expr::Call { callee, args, .. } => {
                self.expr(callee);
                for a in args {
                    self.expr(&a.value);
                }
            }
            Expr::Member { object, .. } => self.expr(object),
            Expr::SelfExpr { .. } => {
                if self.fn_depth == 0 {
                    let w = self.w("fn.self");
                    self.diags.push(Diagnostic::error("E-NAME-005", format!("«{w}» можно использовать только внутри метода объекта"), sp));
                }
            }
            Expr::FunctionExpr { params, body, .. } => self.function(params, body),
        }
    }
}
