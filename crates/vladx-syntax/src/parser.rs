//! Парсер рекурсивного спуска: токены → нейтральный AST.
//!
//! Здесь нет ни одной словоформы: ключевые слова распознаны лексером по
//! словарю, парсер видит только нейтральные id (`ctrl.if`, `var.declare`, ...).
//! Слова в сообщениях и подсказках берутся из словаря — на языке программы.
//! Порядок слов фиксирован: парсер не «догадывается», а объясняет канон.

use crate::keywords::{Dictionary, Lang};
use crate::lexer::{RawPart, Tk, Token};
use std::rc::Rc;
use std::collections::HashMap;
use vladx_ast::{Arg, BinOp, Block, Diagnostic, Entry, Expr, Param, Program, Span, Stmt, StringPart, UnOp};

type PR<T> = Result<T, Diagnostic>;

/// Позиция начала конструкции → строка её конца.
pub type Ends = HashMap<(u32, u32), u32>;

pub struct Parser<'a> {
    toks: Vec<Token>,
    pos: usize,
    dict: &'a Dictionary,
    lang: Lang,
    /// Строка последнего прочитанного токена.
    last_line: u32,
    /// Конец последнего прочитанного токена (строка, колонка после него).
    last_end: (u32, u32),
    /// Для форматтера: начало инструкции → строка её конца.
    pub stmt_ends: Ends,
    /// Для форматтера: «{» блока → строка «}».
    pub block_ends: Ends,
}

fn join(a: Option<Span>, b: Span) -> Option<Span> {
    let a = a?;
    if a.line == b.line && b.col + b.len >= a.col {
        Some(Span::new(a.line, a.col, b.col + b.len - a.col))
    } else {
        Some(a)
    }
}

impl<'a> Parser<'a> {
    pub fn new(toks: Vec<Token>, dict: &'a Dictionary, lang: Lang) -> Self {
        Parser {
            toks,
            pos: 0,
            dict,
            lang,
            last_line: 1,
            last_end: (1, 1),
            stmt_ends: HashMap::new(),
            block_ends: HashMap::new(),
        }
    }

    // ───────────── служебное ─────────────

    fn peek(&self) -> &Token {
        &self.toks[self.pos]
    }
    fn peek_at(&self, k: usize) -> &Token {
        let i = (self.pos + k).min(self.toks.len() - 1);
        &self.toks[i]
    }
    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if !t.is_eof() {
            self.pos += 1;
            self.last_line = t.line;
            self.last_end = (t.line, t.col + t.len);
        }
        t
    }
    fn at_sym(&self, s: &str) -> bool {
        self.peek().is_sym(s)
    }
    /// Следующий токен на той же строке, что и предыдущий.
    fn same_line(&self) -> bool {
        !self.peek().is_eof() && self.peek().line == self.last_line
    }
    /// Место сразу после последнего токена — для «здесь ожидалось ...».
    fn after_last(&self) -> Span {
        Span::new(self.last_end.0, self.last_end.1, 1)
    }
    fn w(&self, id: &str) -> String {
        self.dict.form(self.lang, id)
    }
    fn sup(&self, id: &str, role: &str) -> String {
        self.dict.support(self.lang, id, role).unwrap_or_else(|| "=".to_string())
    }
    fn err_at_peek_or_after(&self) -> Span {
        if self.same_line() {
            self.peek().span()
        } else {
            self.after_last()
        }
    }

    // ───────────── программа и блоки ─────────────

    pub fn program(mut self) -> PR<Program> {
        let body = self.stmts(None)?;
        Ok(Program::new(self.lang.code(), body))
    }

    /// Разбор с позициями концов инструкций и блоков (для форматтера).
    pub fn program_with_ends(mut self) -> PR<(Program, Ends, Ends)> {
        let body = self.stmts(None)?;
        Ok((Program::new(self.lang.code(), body), self.stmt_ends, self.block_ends))
    }

    /// Список инструкций до `}` (если `open` задан) или до конца файла.
    fn stmts(&mut self, open: Option<&Token>) -> PR<Vec<Stmt>> {
        let mut out = vec![];
        loop {
            while self.at_sym(";") {
                self.bump();
            }
            if self.peek().is_eof() {
                if let Some(o) = open {
                    return Err(Diagnostic::error("E-SYNTAX-003", "Не закрыт блок: нужна «}»", o.span())
                        .with_hint("добавьте «}» в конце блока, открытого здесь"));
                }
                break;
            }
            if self.at_sym("}") {
                if let Some(o) = open {
                    let close = self.bump();
                    self.block_ends.insert((o.line, o.col), close.line);
                    break;
                }
                return Err(Diagnostic::error("E-SYNTAX-001", "Лишняя закрывающая скобка «}»", self.peek().span())
                    .with_hint("у этой скобки нет пары «{» — удалите её"));
            }
            let st = self.statement()?;
            if let Some(sp) = st.span() {
                self.stmt_ends.insert((sp.line, sp.col), self.last_line);
            }
            out.push(st);
            if self.at_sym(";") {
                self.bump();
                continue;
            }
            if self.peek().is_eof() || self.at_sym("}") {
                continue;
            }
            if self.same_line() {
                let t = self.peek();
                let mut hint = format!("перенесите «{}» на новую строку или разделите инструкции «;»", t.text);
                if t.is_sym("=") {
                    hint = format!("для изменения значения используется «{}»: имя {} значение", self.w("var.assign"), self.w("var.assign"));
                }
                return Err(Diagnostic::error(
                    "E-SYNTAX-016",
                    "Лишний элемент после инструкции: две инструкции в одной строке нужно разделять переводом строки",
                    t.span(),
                )
                .with_hint(hint));
            }
        }
        Ok(out)
    }

    fn block(&mut self, after: &str) -> PR<Block> {
        if !self.at_sym("{") {
            return Err(Diagnostic::error("E-SYNTAX-002", "Ожидалась открывающая скобка «{»", self.err_at_peek_or_after())
                .with_hint(format!("после {after} нужен блок {{ ... }}")));
        }
        let open = self.bump();
        let body = self.stmts(Some(&open))?;
        Ok(Block::new(Some(open.span()), body))
    }

    // ───────────── инструкции ─────────────

    fn statement(&mut self) -> PR<Stmt> {
        let t = self.peek().clone();
        if t.is_kw() {
            if t.has_op("var.declare") && !t.has_op("obj.withProto") {
                return self.declare(false);
            }
            if t.has_op("var.mutable") {
                return self.declare(true);
            }
            if t.has_op("io.say") {
                self.bump();
                let value = self.value_after(&t)?;
                return Ok(Stmt::Say { span: Some(t.span()), value });
            }
            if t.has_op("io.ask") {
                return self.ask();
            }
            if t.has_op("ctrl.if") {
                return self.if_stmt();
            }
            if t.has_op("ctrl.while") {
                self.bump();
                let test = self.value_after(&t)?;
                let body = self.block(&format!("условия «{}»", t.text))?;
                return Ok(Stmt::While { span: Some(t.span()), test, body });
            }
            if t.has_op("ctrl.repeat") {
                return self.repeat();
            }
            if t.has_op("ctrl.foreach") {
                return self.foreach();
            }
            if t.has_op("ctrl.iterate") {
                return Err(Diagnostic::error(
                    "E-SYNTAX-001",
                    format!("Конструкция «{}» пока не поддерживается (открытый вопрос спецификации)", t.text),
                    t.span(),
                )
                .with_hint(format!(
                    "используйте {} элемент {} коллекция {{ ... }}",
                    self.w("ctrl.foreach"),
                    self.sup("ctrl.foreach", "iter")
                )));
            }
            if t.has_op("ctrl.break") {
                self.bump();
                return Ok(Stmt::Break { span: Some(t.span()) });
            }
            if t.has_op("ctrl.continue") {
                self.bump();
                return Ok(Stmt::Continue { span: Some(t.span()) });
            }
            if t.has_op("fn.declare") && matches!(self.peek_at(1).tk, Tk::Ident(_) | Tk::Kw(_)) && !self.peek_at(1).is_sym("(") {
                return self.function_decl();
            }
            if t.has_op("fn.return") {
                self.bump();
                let value = if self.same_line() && !self.at_sym("}") && !self.at_sym(";") {
                    Some(self.expr()?)
                } else {
                    None
                };
                return Ok(Stmt::Return { span: Some(t.span()), value });
            }
            if t.has_op("err.try") {
                return self.try_stmt();
            }
            if t.has_op("err.throw") {
                self.bump();
                let value = self.value_after(&t)?;
                return Ok(Stmt::Throw { span: Some(t.span()), value });
            }
            if t.has_op("mod.import") || t.has_sup("mod.import", "from") {
                return self.import();
            }
            if t.has_op("ctrl.else") {
                return Err(Diagnostic::error("E-SYNTAX-001", format!("«{}» без «{}»", t.text, self.w("ctrl.if")), t.span())
                    .with_hint(format!(
                        "«{}» ставится сразу после блока: {} условие {} {{ ... }} {} {{ ... }}",
                        t.text,
                        self.w("ctrl.if"),
                        self.sup("ctrl.if", "then"),
                        t.text
                    )));
            }
            if t.has_op("err.catch") {
                return Err(Diagnostic::error("E-SYNTAX-001", format!("«{}» без «{}»", t.text, self.w("err.try")), t.span())
                    .with_hint(format!("{} {{ ... }} {} ошибку {{ ... }}", self.w("err.try"), t.text)));
            }
        }
        // «имя пусть будет 5», «имя будет 5» — порядок слов перепутан.
        if let Tk::Ident(name) = &t.tk {
            let n = self.peek_at(1);
            if n.line == t.line && (n.has_op("var.declare") || n.has_sup("var.declare", "be")) {
                return Err(self.order_declare(name, n.span()));
            }
        }
        self.expr_statement()
    }

    fn order_declare(&self, name: &str, span: Span) -> Diagnostic {
        Diagnostic::error(
            "E-ORDER-001",
            format!("Порядок слов: объявление начинается с «{}», затем имя, затем «{}»", self.w("var.declare"), self.sup("var.declare", "be")),
            span,
        )
        .with_hint(format!("{} {} {} значение", self.w("var.declare"), name, self.sup("var.declare", "be")))
    }

    /// Значение после ключевого слова: должно начинаться на той же строке.
    fn value_after(&mut self, kw: &Token) -> PR<Expr> {
        if !self.same_line() || self.at_sym("}") || self.at_sym(";") {
            return Err(Diagnostic::error(
                "E-SYNTAX-011",
                "Ожидалось значение: число, строка, имя, список или блок",
                self.after_last(),
            )
            .with_hint(format!("после «{}» напишите значение на той же строке", kw.text)));
        }
        self.expr()
    }

    /// Имя (переменной, параметра, функции...). Ключевое слово здесь — E-IDENT-002.
    fn name(&mut self, what: &str) -> PR<(String, Span)> {
        let t = self.peek().clone();
        match &t.tk {
            Tk::Ident(n) if self.same_line() => {
                self.bump();
                Ok((n.clone(), t.span()))
            }
            Tk::Kw(_) if self.same_line() => Err(Diagnostic::error(
                "E-IDENT-002",
                format!("Здесь ожидалось {what}, а стоит ключевое слово «{}» — оно зарезервировано языком", t.text),
                t.span(),
            )
            .with_hint(format!("выберите другое имя, например «{}_1»", t.text.replace(' ', "_")))),
            _ => Err(Diagnostic::error("E-NAME-003", format!("Ожидалось {what}"), self.err_at_peek_or_after())),
        }
    }

    fn declare(&mut self, mutable: bool) -> PR<Stmt> {
        let kw = self.bump();
        let be = if mutable { self.w("var.assign") } else { self.sup("var.declare", "be") };
        if self.peek().has_sup("var.declare", "be") || (mutable && self.peek().has_op("var.assign")) {
            let span = self.peek().span();
            let mut d = self.order_declare("имя", span);
            d.hint = Some(format!("{} имя {} значение", kw.text, be));
            return Err(d);
        }
        let (name, _) = self.name("имя переменной")?;
        let p = self.peek();
        let sep = p.same_line_as(self.last_line)
            && (p.has_sup("var.declare", "be") || p.is_sym("=") || (mutable && p.has_op("var.assign")));
        if !sep {
            return Err(Diagnostic::error(
                "E-ORDER-001",
                format!("Не указано начальное значение: после имени нужно «{be}»"),
                self.err_at_peek_or_after(),
            )
            .with_hint(format!("{} {} {} значение", kw.text, name, be)));
        }
        let sep_tok = self.bump();
        let value = self.value_after(&sep_tok)?;
        Ok(Stmt::Declare { span: Some(kw.span()), name, mutable, value })
    }

    fn ask(&mut self) -> PR<Stmt> {
        let kw = self.bump();
        let prompt = self.value_after(&kw)?;
        let into = self.sup("io.ask", "into");
        if !(self.same_line() && self.peek().has_sup("io.ask", "into")) {
            return Err(Diagnostic::error(
                "E-ORDER-002",
                "После вопроса нужно указать, куда сохранить ответ",
                self.err_at_peek_or_after(),
            )
            .with_hint(format!("{} \"вопрос?\" {} имя", kw.text, into)));
        }
        self.bump();
        let (name, _) = self.name("имя переменной для ответа")?;
        Ok(Stmt::Ask { span: Some(kw.span()), prompt, name })
    }

    fn if_stmt(&mut self) -> PR<Stmt> {
        let kw = self.bump();
        let test = self.value_after(&kw)?;
        if self.peek().has_sup("ctrl.if", "then") {
            self.bump();
        }
        let then = self.block(&format!("условия «{}»", kw.text))?;
        let mut else_ = None;
        if self.peek().has_op("ctrl.else") {
            let e = self.bump();
            if self.peek().has_op("ctrl.if") {
                let inner = self.if_stmt()?;
                else_ = Some(Block::new(Some(e.span()), vec![inner]));
            } else {
                else_ = Some(self.block(&format!("«{}»", e.text))?);
            }
        }
        Ok(Stmt::If { span: Some(kw.span()), test, then, else_ })
    }

    fn repeat(&mut self) -> PR<Stmt> {
        let kw = self.bump();
        let times = self.value_after(&kw)?;
        if !self.peek().has_sup("ctrl.repeat", "times") {
            let w = self.sup("ctrl.repeat", "times");
            return Err(Diagnostic::error("E-ORDER-004", format!("После числа повторов нужно слово «{w}»"), self.err_at_peek_or_after())
                .with_hint(format!("{} 3 {} {{ ... }}", kw.text, w)));
        }
        self.bump();
        let body = self.block(&format!("«{}»", self.sup("ctrl.repeat", "times")))?;
        Ok(Stmt::Repeat { span: Some(kw.span()), times, body })
    }

    fn foreach(&mut self) -> PR<Stmt> {
        let kw = self.bump();
        let (item, _) = self.name("имя элемента")?;
        let inw = self.sup("ctrl.foreach", "iter");
        if !self.peek().has_sup("ctrl.foreach", "iter") {
            return Err(Diagnostic::error("E-ORDER-005", format!("Нужно слово «{inw}» перед коллекцией"), self.err_at_peek_or_after())
                .with_hint(format!("{} {} {} коллекция {{ ... }}", kw.text, item, inw)));
        }
        let in_tok = self.bump();
        let iter = self.value_after(&in_tok)?;
        let body = self.block("коллекции")?;
        Ok(Stmt::ForEach { span: Some(kw.span()), item, iter, body })
    }

    fn params(&mut self) -> PR<Vec<Param>> {
        if !self.at_sym("(") {
            return Err(Diagnostic::error("E-SYNTAX-004", "Ожидалась «(» после имени функции", self.err_at_peek_or_after())
                .with_hint(format!("{} имя(a, b) {{ ... }}", self.w("fn.declare"))));
        }
        let open = self.bump();
        let mut params = vec![];
        loop {
            if self.at_sym(")") {
                self.bump();
                break;
            }
            if self.peek().is_eof() || self.at_sym("{") {
                return Err(Diagnostic::error("E-SYNTAX-005", "Не закрыт список параметров: нужна «)»", open.span()));
            }
            let t = self.peek().clone();
            let (name, _) = match &t.tk {
                Tk::Ident(n) => {
                    self.bump();
                    (n.clone(), t.span())
                }
                Tk::Kw(_) => {
                    return Err(Diagnostic::error(
                        "E-IDENT-002",
                        format!("Здесь ожидалось имя параметра, а стоит ключевое слово «{}» — оно зарезервировано языком", t.text),
                        t.span(),
                    ))
                }
                _ => return Err(Diagnostic::error("E-NAME-003", "Ожидалось имя параметра", t.span())),
            };
            let mut type_hint = None;
            if self.at_sym(":") {
                self.bump();
                let tt = self.peek().clone();
                let Some(word) = tt.single_word() else {
                    return Err(Diagnostic::error("E-TYPE-002", "После «:» ожидалось имя типа", tt.span()));
                };
                match self.dict.type_id(self.lang, word) {
                    Some(id) => type_hint = Some(id),
                    None => {
                        return Err(Diagnostic::error("E-TYPE-002", format!("Неизвестный тип «{word}»"), tt.span())
                            .with_hint(format!("известные типы: {}", self.dict.type_names(self.lang).join(", "))))
                    }
                }
                self.bump();
            }
            let mut default = None;
            if self.at_sym("=") {
                self.bump();
                default = Some(self.expr()?);
            }
            params.push(Param { name, type_hint, default });
            if self.at_sym(",") {
                self.bump();
            } else if !self.at_sym(")") {
                return Err(Diagnostic::error("E-SYNTAX-005", "Не закрыт список параметров: нужна «)»", self.err_at_peek_or_after())
                    .with_hint("параметры разделяются запятой: (a, b)"));
            }
        }
        Ok(params)
    }

    fn function_decl(&mut self) -> PR<Stmt> {
        let kw = self.bump();
        let (name, _) = self.name("имя функции")?;
        let params = self.params()?;
        let body = self.block("списка параметров")?;
        Ok(Stmt::FunctionDecl { span: Some(kw.span()), name, params, body: Rc::new(body) })
    }

    fn try_stmt(&mut self) -> PR<Stmt> {
        let kw = self.bump();
        let body = self.block(&format!("«{}»", kw.text))?;
        if !self.peek().has_op("err.catch") {
            return Err(Diagnostic::error(
                "E-ORDER-006",
                format!("После «{} {{ ... }}» нужен блок «{} ошибку {{ ... }}»", kw.text, self.w("err.catch")),
                self.err_at_peek_or_after_any(),
            )
            .with_hint(format!("{} {{ ... }} {} ошибку {{ ... }}", kw.text, self.w("err.catch"))));
        }
        let c = self.bump();
        let (error_name, _) = self.name("имя для ошибки")?;
        let handler = self.block(&format!("«{} {}»", c.text, error_name))?;
        Ok(Stmt::Try { span: Some(kw.span()), body, error_name, handler })
    }

    fn err_at_peek_or_after_any(&self) -> Span {
        if self.peek().is_eof() {
            self.after_last()
        } else {
            self.peek().span()
        }
    }

    fn import(&mut self) -> PR<Stmt> {
        let first = self.bump();
        let mut from_tok = None;
        if first.has_sup("mod.import", "from") {
            from_tok = Some(first.clone());
        } else if self.same_line() && self.peek().has_sup("mod.import", "from") {
            from_tok = Some(self.bump());
        }
        let bad = |p: &Self| {
            Diagnostic::error("E-ORDER-008", "Ожидалось имя модуля или путь в кавычках", p.err_at_peek_or_after())
                .with_hint(format!(
                    "{} numeri — или — {} \"путь\" {} имя",
                    p.w("mod.import"),
                    p.sup("mod.import", "from"),
                    p.sup("mod.import", "take")
                ))
        };
        if from_tok.is_none() {
            return match self.peek().tk.clone() {
                Tk::Ident(m) if self.same_line() => {
                    self.bump();
                    Ok(Stmt::Import { span: Some(first.span()), module: Some(m), path: None, names: vec![] })
                }
                _ => Err(bad(self)),
            };
        }
        let path = match self.peek().tk.clone() {
            Tk::Str { value, parts: None } if self.same_line() => {
                self.bump();
                value
            }
            _ => return Err(bad(self)),
        };
        if !self.peek().has_sup("mod.import", "take") {
            let take = self.sup("mod.import", "take");
            return Err(Diagnostic::error("E-ORDER-007", format!("После пути нужно слово «{take}»"), self.err_at_peek_or_after())
                .with_hint(format!("{} \"{}\" {} имя", self.sup("mod.import", "from"), path, take)));
        }
        self.bump();
        let mut names = vec![self.name("имя для импорта")?.0];
        while self.at_sym(",") {
            self.bump();
            names.push(self.name("имя для импорта")?.0);
        }
        Ok(Stmt::Import { span: Some(first.span()), module: None, path: Some(path), names })
    }

    fn expr_statement(&mut self) -> PR<Stmt> {
        let e = self.expr()?;
        if self.same_line() && self.peek().has_op("var.assign") {
            let op = self.bump();
            if !matches!(e, Expr::Identifier { .. } | Expr::Member { .. }) {
                return Err(Diagnostic::error(
                    "E-TYPE-004",
                    format!("Слева от «{}» должно быть имя или поле", op.text),
                    e.span().unwrap_or(op.span()),
                ));
            }
            let value = self.value_after(&op)?;
            return Ok(Stmt::Assign { span: e.span(), target: e, value });
        }
        Ok(Stmt::ExprStmt { span: e.span(), expr: e })
    }

    // ───────────── выражения ─────────────

    pub fn expr(&mut self) -> PR<Expr> {
        self.or_expr()
    }

    fn or_expr(&mut self) -> PR<Expr> {
        let mut left = self.and_expr()?;
        while self.same_line() && self.peek().has_op("logic.or") {
            let op = self.bump();
            let right = self.and_expr()?;
            left = Expr::Binary { span: Some(op.span()), op: BinOp::Or, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn and_expr(&mut self) -> PR<Expr> {
        let mut left = self.cmp_expr()?;
        while self.same_line() && self.peek().has_op("logic.and") {
            let op = self.bump();
            let right = self.cmp_expr()?;
            left = Expr::Binary { span: Some(op.span()), op: BinOp::And, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn cmp_op(&self) -> Option<BinOp> {
        let t = self.peek();
        [
            ("cmp.ge", BinOp::Ge),
            ("cmp.le", BinOp::Le),
            ("cmp.ne", BinOp::Ne),
            ("cmp.gt", BinOp::Gt),
            ("cmp.lt", BinOp::Lt),
            ("cmp.eq", BinOp::Eq),
        ]
        .into_iter()
        .find(|(id, _)| t.has_op(id))
        .map(|(_, op)| op)
    }

    fn cmp_expr(&mut self) -> PR<Expr> {
        let left = self.add_expr()?;
        if self.same_line() {
            if let Some(op) = self.cmp_op() {
                let t = self.bump();
                let right = self.add_expr()?;
                return Ok(Expr::Binary { span: Some(t.span()), op, left: Box::new(left), right: Box::new(right) });
            }
        }
        Ok(left)
    }

    fn add_expr(&mut self) -> PR<Expr> {
        let mut left = self.mul_expr()?;
        while self.same_line() && (self.at_sym("+") || self.at_sym("-")) {
            let t = self.bump();
            let op = if t.is_sym("+") { BinOp::Add } else { BinOp::Sub };
            let right = self.mul_expr()?;
            left = Expr::Binary { span: Some(t.span()), op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn mul_expr(&mut self) -> PR<Expr> {
        let mut left = self.unary()?;
        while self.same_line() && (self.at_sym("*") || self.at_sym("/") || self.at_sym("%")) {
            let t = self.bump();
            let op = match t.text.as_str() {
                "*" => BinOp::Mul,
                "/" => BinOp::Div,
                _ => BinOp::Mod,
            };
            let right = self.unary()?;
            left = Expr::Binary { span: Some(t.span()), op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn unary(&mut self) -> PR<Expr> {
        if self.at_sym("-") {
            let t = self.bump();
            let operand = self.unary()?;
            return Ok(Expr::Unary { span: Some(t.span()), op: UnOp::Neg, operand: Box::new(operand) });
        }
        if self.peek().has_op("logic.not") {
            let t = self.bump();
            let operand = self.unary()?;
            return Ok(Expr::Unary { span: Some(t.span()), op: UnOp::Not, operand: Box::new(operand) });
        }
        self.postfix()
    }

    fn postfix(&mut self) -> PR<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.at_sym("(") && self.same_line() {
                let open = self.bump();
                let args = self.args(&open)?;
                let span = join(e.span(), Span::new(self.last_end.0, self.last_end.1 - 1, 1));
                e = Expr::Call { span, callee: Box::new(e), args };
            } else if self.at_sym(".") {
                let dot = self.bump();
                let t = self.peek().clone();
                let Some(prop) = t.single_word().map(|s| s.to_string()) else {
                    return Err(Diagnostic::error("E-SYNTAX-007", "После точки ожидалось имя свойства", dot.span()));
                };
                self.bump();
                let span = join(e.span(), t.span());
                e = Expr::Member { span, object: Box::new(e), property: prop };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn args(&mut self, open: &Token) -> PR<Vec<Arg>> {
        let mut args = vec![];
        let mut named_seen = false;
        loop {
            if self.at_sym(")") {
                self.bump();
                break;
            }
            if self.peek().is_eof() {
                return Err(Diagnostic::error("E-SYNTAX-006", "Не закрыт вызов: нужна «)»", open.span()));
            }
            let named = matches!(self.peek().tk, Tk::Ident(_)) && self.peek_at(1).is_sym("=");
            if named {
                let n = self.bump();
                self.bump(); // =
                named_seen = true;
                args.push(Arg { value: self.expr()?, name: Some(n.text) });
            } else {
                if named_seen {
                    return Err(Diagnostic::error(
                        "E-SYNTAX-001",
                        "Позиционный аргумент после именованного",
                        self.peek().span(),
                    )
                    .with_hint("сначала позиционные аргументы, потом именованные: f(1, b = 2)"));
                }
                args.push(Arg { value: self.expr()?, name: None });
            }
            if self.at_sym(",") {
                self.bump();
            } else if !self.at_sym(")") {
                return Err(Diagnostic::error("E-SYNTAX-006", "Не закрыт вызов: нужна «)»", self.err_at_peek_or_after_any())
                    .with_hint("аргументы разделяются запятой: f(a, b)"));
            }
        }
        Ok(args)
    }

    fn entries(&mut self, open: &Token, unclosed: (&str, &str)) -> PR<Vec<Entry>> {
        let mut entries = vec![];
        loop {
            if self.at_sym("}") {
                self.bump();
                break;
            }
            if self.peek().is_eof() {
                return Err(Diagnostic::error(unclosed.0, unclosed.1, open.span()));
            }
            let t = self.peek().clone();
            let key = match &t.tk {
                Tk::Str { value, parts: None } => value.clone(),
                _ => match t.single_word() {
                    Some(w) => w.to_string(),
                    None => return Err(Diagnostic::error("E-SYNTAX-014", "Ожидалось имя поля", t.span())),
                },
            };
            self.bump();
            if !self.at_sym(":") {
                return Err(Diagnostic::error("E-SYNTAX-015", "После имени поля нужно «:»", self.err_at_peek_or_after())
                    .with_hint(format!("{key}: значение")));
            }
            self.bump();
            let value = self.expr()?;
            entries.push(Entry { key, value });
            if self.at_sym(",") {
                self.bump();
            } else if !self.at_sym("}") {
                if self.peek().is_eof() {
                    return Err(Diagnostic::error(unclosed.0, unclosed.1, open.span()));
                }
                return Err(Diagnostic::error(unclosed.0, unclosed.1, self.peek().span())
                    .with_hint("поля разделяются запятой: { a: 1, b: 2 }"));
            }
        }
        Ok(entries)
    }

    fn string_parts(&self, parts: &[RawPart], span: Span) -> PR<Vec<StringPart>> {
        let mut out = vec![];
        for p in parts {
            match p {
                RawPart::Text(t) => out.push(StringPart::Text { value: t.clone() }),
                RawPart::Hole(path) => {
                    let first = &path[0];
                    let mut e = match self.dict.lookup(self.lang, &[crate::keywords::norm(first)]) {
                        Some((_, ms)) if ms.contains(&crate::keywords::Meaning::Op("fn.self".into())) => Expr::SelfExpr { span: None },
                        Some(_) => {
                            return Err(Diagnostic::error(
                                "E-IDENT-002",
                                format!("Здесь ожидалось имя, а стоит ключевое слово «{first}» — оно зарезервировано языком"),
                                span,
                            ))
                        }
                        None => Expr::Identifier { span: None, name: first.clone() },
                    };
                    for prop in &path[1..] {
                        e = Expr::Member { span: None, object: Box::new(e), property: prop.clone() };
                    }
                    out.push(StringPart::Hole { expr: e });
                }
            }
        }
        Ok(out)
    }

    fn primary(&mut self) -> PR<Expr> {
        let t = self.peek().clone();
        let sp = Some(t.span());
        match &t.tk {
            Tk::Num(v) => {
                self.bump();
                Ok(Expr::NumberLit { span: sp, value: *v })
            }
            Tk::Str { value, parts } => {
                self.bump();
                let parts = match parts {
                    Some(ps) => Some(self.string_parts(ps, t.span())?),
                    None => None,
                };
                Ok(Expr::StringLit { span: sp, value: value.clone(), parts })
            }
            Tk::Ident(name) => {
                self.bump();
                Ok(Expr::Identifier { span: sp, name: name.clone() })
            }
            Tk::Sym("(") => {
                self.bump();
                let e = self.expr()?;
                if !self.at_sym(")") {
                    return Err(Diagnostic::error("E-SYNTAX-008", "Не закрыта скобка «)»", t.span()));
                }
                self.bump();
                Ok(e)
            }
            Tk::Sym("[") => {
                self.bump();
                let mut items = vec![];
                loop {
                    if self.at_sym("]") {
                        self.bump();
                        break;
                    }
                    if self.peek().is_eof() {
                        return Err(Diagnostic::error("E-SYNTAX-009", "Не закрыт список: нужна «]»", t.span()));
                    }
                    items.push(self.expr()?);
                    if self.at_sym(",") {
                        self.bump();
                    } else if !self.at_sym("]") {
                        return Err(Diagnostic::error("E-SYNTAX-009", "Не закрыт список: нужна «]»", self.err_at_peek_or_after_any())
                            .with_hint("элементы списка разделяются запятой: [1, 2, 3]"));
                    }
                }
                Ok(Expr::ListLit { span: sp, items })
            }
            Tk::Sym("{") => {
                let open = self.bump();
                let entries = self.entries(&open, ("E-SYNTAX-010", "Не закрыт словарь: нужна «}»"))?;
                Ok(Expr::MapLit { span: sp, entries })
            }
            Tk::Kw(_) => {
                if t.has_op("lit.null") {
                    self.bump();
                    return Ok(Expr::NullLit { span: sp });
                }
                if t.has_op("lit.true") || t.has_op("lit.false") {
                    self.bump();
                    return Ok(Expr::BoolLit { span: sp, value: t.has_op("lit.true") });
                }
                if t.has_op("fn.self") {
                    self.bump();
                    return Ok(Expr::SelfExpr { span: sp });
                }
                if t.has_op("fn.declare") {
                    self.bump();
                    let params = self.params()?;
                    let body = self.block("списка параметров")?;
                    return Ok(Expr::FunctionExpr { span: sp, params, body: Rc::new(body) });
                }
                if t.has_op("obj.withProto") {
                    self.bump();
                    let proto = self.postfix()?;
                    if !self.at_sym("{") {
                        return Err(Diagnostic::error("E-SYNTAX-012", "Ожидалась «{» после прототипа", self.err_at_peek_or_after())
                            .with_hint(format!("{} прототип {{ поле: значение }}", t.text)));
                    }
                    let open = self.bump();
                    let entries = self.entries(&open, ("E-SYNTAX-013", "Не закрыт объект: нужна «}»"))?;
                    return Ok(Expr::ObjectExpr { span: sp, proto: Box::new(proto), entries });
                }
                Err(Diagnostic::error(
                    "E-ORDER-009",
                    format!("Ключевое слово «{}» стоит не на месте: здесь ожидалось значение или имя", t.text),
                    t.span(),
                ))
            }
            Tk::Eof => Err(Diagnostic::error(
                "E-SYNTAX-011",
                "Ожидалось значение: число, строка, имя, список или блок",
                self.after_last(),
            )),
            Tk::Sym(_) => Err(Diagnostic::error(
                "E-SYNTAX-011",
                "Ожидалось значение: число, строка, имя, список или блок",
                t.span(),
            )
            .with_hint(format!("«{}» не может начинать значение", t.text))),
        }
    }
}

trait SameLine {
    fn same_line_as(&self, line: u32) -> bool;
}
impl SameLine for Token {
    fn same_line_as(&self, line: u32) -> bool {
        !self.is_eof() && self.line == line
    }
}
