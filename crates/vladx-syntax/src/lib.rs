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
pub mod semantic;

pub use keywords::{Dictionary, Lang, Meaning};
pub use vladx_ast::{Diagnostic, Program, Severity, Span};

/// Словарь, вшитый в бинарь: `vladx` работает из любой папки.
pub const BUILTIN_KEYWORDS: &str = include_str!("../../../spec/keywords.toml");

/// Разбор исходника в AST. Первая ошибка останавливает разбор.
pub fn parse(src: &str, lang: Lang, dict: &Dictionary) -> Result<Program, Diagnostic> {
    let toks = lexer::lex(src, lang, dict)?;
    parser::Parser::new(toks, dict, lang).program()
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
