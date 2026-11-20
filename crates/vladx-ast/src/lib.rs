//! Нейтральный AST VladX.
//!
//! Один смысл — один AST: код на `ru`, `it`, `scn` даёт одинаковое дерево
//! (кроме `span` и поля `lang` у корня). Формат сериализации — JSON по
//! `spec/ast.schema.json`, дискриминатор узла — поле `kind`.

use serde::{Deserialize, Serialize};
use std::rc::Rc;

pub mod diag;
pub use diag::{Diagnostic, Severity};

/// Версия схемы AST (`spec/ast.schema.json`).
pub const SCHEMA_VERSION: u32 = 1;

/// Позиция в исходнике: строка и колонка с 1, длина в символах.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Span {
    pub line: u32,
    pub col: u32,
    pub len: u32,
}

impl Span {
    pub fn new(line: u32, col: u32, len: u32) -> Self {
        Span { line, col, len }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgramKind {
    Program,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockKind {
    Block,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Program {
    pub kind: ProgramKind,
    pub schema_version: u32,
    pub lang: String,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub kind: BlockKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    pub body: Vec<Stmt>,
}

impl Block {
    pub fn new(span: Option<Span>, body: Vec<Stmt>) -> Self {
        Block { kind: BlockKind::Block, span, body }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Param {
    pub name: String,
    #[serde(rename = "typeHint", default, skip_serializing_if = "Option::is_none")]
    pub type_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Arg {
    pub value: Expr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub key: String,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum StringPart {
    Text { value: String },
    Hole { expr: Expr },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnOp {
    Neg,
    Not,
}

type Sp = Option<Span>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Stmt {
    Declare {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        name: String,
        mutable: bool,
        value: Expr,
    },
    Assign {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        target: Expr,
        value: Expr,
    },
    Say {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        value: Expr,
    },
    Ask {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        prompt: Expr,
        name: String,
    },
    If {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        test: Expr,
        then: Block,
        #[serde(rename = "else", default)]
        else_: Option<Block>,
    },
    While {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        test: Expr,
        body: Block,
    },
    Repeat {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        times: Expr,
        body: Block,
    },
    ForEach {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        item: String,
        iter: Expr,
        body: Block,
    },
    Break {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
    },
    Continue {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
    },
    FunctionDecl {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        name: String,
        params: Vec<Param>,
        body: Rc<Block>,
    },
    Return {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        #[serde(default)]
        value: Option<Expr>,
    },
    Try {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        body: Block,
        error_name: String,
        handler: Block,
    },
    Throw {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        value: Expr,
    },
    Import {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        module: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        names: Vec<String>,
    },
    ExprStmt {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        expr: Expr,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Expr {
    NumberLit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        value: f64,
    },
    StringLit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        value: String,
        #[serde(default)]
        parts: Option<Vec<StringPart>>,
    },
    BoolLit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        value: bool,
    },
    NullLit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
    },
    ListLit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        items: Vec<Expr>,
    },
    MapLit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        entries: Vec<Entry>,
    },
    Identifier {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        name: String,
    },
    Binary {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Unary {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        op: UnOp,
        operand: Box<Expr>,
    },
    Call {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        callee: Box<Expr>,
        args: Vec<Arg>,
    },
    Member {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        object: Box<Expr>,
        property: String,
    },
    SelfExpr {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
    },
    FunctionExpr {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        params: Vec<Param>,
        body: Rc<Block>,
    },
    ObjectExpr {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Sp,
        proto: Box<Expr>,
        entries: Vec<Entry>,
    },
}

impl Expr {
    pub fn span(&self) -> Option<Span> {
        match self {
            Expr::NumberLit { span, .. }
            | Expr::StringLit { span, .. }
            | Expr::BoolLit { span, .. }
            | Expr::NullLit { span }
            | Expr::ListLit { span, .. }
            | Expr::MapLit { span, .. }
            | Expr::Identifier { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Call { span, .. }
            | Expr::Member { span, .. }
            | Expr::SelfExpr { span }
            | Expr::FunctionExpr { span, .. }
            | Expr::ObjectExpr { span, .. } => *span,
        }
    }
}

impl Stmt {
    pub fn span(&self) -> Option<Span> {
        match self {
            Stmt::Declare { span, .. }
            | Stmt::Assign { span, .. }
            | Stmt::Say { span, .. }
            | Stmt::Ask { span, .. }
            | Stmt::If { span, .. }
            | Stmt::While { span, .. }
            | Stmt::Repeat { span, .. }
            | Stmt::ForEach { span, .. }
            | Stmt::Break { span }
            | Stmt::Continue { span }
            | Stmt::FunctionDecl { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::Try { span, .. }
            | Stmt::Throw { span, .. }
            | Stmt::Import { span, .. }
            | Stmt::ExprStmt { span, .. } => *span,
        }
    }
}

impl Program {
    pub fn new(lang: &str, body: Vec<Stmt>) -> Self {
        Program { kind: ProgramKind::Program, schema_version: SCHEMA_VERSION, lang: lang.to_string(), body }
    }

    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("AST всегда сериализуется")
    }
}

/// JSON-дерево без `span`, без `lang` у корня и без исходного текста `value`
/// у строк с интерполяцией (источник правды там — `parts`; в `value` остаются
/// слова языка вроде `{это.имя}`) — форма для сравнения эквивалентности трёх
/// языков («один смысл — один AST»).
pub fn neutral_json(p: &Program) -> serde_json::Value {
    let mut v = serde_json::to_value(p).expect("AST всегда сериализуется");
    strip_spans(&mut v);
    if let serde_json::Value::Object(m) = &mut v {
        m.remove("lang");
    }
    v
}

/// JSON-дерево без позиций `span` (остальное как есть).
pub fn json_without_spans(p: &Program) -> serde_json::Value {
    let mut v = serde_json::to_value(p).expect("AST всегда сериализуется");
    strip_spans_only(&mut v);
    v
}

fn strip_spans_only(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(m) => {
            m.remove("span");
            m.values_mut().for_each(strip_spans_only);
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(strip_spans_only),
        _ => {}
    }
}

fn strip_spans(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(m) => {
            m.remove("span");
            if m.get("kind").and_then(|k| k.as_str()) == Some("StringLit") && m.get("parts").is_some_and(|x| !x.is_null()) {
                m.remove("value");
            }
            for (_, x) in m.iter_mut() {
                strip_spans(x);
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(strip_spans),
        _ => {}
    }
}
