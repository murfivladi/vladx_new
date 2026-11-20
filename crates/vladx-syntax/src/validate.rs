//! Проверка AST, пришедшего не из парсера (например, от ИИ для `.vlhmn`).
//!
//! Схема проверяется типами serde; затем AST печатается в `.vlstd` и
//! разбирается обратно — результат обязан совпасть с исходным деревом.
//! Так гарантируется, что программа выразима обычным текстом языка и
//! человек видит ровно то, что будет исполнено. В конце — статические проверки.

use crate::keywords::{Dictionary, Lang};
use crate::printer::print_program;
use vladx_ast::{neutral_json, Block, Diagnostic, Expr, Program, Span, Stmt, StringPart, UnOp, SCHEMA_VERSION};

fn d(code: &str, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::error(code, msg, Span::new(0, 0, 0))
}

/// Проверенная программа: дерево (позиции — по `text`) и его текст `.vlstd`.
#[derive(Debug, Clone)]
pub struct Checked {
    pub program: Program,
    pub text: String,
    pub lang: Lang,
}

/// Из JSON: схема → печать → разбор → сравнение → статические проверки.
pub fn validate_json(json: &str, dict: &Dictionary, globals: &[&str]) -> Result<Checked, Vec<Diagnostic>> {
    let p: Program = serde_json::from_str(json)
        .map_err(|e| vec![d("E-AST-001", format!("AST не соответствует схеме: {e}"))])?;
    validate(p, dict, globals)
}

pub fn validate(mut p: Program, dict: &Dictionary, globals: &[&str]) -> Result<Checked, Vec<Diagnostic>> {
    if p.schema_version != SCHEMA_VERSION {
        return Err(vec![d(
            "E-AST-001",
            format!("schema_version = {}, ожидалась {SCHEMA_VERSION}", p.schema_version),
        )]);
    }
    let lang = Lang::from_code(&p.lang)
        .ok_or_else(|| vec![d("E-AST-001", format!("неизвестный язык «{}»: ru, it или scn", p.lang))])?;
    normalize_body(&mut p.body, dict, lang);
    let text = print_program(&p, dict, lang, None).map_err(|e| vec![d("E-AST-002", format!("AST нельзя записать текстом: {e}"))])?;
    let back = crate::parse(&text, lang, dict).map_err(|e| {
        vec![d("E-AST-003", format!("напечатанный текст не разбирается: {} ({}:{})", e.message, e.line, e.col))]
    })?;
    if comparable(&p) != comparable(&back) {
        return Err(vec![d("E-AST-003", "AST неоднозначен: после печати и разбора дерево изменилось")]);
    }
    let errors: Vec<Diagnostic> =
        crate::semantic::check(&back, dict, lang, globals).into_iter().filter(|x| x.is_error()).collect();
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Checked { program: back, text, lang })
}

fn comparable(p: &Program) -> serde_json::Value {
    neutral_json(p)
}

// ───── нормализация того, что текст записывает одним способом ─────

fn normalize_body(b: &mut [Stmt], dict: &Dictionary, lang: Lang) {
    for s in b {
        normalize_stmt(s, dict, lang);
    }
}

fn nb(b: &mut Block, dict: &Dictionary, lang: Lang) {
    normalize_body(&mut b.body, dict, lang);
}

fn normalize_params(ps: &mut [vladx_ast::Param], dict: &Dictionary, lang: Lang) {
    for p in ps {
        if let Some(t) = &p.type_hint {
            if let Some(id) = dict.type_id(lang, t) {
                p.type_hint = Some(id);
            }
        }
        if let Some(e) = &mut p.default {
            ne(e, dict, lang);
        }
    }
}

fn normalize_stmt(s: &mut Stmt, dict: &Dictionary, lang: Lang) {
    match s {
        Stmt::Declare { value, .. } | Stmt::Say { value, .. } | Stmt::Throw { value, .. } => ne(value, dict, lang),
        Stmt::Assign { target, value, .. } => {
            ne(target, dict, lang);
            ne(value, dict, lang);
        }
        Stmt::Ask { prompt, .. } => ne(prompt, dict, lang),
        Stmt::If { test, then, else_, .. } => {
            ne(test, dict, lang);
            nb(then, dict, lang);
            if let Some(e) = else_ {
                nb(e, dict, lang);
            }
        }
        Stmt::While { test, body, .. } => {
            ne(test, dict, lang);
            nb(body, dict, lang);
        }
        Stmt::Repeat { times, body, .. } => {
            ne(times, dict, lang);
            nb(body, dict, lang);
        }
        Stmt::ForEach { iter, body, .. } => {
            ne(iter, dict, lang);
            nb(body, dict, lang);
        }
        Stmt::FunctionDecl { params, body, .. } => {
            normalize_params(params, dict, lang);
            nb(std::rc::Rc::make_mut(body), dict, lang);
        }
        Stmt::Return { value: Some(v), .. } => ne(v, dict, lang),
        Stmt::Try { body, handler, .. } => {
            nb(body, dict, lang);
            nb(handler, dict, lang);
        }
        Stmt::ExprStmt { expr, .. } => ne(expr, dict, lang),
        _ => {}
    }
}

fn ne(e: &mut Expr, dict: &Dictionary, lang: Lang) {
    match e {
        Expr::NumberLit { value, span } if *value < 0.0 => {
            let pos = Expr::NumberLit { value: -*value, span: *span };
            *e = Expr::Unary { span: None, op: UnOp::Neg, operand: Box::new(pos) };
        }
        Expr::StringLit { parts: Some(ps), .. } => {
            // Пустые куски и соседние тексты склеиваются так же, как их склеит лексер.
            let mut out: Vec<StringPart> = vec![];
            for p in ps.drain(..) {
                match p {
                    StringPart::Text { value } if value.is_empty() => {}
                    StringPart::Text { value } => match out.last_mut() {
                        Some(StringPart::Text { value: prev }) => prev.push_str(&value),
                        _ => out.push(StringPart::Text { value }),
                    },
                    hole => out.push(hole),
                }
            }
            let has_hole = out.iter().any(|p| matches!(p, StringPart::Hole { .. }));
            if has_hole {
                *ps = out;
            } else {
                let text: String = out
                    .into_iter()
                    .map(|p| if let StringPart::Text { value } = p { value } else { String::new() })
                    .collect();
                *e = Expr::StringLit { span: None, value: text, parts: None };
            }
        }
        Expr::ListLit { items, .. } => items.iter_mut().for_each(|i| ne(i, dict, lang)),
        Expr::MapLit { entries, .. } => entries.iter_mut().for_each(|x| ne(&mut x.value, dict, lang)),
        Expr::ObjectExpr { proto, entries, .. } => {
            ne(proto, dict, lang);
            entries.iter_mut().for_each(|x| ne(&mut x.value, dict, lang));
        }
        Expr::Binary { left, right, .. } => {
            ne(left, dict, lang);
            ne(right, dict, lang);
        }
        Expr::Unary { operand, .. } => ne(operand, dict, lang),
        Expr::Call { callee, args, .. } => {
            ne(callee, dict, lang);
            args.iter_mut().for_each(|a| ne(&mut a.value, dict, lang));
        }
        Expr::Member { object, .. } => ne(object, dict, lang),
        Expr::FunctionExpr { params, body, .. } => {
            normalize_params(params, dict, lang);
            nb(std::rc::Rc::make_mut(body), dict, lang);
        }
        _ => {}
    }
}
