//! Провайдер LLM для `.vlhmn` и кэш на диске.
//!
//! Два пути:
//! 1. Посредник (рекомендуется): `VLADX_LLM_PROXY=https://сайт/vladx-llm.php` — ключ и модель
//!    живут на сервере (deploy/shared/vladx-llm.php или `vladx serve` → /llm), клиенту ключ не нужен.
//!    `VLADX_LLM_PROXY_TOKEN` — токен, если сервер его требует.
//! 2. Напрямую в Polza.ai (OpenAI-совместимый API):
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

/// Посредник с ключом на сервере: протокол deploy/shared/vladx-llm.php.
pub struct Proxy {
    url: String,
    token: Option<String>,
    model: String,
    agent: ureq::Agent,
}

impl Proxy {
    pub fn from_env() -> Option<Result<Proxy, String>> {
        let url = env("VLADX_LLM_PROXY")?;
        Some(Proxy::connect(&url, env("VLADX_LLM_PROXY_TOKEN")))
    }

    /// GET у посредника: доступен ли ИИ и какая модель (модель входит в ключ кэша).
    pub fn connect(url: &str, token: Option<String>) -> Result<Proxy, String> {
        let agent = crate::host::agent_for(url, 180);
        let text = agent
            .get(url)
            .call()
            .map_err(|e| format!("сервер ИИ {url} недоступен: {e}"))?
            .into_string()
            .map_err(|e| format!("ответ сервера ИИ {url} не прочитан: {e}"))?;
        let info: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("сервер ИИ {url} ответил не JSON: {e}"))?;
        if info["available"] != serde_json::Value::Bool(true) {
            let why = info["reason"].as_str().unwrap_or("ИИ выключен");
            return Err(format!("сервер ИИ {url}: {why}"));
        }
        let model = info["model"].as_str().unwrap_or("?").to_string();
        Ok(Proxy { url: url.to_string(), token, model, agent })
    }
}

impl Llm for Proxy {
    fn model(&self) -> String {
        self.model.clone()
    }

    fn chat(&mut self, messages: &[Msg]) -> Result<String, String> {
        let msgs: Vec<serde_json::Value> =
            messages.iter().map(|m| serde_json::json!({"role": m.role, "content": m.content})).collect();
        let mut req = self.agent.post(&self.url).set("Content-Type", "application/json");
        if let Some(t) = &self.token {
            req = req.set("X-VladX-Token", t);
        }
        let body = serde_json::json!({ "messages": msgs }).to_string();
        let text = match req.send_string(&body) {
            Ok(r) => r.into_string(),
            Err(ureq::Error::Status(_, r)) => r.into_string(),
            Err(e) => return Err(format!("сервер ИИ недоступен: {e}")),
        }
        .map_err(|e| format!("ответ сервера ИИ не прочитан: {e}"))?;
        let j: serde_json::Value = serde_json::from_str(&text).map_err(|_| format!("сервер ИИ ответил не JSON: {}", text.chars().take(200).collect::<String>()))?;
        if j["ok"] == serde_json::Value::Bool(true) {
            j["content"].as_str().map(|s| s.to_string()).ok_or_else(|| "в ответе сервера ИИ нет текста".into())
        } else {
            Err(j["error"].as_str().unwrap_or("сервер ИИ вернул ошибку").to_string())
        }
    }
}

/// Провайдер из окружения: посредник (VLADX_LLM_PROXY), иначе Polza напрямую,
/// иначе «недоступный» с объяснением.
pub fn provider() -> Box<dyn Llm> {
    if let Some(p) = Proxy::from_env() {
        return match p {
            Ok(p) => Box::new(p),
            Err(e) => Box::new(Unavailable(e)),
        };
    }
    match Polza::from_env() {
        Ok(p) => Box::new(p),
        Err(e) => Box::new(Unavailable(format!(
            "{e}; или укажите сервер с ключом: VLADX_LLM_PROXY=https://ваш-сайт/vladx-llm.php"
        ))),
    }
}
