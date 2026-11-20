//! `.vlhmn` — равноправный формат программы: свободный текст, который LLM
//! переводит в тот же нейтральный AST, что и парсер `.vlstd`.
//!
//! Конвейер для каждого куска прозы:
//! текст → LLM → JSON → проверка (схема → печать в `.vlstd` → разбор →
//! совпадение деревьев → статические проверки) → при ошибке один повтор
//! с перечнем проблем → кэш по хешу.
//!
//! Форматы смешиваются: блоки кода в тройных кавычках (```) внутри `.vlhmn`
//! разбираются обычным парсером и в LLM не уходят.
//!
//! Сеть сюда не входит: провайдер (Polza, заглушка в тестах, …) реализует [`Llm`].

pub mod prompt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vladx_ast::{Diagnostic, Program, Span, Stmt};
use vladx_syntax::validate::{validate, validate_json, Checked};
use vladx_syntax::{Dictionary, Lang};

/// Сообщение диалога с LLM.
#[derive(Debug, Clone)]
pub struct Msg {
    pub role: &'static str,
    pub content: String,
}

/// Провайдер LLM: принимает диалог, возвращает текст ответа.
pub trait Llm {
    /// Идентификатор модели (входит в ключ кэша).
    fn model(&self) -> String;
    fn chat(&mut self, messages: &[Msg]) -> Result<String, String>;
}

/// Кэш переводов: ключ — хеш (версия промпта, модель, язык, контекст, текст).
pub trait Cache {
    fn get(&mut self, key: &str) -> Option<String>;
    fn put(&mut self, key: &str, value: &str);
}

/// Без кэша.
pub struct NoCache;
impl Cache for NoCache {
    fn get(&mut self, _: &str) -> Option<String> {
        None
    }
    fn put(&mut self, _: &str, _: &str) {}
}

/// Порог уверенности по умолчанию (открытый вопрос §14.6 — настраивается).
pub const DEFAULT_MIN_CONFIDENCE: f64 = 0.8;

/// Результат перевода всего файла.
#[derive(Debug, Clone)]
pub struct Translation {
    /// Проверенная программа и её текст `.vlstd` (позиции — по этому тексту).
    pub checked: Checked,
    /// Минимальная уверенность по кускам прозы (1.0, если прозы нет).
    pub confidence: f64,
    pub questions: Vec<String>,
    /// Запросов к LLM (с повторами).
    pub llm_calls: u32,
    pub prose_segments: usize,
    pub cached_segments: usize,
}

impl Translation {
    /// E-HMN-004, если ИИ не уверен.
    pub fn check_confidence(&self, min: f64) -> Result<(), Diagnostic> {
        if self.confidence + 1e-9 >= min {
            return Ok(());
        }
        let mut d = diag(
            "E-HMN-004",
            format!("ИИ не уверен, что понял текст правильно (уверенность {:.2} < {:.2})", self.confidence, min),
        );
        let hint = if self.questions.is_empty() {
            "уточните текст или запустите с --min-confidence 0, если перевод вас устраивает".to_string()
        } else {
            format!("уточните: {}", self.questions.join("; "))
        };
        d.hint = Some(hint);
        Err(d)
    }
}

fn diag(code: &str, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::error(code, msg, Span::new(0, 0, 0))
}

/// Кусок файла `.vlhmn`.
#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    /// Свободный текст (строка начала в файле).
    Prose(String, u32),
    /// Код `.vlstd` из блока ``` (строка начала кода в файле).
    Code(String, u32),
}

/// Разбить `.vlhmn` на прозу и блоки кода в тройных кавычках.
pub fn split_segments(text: &str) -> Vec<Segment> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut start = 1u32;
    let mut in_code = false;
    for (i, line) in text.lines().enumerate() {
        let n = i as u32 + 1;
        if line.trim_start().starts_with("```") {
            let piece = std::mem::take(&mut cur);
            if in_code {
                out.push(Segment::Code(piece, start));
            } else if !piece.trim().is_empty() {
                out.push(Segment::Prose(format!("{}\n", piece.trim_end()), start));
            }
            in_code = !in_code;
            start = n + 1;
            continue;
        }
        if cur.is_empty() && !in_code && line.trim().is_empty() {
            start = n + 1;
            continue;
        }
        cur.push_str(line);
        cur.push('\n');
    }
    if in_code {
        out.push(Segment::Code(cur, start));
    } else if !cur.trim().is_empty() {
        out.push(Segment::Prose(format!("{}\n", cur.trim_end()), start));
    }
    out
}

#[derive(Serialize, Deserialize)]
struct Reply {
    confidence: f64,
    #[serde(default)]
    questions: Vec<String>,
    program: serde_json::Value,
}

/// Вырезать JSON-объект из ответа модели (на случай ```json … ``` вокруг).
fn extract_json(s: &str) -> Option<&str> {
    let a = s.find('{')?;
    let b = s.rfind('}')?;
    (b > a).then(|| &s[a..=b])
}

fn cache_key(model: &str, lang: Lang, known: &[String], text: &str) -> String {
    let mut h = Sha256::new();
    for part in [prompt::PROMPT_VERSION, model, lang.code(), &known.join(","), text.trim()] {
        h.update(part.as_bytes());
        h.update([0u8]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Имена верхнего уровня, объявленные программой.
fn declared_names(body: &[Stmt]) -> Vec<String> {
    let mut v = vec![];
    for s in body {
        match s {
            Stmt::Declare { name, .. } | Stmt::FunctionDecl { name, .. } | Stmt::Ask { name, .. } => v.push(name.clone()),
            _ => {}
        }
    }
    v
}

/// Перевести один кусок прозы (с одним повтором при ошибке).
#[allow(clippy::too_many_arguments)]
fn translate_prose(
    text: &str,
    lang: Lang,
    dict: &Dictionary,
    globals: &[&str],
    known: &[String],
    llm: &mut dyn Llm,
    cache: &mut dyn Cache,
    calls: &mut u32,
) -> Result<(Program, f64, Vec<String>, bool), Vec<Diagnostic>> {
    let mut all_globals: Vec<&str> = globals.to_vec();
    all_globals.extend(known.iter().map(|s| s.as_str()));
    let key = cache_key(&llm.model(), lang, known, text);

    if let Some(hit) = cache.get(&key) {
        if let Ok(r) = serde_json::from_str::<Reply>(&hit) {
            if let Ok(c) = validate_json(&r.program.to_string(), dict, &all_globals) {
                return Ok((c.program, r.confidence, r.questions, true));
            }
        }
    }

    let mut messages = vec![
        Msg { role: "system", content: prompt::system_prompt(dict, lang) },
        Msg { role: "user", content: prompt::user_prompt(text, lang, known) },
    ];
    let mut last_problems: Vec<Diagnostic> = vec![];
    for attempt in 0..2 {
        *calls += 1;
        let answer = llm.chat(&messages).map_err(|e| vec![diag("E-HMN-001", format!("ИИ недоступен: {e}"))])?;
        let problems: Vec<Diagnostic> = match extract_json(&answer).map(serde_json::from_str::<Reply>) {
            None => vec![diag("E-HMN-002", "ответ ИИ не содержит JSON")],
            Some(Err(e)) => vec![diag("E-HMN-002", format!("ответ ИИ — не JSON нужного вида: {e}"))],
            Some(Ok(mut r)) => {
                if let Some(o) = r.program.as_object_mut() {
                    o.insert("lang".into(), lang.code().into());
                }
                match validate_json(&r.program.to_string(), dict, &all_globals) {
                    Ok(c) => {
                        let confidence = r.confidence.clamp(0.0, 1.0);
                        let stored = Reply {
                            confidence,
                            questions: r.questions.clone(),
                            program: vladx_ast::json_without_spans(&c.program),
                        };
                        cache.put(&key, &serde_json::to_string(&stored).unwrap_or_default());
                        return Ok((c.program, confidence, r.questions, false));
                    }
                    Err(ds) => ds,
                }
            }
        };
        last_problems = problems;
        if attempt == 0 {
            let list: Vec<String> = last_problems.iter().map(|d| format!("{}: {}", d.code, d.message)).collect();
            messages.push(Msg { role: "assistant", content: answer });
            messages.push(Msg { role: "user", content: prompt::repair_prompt(&list) });
        }
    }
    let mut out = vec![diag("E-HMN-003", "ИИ не смог дать корректную программу и после повтора")];
    out.extend(last_problems);
    Err(out)
}

/// Перевести файл `.vlhmn` целиком.
pub fn translate(
    text: &str,
    lang: Lang,
    dict: &Dictionary,
    globals: &[&str],
    llm: &mut dyn Llm,
    cache: &mut dyn Cache,
) -> Result<Translation, Vec<Diagnostic>> {
    let mut body: Vec<Stmt> = vec![];
    let mut known: Vec<String> = vec![];
    let mut confidence: f64 = 1.0;
    let mut questions = vec![];
    let (mut calls, mut prose, mut cached) = (0u32, 0usize, 0usize);

    for seg in split_segments(text) {
        match seg {
            Segment::Code(code, start) => {
                let p = vladx_syntax::parse(&code, lang, dict).map_err(|mut d| {
                    d.line += start - 1;
                    d.message = format!("в блоке кода: {}", d.message);
                    vec![d]
                })?;
                known.extend(declared_names(&p.body));
                body.extend(p.body);
            }
            Segment::Prose(t, _) => {
                prose += 1;
                let (p, c, q, hit) = translate_prose(&t, lang, dict, globals, &known, llm, cache, &mut calls)?;
                cached += hit as usize;
                confidence = confidence.min(c);
                questions.extend(q);
                known.extend(declared_names(&p.body));
                body.extend(p.body);
            }
        }
    }
    let program = Program::new(lang.code(), body);
    let checked = validate(program, dict, globals)?;
    Ok(Translation { checked, confidence, questions, llm_calls: calls, prose_segments: prose, cached_segments: cached })
}
