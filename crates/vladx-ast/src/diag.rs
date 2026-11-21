//! Диагностика: код латиницей (стабилен), сообщение и подсказка — на языке проекта.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub line: u32,
    pub col: u32,
    pub len: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    pub severity: Severity,
    /// Файл, к которому относится позиция (модуль проекта); нет — главная программа.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &str, message: impl Into<String>, span: crate::Span) -> Self {
        Diagnostic {
            code: code.to_string(),
            message: message.into(),
            line: span.line,
            col: span.col,
            len: span.len.max(1),
            hint: None,
            severity: Severity::Error,
            file: None,
        }
    }

    pub fn warning(code: &str, message: impl Into<String>, span: crate::Span) -> Self {
        let mut d = Diagnostic::error(code, message, span);
        d.severity = Severity::Warning;
        d
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// Человеческий вид:
    /// ```text
    /// файл:строка:колонка  КОД  сообщение
    ///   N | строка исходника
    ///     | ^^^^^^^^^
    ///   подсказка: что написать вместо
    /// ```
    pub fn render(&self, file: &str, source: &str) -> String {
        let mut out = format!("{}:{}:{}  {}  {}\n", file, self.line, self.col, self.code, self.message);
        if self.line >= 1 {
            if let Some(src_line) = source.lines().nth(self.line as usize - 1) {
                let src_line = src_line.trim_start_matches('\u{feff}');
                let num = self.line.to_string();
                let pad = " ".repeat(num.len());
                out.push_str(&format!("  {} | {}\n", num, src_line));
                let width = src_line.chars().count() as u32;
                let col = self.col.max(1);
                let len = self.len.max(1).min(width.saturating_sub(col - 1).max(1));
                out.push_str(&format!(
                    "  {} | {}{}\n",
                    pad,
                    " ".repeat((col - 1) as usize),
                    "^".repeat(len as usize)
                ));
            }
        }
        if let Some(h) = &self.hint {
            out.push_str(&format!("  подсказка: {}\n", h));
        }
        out
    }
}
