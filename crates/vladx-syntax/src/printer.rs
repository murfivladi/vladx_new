//! Печать нейтрального AST обратно в `.vlstd` на выбранном языке.
//!
//! Нужна трём вещам: `vladx fmt` (канонический вид, комментарии сохраняются),
//! переводу кода между ru/it/scn и показу того, как ИИ понял `.vlhmn`.
//! Слова берутся из словаря (каноническая, т.е. первая форма).

use crate::keywords::{norm, Dictionary, Lang};
use crate::lexer::Comment;
use std::collections::HashMap;
use vladx_ast::{Arg, BinOp, Block, Entry, Expr, Param, Program, Stmt, StringPart, UnOp};

/// Сведения об исходнике, которых нет в AST: комментарии и концы конструкций.
#[derive(Debug, Clone, Default)]
pub struct Trivia {
    pub comments: Vec<Comment>,
    pub stmt_ends: HashMap<(u32, u32), u32>,
    pub block_ends: HashMap<(u32, u32), u32>,
}

const INDENT: &str = "  ";
const MAX_INLINE: usize = 80;

struct Pr<'a> {
    dict: &'a Dictionary,
    lang: Lang,
    out: Vec<String>,
    trivia: Option<&'a Trivia>,
    next_comment: usize,
    /// Последняя строка исходника, уже «выведенная» (для пустых строк между блоками).
    last_src_line: u32,
}

type R<T> = Result<T, String>;

fn is_ident(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next().is_some_and(|c| c.is_alphabetic() || c == '_') && cs.all(|c| c.is_alphanumeric() || c == '_')
}

fn quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '{' => o.push_str("\\{"),
            '}' => o.push_str("\\}"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn num(n: f64) -> String {
    if n == 0.0 {
        "0".into()
    } else {
        format!("{n}")
    }
}

// Приоритеты (как в парсере): или < и < сравнение < сумма < произведение < унарное < постфикс.
const P_OR: u8 = 1;
const P_AND: u8 = 2;
const P_CMP: u8 = 3;
const P_ADD: u8 = 4;
const P_MUL: u8 = 5;
const P_UN: u8 = 6;
const P_POST: u8 = 7;

impl<'a> Pr<'a> {
    fn w(&self, id: &str) -> R<String> {
        self.dict
            .forms(self.lang, id)
            .into_iter()
            .next()
            .ok_or_else(|| format!("в языке {} нет слова для «{id}»", self.lang.code()))
    }

    fn sup(&self, id: &str, role: &str) -> R<String> {
        self.dict
            .support(self.lang, id, role)
            .ok_or_else(|| format!("в языке {} нет служебного слова «{id}.{role}»", self.lang.code()))
    }

    fn name(&self, n: &str) -> R<String> {
        if !is_ident(n) {
            return Err(format!("«{n}» не годится как имя"));
        }
        if self.dict.lookup(self.lang, &[norm(n)]).is_some() {
            return Err(format!("имя «{n}» совпадает с ключевым словом языка {}", self.lang.code()));
        }
        Ok(n.to_string())
    }

    fn push(&mut self, depth: usize, text: String) {
        for (i, l) in text.split('\n').enumerate() {
            if i == 0 {
                self.out.push(format!("{}{}", INDENT.repeat(depth), l));
            } else {
                self.out.push(l.to_string());
            }
        }
    }

    // ───────────── комментарии ─────────────

    fn comments_before(&mut self, line: u32, depth: usize) {
        let Some(t) = self.trivia else { return };
        while let Some(c) = t.comments.get(self.next_comment) {
            if c.line >= line {
                break;
            }
            if self.blank_needed(c.line) {
                self.out.push(String::new());
            }
            if c.own_line || self.out.is_empty() {
                self.push(depth, c.text.clone());
            } else if let Some(last) = self.out.last_mut() {
                last.push_str("  ");
                last.push_str(&c.text);
            }
            self.last_src_line = c.line;
            self.next_comment += 1;
        }
    }

    /// Комментарий в хвосте строки `line` — дописать к последней строке вывода.
    fn trailing_comment(&mut self, line: u32) {
        let Some(t) = self.trivia else { return };
        while let Some(c) = t.comments.get(self.next_comment) {
            if c.line != line || c.own_line {
                break;
            }
            if let Some(last) = self.out.last_mut() {
                last.push_str("  ");
                last.push_str(&c.text);
            }
            self.next_comment += 1;
        }
    }

    /// Нужна ли пустая строка перед элементом исходника на строке `line`.
    fn blank_needed(&self, line: u32) -> bool {
        self.last_src_line > 0
            && line > self.last_src_line + 1
            && self.out.last().is_some_and(|x| !x.trim().is_empty() && !x.trim_end().ends_with('{'))
    }

    fn rest_comments(&mut self) {
        self.comments_before(u32::MAX, 0);
    }

    // ───────────── инструкции ─────────────

    fn stmts(&mut self, body: &[Stmt], depth: usize) -> R<()> {
        for (i, s) in body.iter().enumerate() {
            let start = s.span().map(|sp| sp.line);
            if let Some(l) = start {
                self.comments_before(l, depth);
                // Пустая строка между инструкциями сохраняется (одна).
                if self.blank_needed(l) {
                    self.out.push(String::new());
                }
            } else if self.trivia.is_none() && i > 0 {
                // Без исходника: функции отделяются пустыми строками.
                let prev_fn = matches!(body[i - 1], Stmt::FunctionDecl { .. });
                if matches!(s, Stmt::FunctionDecl { .. }) || prev_fn {
                    self.out.push(String::new());
                }
            }
            self.stmt(s, depth)?;
            if let (Some(sp), Some(t)) = (s.span(), self.trivia) {
                let end = t.stmt_ends.get(&(sp.line, sp.col)).copied().unwrap_or(sp.line);
                self.trailing_comment(end);
                self.last_src_line = end;
            }
        }
        Ok(())
    }

    fn block(&mut self, b: &Block, depth: usize) -> R<String> {
        // Возвращает «{ ... }» как многострочный текст, первая строка — «{».
        let saved = std::mem::take(&mut self.out);
        if let Some(sp) = b.span {
            if self.trivia.is_some() {
                self.last_src_line = sp.line;
            }
        }
        self.stmts(&b.body, depth + 1)?;
        if let (Some(sp), Some(t)) = (b.span, self.trivia) {
            if let Some(close) = t.block_ends.get(&(sp.line, sp.col)).copied() {
                self.comments_before(close, depth + 1);
                self.last_src_line = close;
            }
        }
        let inner = std::mem::replace(&mut self.out, saved);
        let mut s = String::from("{");
        for l in inner {
            s.push('\n');
            s.push_str(&l);
        }
        s.push('\n');
        s.push_str(&INDENT.repeat(depth));
        s.push('}');
        Ok(s)
    }

    fn stmt(&mut self, s: &Stmt, d: usize) -> R<()> {
        let line = match s {
            Stmt::Declare { name, mutable, value, .. } => {
                let v = self.expr(value, 0, d)?;
                if *mutable {
                    format!("{} {} {} {}", self.w("var.mutable")?, self.name(name)?, self.w("var.assign")?, v)
                } else {
                    let be = self.dict.support(self.lang, "var.declare", "be").unwrap_or_else(|| "=".into());
                    format!("{} {} {} {}", self.w("var.declare")?, self.name(name)?, be, v)
                }
            }
            Stmt::Assign { target, value, .. } => {
                format!("{} {} {}", self.expr(target, P_POST, d)?, self.w("var.assign")?, self.expr(value, 0, d)?)
            }
            Stmt::Say { value, .. } => format!("{} {}", self.w("io.say")?, self.expr(value, 0, d)?),
            Stmt::Ask { prompt, name, .. } => format!(
                "{} {} {} {}",
                self.w("io.ask")?,
                self.expr(prompt, 0, d)?,
                self.sup("io.ask", "into")?,
                self.name(name)?
            ),
            Stmt::If { .. } => self.if_chain(s, d)?,
            Stmt::While { test, body, .. } => {
                format!("{} {} {}", self.w("ctrl.while")?, self.expr(test, 0, d)?, self.block(body, d)?)
            }
            Stmt::Repeat { times, body, .. } => format!(
                "{} {} {} {}",
                self.w("ctrl.repeat")?,
                self.expr(times, 0, d)?,
                self.sup("ctrl.repeat", "times")?,
                self.block(body, d)?
            ),
            Stmt::ForEach { item, iter, body, .. } => format!(
                "{} {} {} {} {}",
                self.w("ctrl.foreach")?,
                self.name(item)?,
                self.sup("ctrl.foreach", "iter")?,
                self.expr(iter, 0, d)?,
                self.block(body, d)?
            ),
            Stmt::Break { .. } => self.w("ctrl.break")?,
            Stmt::Continue { .. } => self.w("ctrl.continue")?,
            Stmt::FunctionDecl { name, params, body, .. } => format!(
                "{} {}({}) {}",
                self.w("fn.declare")?,
                self.name(name)?,
                self.params(params, d)?,
                self.block(body, d)?
            ),
            Stmt::Return { value, .. } => match value {
                Some(v) => format!("{} {}", self.w("fn.return")?, self.expr(v, 0, d)?),
                None => self.w("fn.return")?,
            },
            Stmt::Try { body, error_name, handler, .. } => format!(
                "{} {} {} {} {}",
                self.w("err.try")?,
                self.block(body, d)?,
                self.w("err.catch")?,
                self.name(error_name)?,
                self.block(handler, d)?
            ),
            Stmt::Throw { value, .. } => format!("{} {}", self.w("err.throw")?, self.expr(value, 0, d)?),
            Stmt::Import { module, path, names, .. } => match (module, path) {
                (Some(m), _) => format!("{} {}", self.w("mod.import")?, self.name(m)?),
                (None, Some(p)) => {
                    let ns: R<Vec<String>> = names.iter().map(|n| self.name(n)).collect();
                    format!(
                        "{} {} {} {} {}",
                        self.w("mod.import")?,
                        self.sup("mod.import", "from")?,
                        quote(p),
                        self.sup("mod.import", "take")?,
                        ns?.join(", ")
                    )
                }
                _ => return Err("импорт без модуля и без пути".into()),
            },
            Stmt::ExprStmt { expr, .. } => self.expr(expr, 0, d)?,
        };
        self.push(d, line);
        Ok(())
    }

    fn if_chain(&mut self, s: &Stmt, d: usize) -> R<String> {
        let Stmt::If { test, then, else_, .. } = s else { unreachable!() };
        let mut out = format!(
            "{} {} {} {}",
            self.w("ctrl.if")?,
            self.expr(test, 0, d)?,
            self.sup("ctrl.if", "then")?,
            self.block(then, d)?
        );
        if let Some(e) = else_ {
            out.push(' ');
            out.push_str(&self.w("ctrl.else")?);
            out.push(' ');
            // «иначе если» — если так было написано (у блока нет своей «{»)
            // или исходника нет вовсе; «иначе { если … }» сохраняется как есть.
            let written_with_braces = match (e.span, self.trivia) {
                (Some(sp), Some(t)) => t.block_ends.contains_key(&(sp.line, sp.col)),
                _ => false,
            };
            match e.body.as_slice() {
                [inner @ Stmt::If { .. }] if !written_with_braces => out.push_str(&self.if_chain(inner, d)?),
                _ => out.push_str(&self.block(e, d)?),
            }
        }
        Ok(out)
    }

    fn params(&mut self, ps: &[Param], d: usize) -> R<String> {
        let mut v = vec![];
        for p in ps {
            let mut s = self.name(&p.name)?;
            if let Some(t) = &p.type_hint {
                let word = self
                    .dict
                    .types
                    .iter()
                    .find(|x| &x.id == t)
                    .and_then(|x| x.forms[&self.lang].first().cloned())
                    .unwrap_or_else(|| t.clone());
                s.push_str(": ");
                s.push_str(&word);
            }
            if let Some(def) = &p.default {
                s.push_str(" = ");
                s.push_str(&self.expr(def, 0, d)?);
            }
            v.push(s);
        }
        Ok(v.join(", "))
    }

    // ───────────── выражения ─────────────

    fn wrap(s: String, own: u8, min: u8) -> String {
        if own < min {
            format!("({s})")
        } else {
            s
        }
    }

    fn hole(&self, e: &Expr) -> R<String> {
        match e {
            Expr::Identifier { name, .. } => self.name(name),
            Expr::SelfExpr { .. } => self.w("fn.self"),
            Expr::Member { object, property, .. } => {
                if !is_ident(property) {
                    return Err(format!("поле «{property}» нельзя записать в интерполяции"));
                }
                Ok(format!("{}.{}", self.hole(object)?, property))
            }
            _ => Err("в интерполяции строки допустимы только имя или путь по полям".into()),
        }
    }

    fn entries(&mut self, es: &[Entry], d: usize, force_multiline: bool) -> R<String> {
        if es.is_empty() {
            return Ok("{}".into());
        }
        let mut items = vec![];
        let mut multiline = force_multiline;
        for e in es {
            let key = if is_ident(&e.key) && !e.key.contains(char::is_whitespace) { e.key.clone() } else { quote(&e.key) };
            let v = self.expr(&e.value, 0, d + 1)?;
            multiline |= v.contains('\n') || matches!(e.value, Expr::FunctionExpr { .. });
            items.push(format!("{key}: {v}"));
        }
        let inline = format!("{{ {} }}", items.join(", "));
        if !multiline && inline.chars().count() + d * 2 <= MAX_INLINE {
            return Ok(inline);
        }
        let pad = INDENT.repeat(d + 1);
        Ok(format!("{{\n{pad}{}\n{}}}", items.join(&format!(",\n{pad}")), INDENT.repeat(d)))
    }

    fn args(&mut self, args: &[Arg], d: usize) -> R<String> {
        let mut v = vec![];
        for a in args {
            let val = self.expr(&a.value, 0, d)?;
            v.push(match &a.name {
                Some(n) => format!("{} = {}", self.name(n)?, val),
                None => val,
            });
        }
        Ok(v.join(", "))
    }

    fn expr(&mut self, e: &Expr, min: u8, d: usize) -> R<String> {
        Ok(match e {
            Expr::NumberLit { value, .. } => {
                if !value.is_finite() {
                    return Err("число должно быть конечным".into());
                }
                if *value < 0.0 {
                    Self::wrap(format!("-{}", num(-value)), P_UN, min)
                } else {
                    num(*value)
                }
            }
            Expr::StringLit { value, parts, .. } => match parts {
                None => quote(value),
                Some(ps) => {
                    let mut s = String::from("\"");
                    for p in ps {
                        match p {
                            StringPart::Text { value } => {
                                let q = quote(value);
                                s.push_str(&q[1..q.len() - 1]);
                            }
                            StringPart::Hole { expr } => {
                                s.push('{');
                                s.push_str(&self.hole(expr)?);
                                s.push('}');
                            }
                        }
                    }
                    s.push('"');
                    s
                }
            },
            Expr::BoolLit { value, .. } => self.w(if *value { "lit.true" } else { "lit.false" })?,
            Expr::NullLit { .. } => self.w("lit.null")?,
            Expr::ListLit { items, .. } => {
                let parts: R<Vec<String>> = items.iter().map(|i| self.expr(i, 0, d + 1)).collect();
                let parts = parts?;
                let inline = format!("[{}]", parts.join(", "));
                if inline.contains('\n') || inline.chars().count() + d * 2 > MAX_INLINE {
                    let pad = INDENT.repeat(d + 1);
                    format!("[\n{pad}{}\n{}]", parts.join(&format!(",\n{pad}")), INDENT.repeat(d))
                } else {
                    inline
                }
            }
            Expr::MapLit { entries, .. } => self.entries(entries, d, false)?,
            Expr::ObjectExpr { proto, entries, .. } => {
                let p = self.expr(proto, P_POST, d)?;
                let body = self.entries(entries, d, true)?;
                Self::wrap(format!("{} {} {}", self.w("obj.withProto")?, p, body), 0, min)
            }
            Expr::Identifier { name, .. } => self.name(name)?,
            Expr::SelfExpr { .. } => self.w("fn.self")?,
            Expr::FunctionExpr { params, body, .. } => {
                let s = format!("{}({}) {}", self.w("fn.declare")?, self.params(params, d)?, self.block(body, d)?);
                Self::wrap(s, 0, min)
            }
            Expr::Member { object, property, .. } => {
                if !is_ident(property) {
                    return Err(format!("поле «{property}» нельзя записать через точку"));
                }
                format!("{}.{}", self.expr(object, P_POST, d)?, property)
            }
            Expr::Call { callee, args, .. } => {
                format!("{}({})", self.expr(callee, P_POST, d)?, self.args(args, d)?)
            }
            Expr::Unary { op, operand, .. } => {
                let inner = self.expr(operand, P_UN, d)?;
                let s = match op {
                    UnOp::Neg => format!("-{inner}"),
                    UnOp::Not => format!("{} {inner}", self.w("logic.not")?),
                };
                Self::wrap(s, P_UN, min)
            }
            Expr::Binary { op, left, right, .. } => {
                let (own, word) = match op {
                    BinOp::Or => (P_OR, self.w("logic.or")?),
                    BinOp::And => (P_AND, self.w("logic.and")?),
                    BinOp::Gt => (P_CMP, self.w("cmp.gt")?),
                    BinOp::Lt => (P_CMP, self.w("cmp.lt")?),
                    BinOp::Ge => (P_CMP, self.w("cmp.ge")?),
                    BinOp::Le => (P_CMP, self.w("cmp.le")?),
                    BinOp::Eq => (P_CMP, self.w("cmp.eq")?),
                    BinOp::Ne => (P_CMP, self.w("cmp.ne")?),
                    BinOp::Add => (P_ADD, "+".into()),
                    BinOp::Sub => (P_ADD, "-".into()),
                    BinOp::Mul => (P_MUL, "*".into()),
                    BinOp::Div => (P_MUL, "/".into()),
                    BinOp::Mod => (P_MUL, "%".into()),
                };
                // Сравнение не цепляется: обе стороны — уровня суммы.
                let (lmin, rmin) = if own == P_CMP { (P_ADD, P_ADD) } else { (own, own + 1) };
                let l = self.expr(left, lmin, d)?;
                let r = self.expr(right, rmin, d)?;
                Self::wrap(format!("{l} {word} {r}"), own, min)
            }
        })
    }
}

/// Напечатать программу на языке `lang`. С `trivia` сохраняются комментарии
/// и пустые строки исходника.
pub fn print_program(p: &Program, dict: &Dictionary, lang: Lang, trivia: Option<&Trivia>) -> Result<String, String> {
    let mut pr = Pr { dict, lang, out: vec![], trivia, next_comment: 0, last_src_line: 0 };
    pr.stmts(&p.body, 0)?;
    pr.rest_comments();
    while pr.out.last().is_some_and(|l| l.trim().is_empty()) {
        pr.out.pop();
    }
    let mut s: String = pr.out.into_iter().map(|l| l.trim_end().to_string()).collect::<Vec<_>>().join("\n");
    s.push('\n');
    Ok(s)
}
