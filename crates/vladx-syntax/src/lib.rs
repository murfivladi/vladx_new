//! VladX: словарь → лексер → парсер → статические проверки.
//!
//! ```
//! use vladx_syntax::{Dictionary, Lang};
//! let dict = Dictionary::builtin();
//! let p = vladx_syntax::parse("скажи 1 + 2", Lang::Ru, &dict).unwrap();
//! assert_eq!(p.body.len(), 1);
//! ```

pub mod keywords;
pub mod lexer;
pub mod parser;
pub mod printer;
pub mod semantic;
pub mod validate;

pub use keywords::{Dictionary, Lang, Meaning};
pub use vladx_ast::{Diagnostic, Program, Severity, Span};

/// Словарь, вшитый в бинарь: `vladx` работает из любой папки.
pub const BUILTIN_KEYWORDS: &str = include_str!("../../../spec/keywords.toml");

/// Разбор исходника в AST. Первая ошибка останавливает разбор.
pub fn parse(src: &str, lang: Lang, dict: &Dictionary) -> Result<Program, Diagnostic> {
    let toks = lexer::lex(src, lang, dict)?;
    parser::Parser::new(toks, dict, lang).program()
}

/// Разбор с комментариями и концами конструкций (для форматтера).
pub fn parse_with_trivia(src: &str, lang: Lang, dict: &Dictionary) -> Result<(Program, printer::Trivia), Diagnostic> {
    let (toks, comments) = lexer::lex_with_comments(src, lang, dict)?;
    let (p, stmt_ends, block_ends) = parser::Parser::new(toks, dict, lang).program_with_ends()?;
    Ok((p, printer::Trivia { comments, stmt_ends, block_ends }))
}

/// Форматирование и перевод: исходник на `from` → канонический текст на `to`.
/// Комментарии и пустые строки сохраняются. Ошибка перевода (например, у `scn`
/// нет слова «больше или равно») возвращается как E-FMT-001.
pub fn format_source(src: &str, from: Lang, to: Lang, dict: &Dictionary) -> Result<String, Diagnostic> {
    let (p, trivia) = parse_with_trivia(src, from, dict)?;
    let out = printer::print_program(&p, dict, to, Some(&trivia))
        .map_err(|e| Diagnostic::error("E-FMT-001", format!("не удалось записать на {}: {e}", to.code()), Span::new(1, 1, 1)))?;
    // Страховка: смысл не должен измениться.
    let back = parse(&out, to, dict).map_err(|e| {
        Diagnostic::error("E-FMT-001", format!("результат не разбирается: {}", e.message), Span::new(e.line, e.col, e.len))
    })?;
    if vladx_ast::neutral_json(&back) != vladx_ast::neutral_json(&p) {
        return Err(Diagnostic::error("E-FMT-001", "форматирование изменило бы смысл программы", Span::new(1, 1, 1)));
    }
    Ok(out)
}

/// Разбор + статические проверки. Возвращает AST (если разбор удался)
/// и все найденные диагностики.
pub fn analyze(src: &str, lang: Lang, dict: &Dictionary, globals: &[&str]) -> (Option<Program>, Vec<Diagnostic>) {
    match parse(src, lang, dict) {
        Err(d) => (None, vec![d]),
        Ok(p) => {
            let diags = semantic::check(&p, dict, lang, globals);
            (Some(p), diags)
        }
    }
}

/// Исходник из байтов: UTF-8 обязателен (E-ENC-001).
pub fn decode_source(bytes: &[u8]) -> Result<String, Diagnostic> {
    String::from_utf8(bytes.to_vec()).map_err(|e| {
        let valid = &bytes[..e.utf8_error().valid_up_to()];
        let text = String::from_utf8_lossy(valid);
        let line = text.matches('\n').count() as u32 + 1;
        let col = text.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0) as u32 + 1;
        Diagnostic::error("E-ENC-001", "Исходник не в UTF-8", Span::new(line, col, 1))
            .with_hint("сохраните файл в кодировке UTF-8")
    })
}
