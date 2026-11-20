//! Хост CLI: настоящий диск (std::fs) и настоящий HTTPS (ureq + rustls).
//! По умолчанию открыто всё, как в Node: любые пути и сеть без флагов.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use vladx_runtime::Host;

pub struct CliHost {
    /// Папка программы: от неё считаются относительные пути.
    pub base: PathBuf,
    /// --restrict-files: только папка программы.
    pub restrict: bool,
    pub deadline: Option<(Instant, u64)>,
    pub stream: bool,
}

/// Нормализация пути без обращения к диску (`a/./b/../c` → `a/c`).
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

impl CliHost {
    fn resolve(&self, path: &str) -> Result<PathBuf, String> {
        let p = Path::new(path);
        let full = if p.is_absolute() { p.to_path_buf() } else { self.base.join(p) };
        let full = normalize(&full);
        if self.restrict {
            let base = normalize(&self.base);
            if !full.starts_with(&base) {
                return Err(format!(
                    "путь «{path}» вне папки программы ({}): запуск с --restrict-files разрешает только её",
                    base.display()
                ));
            }
        }
        Ok(full)
    }
}

/// Хост из URL без порта: `https://a.b:8000/x` → `a.b`.
fn url_host(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    if let Some(v6) = authority.strip_prefix('[') {
        return v6.split(']').next().unwrap_or("").to_lowercase();
    }
    authority.split(':').next().unwrap_or("").to_lowercase()
}

/// Обходит ли хост прокси по NO_PROXY (ureq 2 сам эту переменную не читает).
fn bypass_proxy(host: &str) -> bool {
    if host == "localhost" || host == "::1" || host.starts_with("127.") {
        return true;
    }
    let list = std::env::var("NO_PROXY").or_else(|_| std::env::var("no_proxy")).unwrap_or_default();
    list.split(',').map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).any(|e| {
        if e == "*" {
            return true;
        }
        let e = e.trim_start_matches("*.").trim_start_matches('.');
        host == e || host.ends_with(&format!(".{e}"))
    })
}

/// HTTP-клиент для адреса: прокси окружения, если хост не в NO_PROXY.
pub fn agent_for(url: &str, timeout_secs: u64) -> ureq::Agent {
    let b = ureq::AgentBuilder::new().timeout(Duration::from_secs(timeout_secs)).user_agent("vladx/0.1");
    let b = if bypass_proxy(&url_host(url)) { b } else { b.try_proxy_from_env(true) };
    b.build()
}

pub fn http_get(url: &str) -> Result<String, String> {
    match agent_for(url, 20).get(url).call() {
        Ok(r) => r.into_string().map_err(|e| format!("запрос не удался: {e}")),
        Err(ureq::Error::Status(code, _)) => Err(format!("сервер ответил {code}")),
        Err(e) => Err(format!("запрос не удался: {e}")),
    }
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let path = e.path();
        if path.is_dir() {
            if matches!(name.as_str(), "target" | ".git" | "node_modules") {
                continue;
            }
            walk(&path, root, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/"));
        }
    }
}

impl Host for CliHost {
    fn read(&mut self, path: &str) -> Result<String, String> {
        let p = self.resolve(path)?;
        std::fs::read_to_string(&p).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("файл «{path}» не найден ({})", p.display()),
            _ => format!("файл «{path}» не прочитан: {e}"),
        })
    }
    fn write(&mut self, path: &str, data: &str) -> Result<(), String> {
        let p = self.resolve(path)?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("папка для «{path}» не создана: {e}"))?;
        }
        std::fs::write(&p, data).map_err(|e| format!("файл «{path}» не записан: {e}"))
    }
    fn delete(&mut self, path: &str) -> Result<(), String> {
        let p = self.resolve(path)?;
        std::fs::remove_file(&p).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("файл «{path}» не найден"),
            _ => format!("файл «{path}» не удалён: {e}"),
        })
    }
    fn list(&mut self) -> Result<Vec<String>, String> {
        let mut out = vec![];
        walk(&self.base, &self.base, &mut out);
        out.sort();
        Ok(out)
    }
    fn get(&mut self, url: &str) -> Result<String, String> {
        http_get(url)
    }
    fn input(&mut self, prompt: &str) -> Option<String> {
        print!("{prompt} ");
        std::io::stdout().flush().ok();
        let mut s = String::new();
        match std::io::stdin().lock().read_line(&mut s) {
            Ok(0) | Err(_) => None,
            Ok(_) => {
                let answer = s.trim_end_matches(['\r', '\n']).to_string();
                if !std::io::stdin().is_terminal() {
                    println!("{answer}"); // ответ пришёл не с клавиатуры — показать его
                }
                Some(answer)
            }
        }
    }
    fn say(&mut self, line: &str) {
        if self.stream {
            println!("{line}");
        }
    }
    fn warn(&mut self, line: &str) {
        if self.stream {
            eprintln!("предупреждение {line}");
        }
    }
    fn now_ms(&mut self) -> f64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
    }
    fn should_stop(&mut self) -> Option<String> {
        let (start, secs) = self.deadline?;
        if start.elapsed().as_secs() >= secs {
            Some(format!("Превышено время исполнения ({secs} с): программа мягко остановлена"))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hosts_and_no_proxy() {
        assert_eq!(url_host("https://user@Polza.ai:443/api?x"), "polza.ai");
        assert_eq!(url_host("http://[::1]:8000/"), "::1");
        assert!(bypass_proxy("127.0.0.1") && bypass_proxy("localhost"));
    }
}
