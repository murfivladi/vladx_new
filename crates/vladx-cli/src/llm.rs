//! Провайдер LLM для `.vlhmn`: Polza.ai (OpenAI-совместимый API) и кэш на диске.
//!
//! Настройка через окружение:
//!   VLADX_LLM_KEY | POLZA_API_KEY | POLZA — ключ (Authorization: Bearer …)
//!   VLADX_LLM_MODEL  — модель (по умолчанию anthropic/claude-haiku-5.5: дешёвая модель для разбора, §15.1)
//!   VLADX_LLM_URL    — адрес (по умолчанию https://polza.ai/api/v1/chat/completions)
//!   VLADX_LLM_EFFORT — reasoning_effort (по умолчанию low; пусто — не передавать)

use std::path::PathBuf;
use vladx_hmn::{Cache, Llm, Msg};

pub const DEFAULT_URL: &str = "https://polza.ai/api/v1/chat/completions";
pub const DEFAULT_MODEL: &str = "anthropic/claude-haiku-5.5";

pub struct Polza {
    url: String,
    key: String,
    model: String,
    effort: Option<String>,
    agent: ureq::Agent,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

impl Polza {
    pub fn from_env() -> Result<Polza, String> {
        let key = env("VLADX_LLM_KEY").or_else(|| env("POLZA_API_KEY")).or_else(|| env("POLZA")).ok_or_else(|| {
            "нет ключа LLM: задайте POLZA_API_KEY (ключ создаётся на https://polza.ai/dashboard/api-keys)".to_string()
        })?;
        let effort = match std::env::var("VLADX_LLM_EFFORT") {
            Ok(v) if v.trim().is_empty() => None,
            Ok(v) => Some(v),
            Err(_) => Some("low".to_string()),
        };
        let url = env("VLADX_LLM_URL").unwrap_or_else(|| DEFAULT_URL.into());
        Ok(Polza {
            agent: crate::host::agent_for(&url, 180),
            url,
            key,
            model: env("VLADX_LLM_MODEL").unwrap_or_else(|| DEFAULT_MODEL.into()),
            effort,
        })
    }
}

impl Llm for Polza {
    fn model(&self) -> String {
        self.model.clone()
    }

    fn chat(&mut self, messages: &[Msg]) -> Result<String, String> {
        let msgs: Vec<serde_json::Value> =
            messages.iter().map(|m| serde_json::json!({"role": m.role, "content": m.content})).collect();
        let mut body = serde_json::json!({"model": self.model, "messages": msgs, "max_tokens": 16000});
        if let Some(e) = &self.effort {
            body["reasoning_effort"] = e.clone().into();
        }
        let resp = self
            .agent
            .post(&self.url)
            .set("Authorization", &format!("Bearer {}", self.key))
            .set("Content-Type", "application/json")
            .send_string(&body.to_string());
        let text = match resp {
            Ok(r) => r.into_string().map_err(|e| format!("ответ не прочитан: {e}"))?,
            Err(ureq::Error::Status(code, r)) => {
                let raw = r.into_string().unwrap_or_default();
                let msg = serde_json::from_str::<serde_json::Value>(&raw)
                    .ok()
                    .and_then(|j| j["error"]["message"].as_str().map(|s| s.to_string()))
                    .unwrap_or(raw);
                return Err(match code {
                    401 => format!("ключ не принят (401): {msg}"),
                    402 => format!("недостаточно средств на балансе (402): {msg}"),
                    429 => format!("слишком много запросов (429): {msg}"),
                    _ => format!("сервер ответил {code}: {msg}"),
                });
            }
            Err(e) => return Err(format!("запрос не удался: {e}")),
        };
        let j: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("ответ провайдера не JSON: {e}"))?;
        let choice = &j["choices"][0];
        match choice["finish_reason"].as_str() {
            Some("length") => return Err("ответ модели обрезан по длине".into()),
            Some("content_filter") => return Err("модель отказалась отвечать (content_filter)".into()),
            Some("error") => return Err(format!("ошибка провайдера: {}", j["error"])),
            _ => {}
        }
        choice["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "в ответе провайдера нет текста".to_string())
    }
}

/// Кэш переводов на диске: `<папка>/.vladx-cache/hmn/<хеш>.json`.
pub struct FileCache {
    pub dir: PathBuf,
}

impl Cache for FileCache {
    fn get(&mut self, key: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.join(format!("{key}.json"))).ok()
    }
    fn put(&mut self, key: &str, value: &str) {
        if std::fs::create_dir_all(&self.dir).is_ok() {
            let _ = std::fs::write(self.dir.join(format!("{key}.json")), value);
        }
    }
}

/// LLM, которой нет: первый же запрос — понятная ошибка (кэш при этом работает).
pub struct Unavailable(pub String);
impl Llm for Unavailable {
    fn model(&self) -> String {
        env("VLADX_LLM_MODEL").unwrap_or_else(|| DEFAULT_MODEL.into())
    }
    fn chat(&mut self, _: &[Msg]) -> Result<String, String> {
        Err(self.0.clone())
    }
}

/// Провайдер из окружения или «недоступный» с объяснением.
pub fn provider() -> Box<dyn Llm> {
    match Polza::from_env() {
        Ok(p) => Box::new(p),
        Err(e) => Box::new(Unavailable(e)),
    }
}
