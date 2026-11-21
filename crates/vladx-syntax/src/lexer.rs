//! Лексер: исходник → токены. Ключевые слова распознаются по словарю
//! (самая длинная фраза), сравнение без учёта регистра.

use crate::keywords::{norm, Dictionary, Lang, Meaning};
use unicode_normalization::UnicodeNormalization;
use vladx_ast::{Diagnostic, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum RawPart {
    Text(String),
    /// Путь по полям: `человек.имя` → ["человек", "имя"].
    Hole(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tk {
    Num(f64),
    Str { value: String, parts: Option<Vec<RawPart>> },
    Ident(String),
    Kw(Vec<Meaning>),
    Sym(&'static str),
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tk: Tk,
    /// Текст как в исходнике.
    pub text: String,
    pub line: u32,
    pub col: u32,
    pub len: u32,
}

impl Token {
    pub fn span(&self) -> Span {
        Span::new(self.line, self.col, self.len)
    }
    pub fn has_op(&self, id: &str) -> bool {
        matches!(&self.tk, Tk::Kw(ms) if ms.iter().any(|m| matches!(m, Meaning::Op(x) if x == id)))
    }
    pub fn has_sup(&self, op: &str, role: &str) -> bool {
        matches!(&self.tk, Tk::Kw(ms) if ms.iter().any(|m| matches!(m, Meaning::Sup(o, r) if o == op && r == role)))
    }
    pub fn is_sym(&self, s: &str) -> bool {
        matches!(&self.tk, Tk::Sym(x) if *x == s)
    }
    pub fn is_kw(&self) -> bool {
        matches!(self.tk, Tk::Kw(_))
    }
    pub fn is_eof(&self) -> bool {
        matches!(self.tk, Tk::Eof)
    }
    /// Одно слово (идентификатор или ключевое слово из одного слова).
    pub fn single_word(&self) -> Option<&str> {
        match &self.tk {
            Tk::Ident(s) => Some(s),
            Tk::Kw(_) if !self.text.contains(char::is_whitespace) => Some(&self.text),
            _ => None,
        }
    }
}

const SYMS: [&str; 15] = ["{", "}", "(", ")", "[", "]", ",", ":", ".", ";", "=", "+", "-", "*", "/"];

fn is_word_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || unicode_mark(c)
}
fn unicode_mark(c: char) -> bool {
    // Комбинируемые диакритики (на случай, если NFC не свёл букву в один символ).
    ('\u{0300}'..='\u{036f}').contains(&c)
}

struct Lx<'a> {
    c: Vec<char>,
    i: usize,
    line: u32,
    col: u32,
    dict: &'a Dictionary,
    lang: Lang,
}

impl<'a> Lx<'a> {
    fn peek(&self, k: usize) -> Option<char> {
        self.c.get(self.i + k).copied()
    }
    fn adv(&mut self) -> Option<char> {
        let ch = self.c.get(self.i).copied()?;
        self.i += 1;
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(ch)
    }
}

/// Комментарий исходника (для форматтера): текст вместе с `//` или `#`.
#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    pub line: u32,
    pub col: u32,
    pub text: String,
    /// Комментарий занимает строку целиком (а не стоит после кода).
    pub own_line: bool,
}

/// Разбор исходника в токены (последний — `Eof`).
pub fn lex(src: &str, lang: Lang, dict: &Dictionary) -> Result<Vec<Token>, Diagnostic> {
    lex_with_comments(src, lang, dict).map(|(t, _)| t)
}

/// То же, что [`lex`], плюс комментарии.
pub fn lex_with_comments(src: &str, lang: Lang, dict: &Dictionary) -> Result<(Vec<Token>, Vec<Comment>), Diagnostic> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let normalized: String = src.nfc().collect();
    let mut lx = Lx { c: normalized.chars().collect(), i: 0, line: 1, col: 1, dict, lang };
    let mut raw: Vec<(Token, bool)> = vec![]; // (токен, это слово?)
    let mut comments: Vec<Comment> = vec![];

    while let Some(ch) = lx.peek(0) {
        let (line, col) = (lx.line, lx.col);
        if ch.is_whitespace() {
            lx.adv();
            continue;
        }
        if ch == '#' || (ch == '/' && lx.peek(1) == Some('/')) {
            let start = lx.i;
            while let Some(c) = lx.peek(0) {
                if c == '\n' {
                    break;
                }
                lx.adv();
            }
            let text: String = lx.c[start..lx.i].iter().collect();
            let own_line = !raw.last().is_some_and(|(t, _)| t.line == line);
            comments.push(Comment { line, col, text: text.trim_end().to_string(), own_line });
            continue;
        }
        if ch.is_ascii_digit() {
            let start = lx.i;
            while lx.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                lx.adv();
            }
            if lx.peek(0) == Some('.') && lx.peek(1).is_some_and(|c| c.is_ascii_digit()) {
                lx.adv();
                while lx.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                    lx.adv();
                }
            }
            if lx.peek(0).is_some_and(is_word_char) {
                while lx.peek(0).is_some_and(is_word_char) {
                    lx.adv();
                }
                let text: String = lx.c[start..lx.i].iter().collect();
                return Err(Diagnostic::error(
                    "E-LEX-002",
                    format!("Не удалось прочитать число «{text}»"),
                    Span::new(line, col, (lx.i - start) as u32),
                )
                .with_hint("между числом и словом нужен пробел; число пишется цифрами, дробная часть — через точку: 3.5"));
            }
            let text: String = lx.c[start..lx.i].iter().collect();
            let v: f64 = text.parse().map_err(|_| {
                Diagnostic::error("E-LEX-002", format!("Не удалось прочитать число «{text}»"), Span::new(line, col, text.chars().count() as u32))
            })?;
            raw.push((Token { tk: Tk::Num(v), len: text.chars().count() as u32, text, line, col }, false));
            continue;
        }
        if ch == '"' {
            raw.push((lex_string(&mut lx)?, false));
            continue;
        }
        if is_word_start(ch) {
            let start = lx.i;
            while lx.peek(0).is_some_and(is_word_char) {
                lx.adv();
            }
            let text: String = lx.c[start..lx.i].iter().collect();
            raw.push((
                Token { tk: Tk::Ident(text.clone()), len: (lx.i - start) as u32, text, line, col },
                true,
            ));
            continue;
        }
        if ch == '%' {
            lx.adv();
            raw.push((Token { tk: Tk::Sym("%"), text: "%".into(), line, col, len: 1 }, false));
            continue;
        }
        if let Some(s) = SYMS.iter().find(|s| s.starts_with(ch)) {
            // «==», «=>» и подобные — подсказка: сравнения пишутся словами.
            if ch == '=' && lx.peek(1) == Some('=') {
                return Err(unknown_symbol(&lx, "==", line, col));
            }
            lx.adv();
            raw.push((Token { tk: Tk::Sym(s), text: s.to_string(), line, col, len: 1 }, false));
            continue;
        }
        let mut sym = ch.to_string();
        if let Some(n) = lx.peek(1) {
            if matches!((ch, n), ('>', '=') | ('<', '=') | ('!', '=') | ('&', '&') | ('|', '|')) {
                sym.push(n);
            }
        }
        return Err(unknown_symbol(&lx, &sym, line, col));
    }

    // Второй проход: слова → ключевые фразы (самая длинная) или имена.
    let mut out: Vec<Token> = Vec::with_capacity(raw.len() + 1);
    let mut i = 0;
    while i < raw.len() {
        let (tok, is_word) = &raw[i];
        if !*is_word {
            out.push(tok.clone());
            i += 1;
            continue;
        }
        // После точки — всегда имя свойства, даже если совпадает с ключевым словом.
        if out.last().is_some_and(|t| t.is_sym(".")) {
            out.push(tok.clone());
            i += 1;
            continue;
        }
        let mut words = vec![];
        let mut j = i;
        while j < raw.len() && raw[j].1 && raw[j].0.line == tok.line && words.len() < lx.dict.max_words() {
            words.push(norm(&raw[j].0.text));
            j += 1;
        }
        if let Some((n, meanings)) = lx.dict.lookup(lx.lang, &words) {
            let last = &raw[i + n - 1].0;
            let text = raw[i..i + n].iter().map(|(t, _)| t.text.as_str()).collect::<Vec<_>>().join(" ");
            out.push(Token {
                tk: Tk::Kw(meanings.clone()),
                text,
                line: tok.line,
                col: tok.col,
                len: last.col + last.len - tok.col,
            });
            i += n;
        } else {
            out.push(tok.clone());
            i += 1;
        }
    }
    out.push(Token { tk: Tk::Eof, text: String::new(), line: lx.line, col: lx.col, len: 1 });
    Ok((out, comments))
}

fn unknown_symbol(lx: &Lx, sym: &str, line: u32, col: u32) -> Diagnostic {
    let d = |w: &str| lx.dict.form(lx.lang, w);
    let hint = match sym {
        ">" => Some(format!("сравнения пишутся словами: a {} b", d("cmp.gt"))),
        "<" => Some(format!("сравнения пишутся словами: a {} b", d("cmp.lt"))),
        ">=" => Some(format!("сравнения пишутся словами: a {} b", d("cmp.ge"))),
        "<=" => Some(format!("сравнения пишутся словами: a {} b", d("cmp.le"))),
        "==" => Some(format!("сравнения пишутся словами: a {} b", d("cmp.eq"))),
        "!=" => Some(format!("сравнения пишутся словами: a {} b", d("cmp.ne"))),
        "!" => Some(format!("отрицание пишется словом: {} a", d("logic.not"))),
        "&&" | "&" => Some(format!("логическое «{0}» пишется словом: a {0} b", d("logic.and"))),
        "||" | "|" => Some(format!("логическое «{0}» пишется словом: a {0} b", d("logic.or"))),
        "'" | "«" | "»" | "“" | "”" => Some("строки пишутся в прямых двойных кавычках: \"текст\"".to_string()),
        _ => None,
    };
    let d = Diagnostic::error("E-LEX-003", format!("Неизвестный символ «{sym}»"), Span::new(line, col, sym.chars().count() as u32));
    match hint {
        Some(h) => d.with_hint(h),
        None => d,
    }
}

fn lex_string(lx: &mut Lx) -> Result<Token, Diagnostic> {
    let (line, col) = (lx.line, lx.col);
    let start = lx.i;
    lx.adv(); // "
    let mut value = String::new(); // текст без интерполяции (экранирование обработано)
    let mut parts: Vec<RawPart> = vec![];
    let mut cur = String::new();
    let mut has_holes = false;
    let unclosed = |lx: &Lx| {
        Diagnostic::error("E-LEX-001", "Незакрытая строка", Span::new(line, col, (lx.i - start) as u32))
            .with_hint("закройте строку кавычкой \" на той же строке")
    };
    loop {
        let Some(ch) = lx.peek(0) else { return Err(unclosed(lx)) };
        match ch {
            '\n' => return Err(unclosed(lx)),
            '"' => {
                lx.adv();
                break;
            }
            '\\' => {
                lx.adv();
                let Some(e) = lx.peek(0) else { return Err(unclosed(lx)) };
                if e == '\n' {
                    return Err(unclosed(lx));
                }
                lx.adv();
                let r = match e {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    other => other, // \" \\ \{ \}
                };
                cur.push(r);
                value.push(r);
            }
            '{' => {
                let (hl, hc) = (lx.line, lx.col);
                lx.adv();
                let mut inner = String::new();
                loop {
                    match lx.peek(0) {
                        None | Some('\n') | Some('"') => {
                            return Err(Diagnostic::error(
                                "E-STRING-001",
                                "Не закрыта интерполяция в строке: нет «}»",
                                Span::new(hl, hc, 1),
                            )
                            .with_hint("закройте скобку: {имя}; литеральная скобка пишется как \\{"));
                        }
                        Some('}') => {
                            lx.adv();
                            break;
                        }
                        Some(c) => {
                            inner.push(c);
                            lx.adv();
                        }
                    }
                }
                let span = Span::new(hl, hc, inner.chars().count() as u32 + 2);
                let trimmed = inner.trim();
                if trimmed.is_empty() {
                    return Err(Diagnostic::error("E-STRING-002", "Пустая интерполяция «{}»", span)
                        .with_hint("внутри скобок напишите имя: {имя}; литеральная скобка — \\{"));
                }
                let segs: Vec<String> = trimmed.split('.').map(|s| s.to_string()).collect();
                let ok = segs.iter().all(|s| {
                    let mut cs = s.chars();
                    cs.next().is_some_and(is_word_start) && cs.all(is_word_char)
                });
                if !ok {
                    let let_w = lx.dict.form(lx.lang, "var.declare");
                    let be = lx.dict.support(lx.lang, "var.declare", "be").unwrap_or_else(|| "=".into());
                    return Err(Diagnostic::error(
                        "E-STRING-003",
                        format!("Внутри скобок интерполяции может быть только имя или путь по полям, а не выражение «{trimmed}»"),
                        span,
                    )
                    .with_hint(format!("вынесите в переменную: {let_w} д {be} {trimmed} — а в строке напишите {{д}}")));
                }
                if !cur.is_empty() {
                    parts.push(RawPart::Text(std::mem::take(&mut cur)));
                }
                parts.push(RawPart::Hole(segs));
                has_holes = true;
                value.push('{');
                value.push_str(&inner);
                value.push('}');
            }
            c => {
                lx.adv();
                cur.push(c);
                value.push(c);
            }
        }
    }
    if !cur.is_empty() {
        parts.push(RawPart::Text(cur));
    }
    let text: String = lx.c[start..lx.i].iter().collect();
    Ok(Token {
        tk: Tk::Str { value, parts: if has_holes { Some(parts) } else { None } },
        len: (lx.i - start) as u32,
        text,
        line,
        col,
    })
}

/// Токен для подсветки: строка, колонка, длина (в символах), вид.
pub type HlToken = (u32, u32, u32, &'static str);

/// Разметка исходника для подсветки синтаксиса тем же лексером, что исполняет код.
/// Виды: `kw` ключевое слово, `lit` литерал, `op` слово-оператор (сравнение, логика),
/// `id` имя, `fn` функция, `mod` модуль/глобальная функция, `prop` поле, `num`,
/// `str`, `com` комментарий, `punct`. При ошибке лексики размечается всё до неё,
/// а ошибка возвращается отдельно.
pub fn highlight(src: &str, lang: Lang, dict: &Dictionary, globals: &[&str]) -> (Vec<HlToken>, Option<Diagnostic>) {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let (toks, comments, err) = match lex_with_comments(src, lang, dict) {
        Ok((t, c)) => (t, c, None),
        Err(d) => {
            // Лексика до места ошибки: сначала до самой ошибки, иначе — до начала её строки
            // (ошибка внутри строки «…» ломает и укороченный текст).
            let mut line_start = 0usize;
            let mut at_error = 0usize;
            for (i, l) in src.split_inclusive('\n').enumerate() {
                if i as u32 + 1 == d.line {
                    at_error = line_start + l.chars().take(d.col.saturating_sub(1) as usize).map(char::len_utf8).sum::<usize>();
                    break;
                }
                line_start += l.len();
            }
            let try_prefix = |end: usize| lex_with_comments(&src[..end.min(src.len())], lang, dict).ok();
            match try_prefix(at_error).or_else(|| try_prefix(line_start)) {
                Some((t, c)) => (t, c, Some(d)),
                None => (vec![], vec![], Some(d)),
            }
        }
    };
    let mut out: Vec<HlToken> = Vec::with_capacity(toks.len() + comments.len());
    for (i, t) in toks.iter().enumerate() {
        let prev = if i > 0 { Some(&toks[i - 1]) } else { None };
        let next = toks.get(i + 1);
        let kind = match &t.tk {
            Tk::Eof => continue,
            Tk::Num(_) => "num",
            Tk::Str { .. } => "str",
            Tk::Sym(_) => "punct",
            Tk::Kw(ms) => {
                let op = |p: &str| ms.iter().any(|m| matches!(m, Meaning::Op(id) if id.starts_with(p)));
                if op("lit.") {
                    "lit"
                } else if op("cmp.") || op("logic.") {
                    "op"
                } else {
                    "kw"
                }
            }
            Tk::Ident(name) => {
                if prev.is_some_and(|p| p.is_sym(".")) {
                    if next.is_some_and(|n| n.is_sym("(") && n.line == t.line) {
                        "fn"
                    } else {
                        "prop"
                    }
                } else if globals.contains(&name.as_str()) {
                    "mod"
                } else if prev.is_some_and(|p| p.has_op("fn.declare")) || next.is_some_and(|n| n.is_sym("(") && n.line == t.line) {
                    "fn"
                } else {
                    "id"
                }
            }
        };
        out.push((t.line, t.col, t.len, kind));
    }
    for c in comments {
        out.push((c.line, c.col, c.text.chars().count() as u32, "com"));
    }
    out.sort();
    (out, err)
}
