//! `vladx serve`: статика проекта + прокси `GET /fetch?url=…`.
//!
//! Причина существования одна: браузер не даёт worker и OPFS на `file://`,
//! а прокси нужен для сайтов без CORS.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

fn mime(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "md" | "txt" | "vlstd" | "vlhmn" | "toml" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = hex {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn respond(s: &mut TcpStream, status: &str, ctype: &str, body: &[u8], extra: &str) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store\r\nConnection: close\r\n{extra}\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

fn handle(mut s: TcpStream, root: PathBuf, files: PathBuf) {
    let mut reader = BufReader::new(match s.try_clone() {
        Ok(c) => c,
        Err(_) => return,
    });
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    // заголовки
    let mut content_length = 0usize;
    loop {
        let mut l = String::new();
        match reader.read_line(&mut l) {
            Ok(0) | Err(_) => break,
            Ok(_) if l == "\r\n" || l == "\n" => break,
            _ => {
                if let Some((k, v)) = l.split_once(':') {
                    if k.trim().eq_ignore_ascii_case("content-length") {
                        content_length = v.trim().parse().unwrap_or(0);
                    }
                }
            }
        }
    }
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    if method == "OPTIONS" {
        respond(
            &mut s,
            "204 No Content",
            "text/plain",
            b"",
            "Access-Control-Allow-Methods: GET, POST\r\nAccess-Control-Allow-Headers: *\r\n",
        );
        return;
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    // /llm и любой …/vladx-llm.php: та же площадка работает и здесь, и на PHP-хостинге.
    if path == "/llm" || path.ends_with("/vladx-llm.php") {
        // Тот же протокол, что у deploy/shared/vladx-llm.php: ключ и модель — у этого сервера.
        let reply = if method == "POST" {
            if content_length > 512 * 1024 {
                serde_json::json!({"ok": false, "error": "запрос слишком большой"})
            } else {
                let mut body = vec![0u8; content_length];
                if reader.read_exact(&mut body).is_err() {
                    return;
                }
                llm_reply(&body)
            }
        } else {
            let p = crate::llm::Polza::from_env();
            serde_json::json!({"available": p.is_ok(), "model": crate::llm::provider().model(), "reason": p.err()})
        };
        respond(&mut s, "200 OK", "application/json; charset=utf-8", reply.to_string().as_bytes(), "");
        return;
    }
    if path == "/fs" || path.ends_with("/vladx-fs.php") {
        // Файлы программ площадки — прямо на диске, в папке --files (запись сразу, как в Node).
        let reply = if method == "POST" {
            let mut body = vec![0u8; content_length.min(64 * 1024 * 1024)];
            if reader.read_exact(&mut body).is_err() {
                return;
            }
            fs_reply(&body, &files)
        } else {
            serde_json::json!({"available": true, "kind": "disk", "dir": files.display().to_string()})
        };
        respond(&mut s, "200 OK", "application/json; charset=utf-8", reply.to_string().as_bytes(), "");
        return;
    }
    if method != "GET" && method != "HEAD" {
        respond(&mut s, "405 Method Not Allowed", "text/plain; charset=utf-8", "только GET (и POST /llm)".as_bytes(), "");
        return;
    }
    if path == "/fetch" || path.ends_with("/vladx-fetch.php") {
        let url = query.split('&').find_map(|kv| kv.strip_prefix("url=")).map(percent_decode);
        let Some(url) = url else {
            // проверка доступности (тот же ответ, что у vladx-fetch.php)
            respond(&mut s, "200 OK", "application/json; charset=utf-8", br#"{"available":true}"#, "");
            return;
        };
        let err_h = "X-VladX-Error: 1\r\n";
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            respond(&mut s, "400 Bad Request", "text/plain; charset=utf-8", "адрес должен начинаться с http:// или https://".as_bytes(), err_h);
            return;
        }
        match crate::host::agent_for(&url, 20).get(&url).call() {
            Ok(r) => {
                let ct = r.content_type().to_string();
                let mut body = vec![];
                let _ = r.into_reader().take(50 * 1024 * 1024).read_to_end(&mut body);
                respond(&mut s, "200 OK", &ct, &body, "");
            }
            Err(ureq::Error::Status(code, r)) => {
                let body = r.into_string().unwrap_or_default();
                respond(&mut s, &format!("{code} Upstream"), "text/plain; charset=utf-8", body.as_bytes(), "");
            }
            Err(e) => respond(&mut s, "502 Bad Gateway", "text/plain; charset=utf-8", format!("запрос не удался: {e}").as_bytes(), err_h),
        }
        return;
    }
    if path == "/" {
        respond(&mut s, "302 Found", "text/plain", b"", "Location: /site/playground.html\r\n");
        return;
    }
    let rel = percent_decode(path.trim_start_matches('/'));
    if rel.split('/').any(|seg| seg == "..") {
        respond(&mut s, "403 Forbidden", "text/plain; charset=utf-8", "запрещено".as_bytes(), "");
        return;
    }
    let file = root.join(&rel);
    match std::fs::read(&file) {
        Ok(body) if file.is_file() => {
            // Изоляция страницы: «спроси» может по-настоящему ждать ответа (SharedArrayBuffer).
            let extra = if mime(&file).starts_with("text/html") {
                "Cross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Embedder-Policy: require-corp\r\n"
            } else {
                ""
            };
            respond(&mut s, "200 OK", mime(&file), if method == "HEAD" { &[] } else { &body }, extra)
        }
        _ => respond(&mut s, "404 Not Found", "text/plain; charset=utf-8", format!("нет файла «{rel}»").as_bytes(), ""),
    }
}

/// POST /llm {messages} → {ok, content}. Принимается только диалог перевода VladX.
fn llm_reply(body: &[u8]) -> serde_json::Value {
    use vladx_hmn::{Llm, Msg};
    let req: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return serde_json::json!({"ok": false, "error": format!("запрос не JSON: {e}")}),
    };
    let mut msgs = vec![];
    for m in req["messages"].as_array().cloned().unwrap_or_default() {
        let role = match m["role"].as_str() {
            Some("system") => "system",
            Some("user") => "user",
            Some("assistant") => "assistant",
            _ => return serde_json::json!({"ok": false, "error": "неизвестная роль сообщения"}),
        };
        msgs.push(Msg { role, content: m["content"].as_str().unwrap_or("").to_string() });
    }
    let vladx_dialog = (2..=4).contains(&msgs.len())
        && msgs[0].role == "system"
        && msgs[0].content.contains("в программу на языке VladX");
    if !vladx_dialog {
        return serde_json::json!({"ok": false, "error": "этот сервер переводит только .vlhmn для VladX"});
    }
    let mut p = match crate::llm::Polza::from_env() {
        Ok(p) => p,
        Err(e) => return serde_json::json!({"ok": false, "error": e}),
    };
    match p.chat(&msgs) {
        Ok(content) => serde_json::json!({"ok": true, "content": content, "model": p.model()}),
        Err(e) => serde_json::json!({"ok": false, "error": e}),
    }
}

/// POST /fs {op, path, data} → {ok, …}: тот же протокол, что у deploy/shared/vladx-fs.php.
fn fs_reply(body: &[u8], files: &Path) -> serde_json::Value {
    use vladx_runtime::Host;
    let req: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return serde_json::json!({"ok": false, "error": format!("запрос не JSON: {e}")}),
    };
    let _ = std::fs::create_dir_all(files);
    let mut host = crate::host::CliHost { base: files.to_path_buf(), restrict: true, deadline: None, stream: false };
    let path = req["path"].as_str().unwrap_or("");
    if path.split(['/', '\\']).any(|seg| seg.starts_with('.') && !seg.is_empty() && seg != ".") {
        return serde_json::json!({"ok": false, "error": format!("недопустимое имя «{path}»")});
    }
    let r = match req["op"].as_str().unwrap_or("") {
        "list" => host.list().map(|f| serde_json::json!({"ok": true, "files": f})),
        "read" => host.read(path).map(|d| serde_json::json!({"ok": true, "data": d})),
        "write" => host.write(path, req["data"].as_str().unwrap_or("")).map(|_| serde_json::json!({"ok": true})),
        "delete" => host.delete(path).map(|_| serde_json::json!({"ok": true})),
        "stat" => host.list().map(|f| serde_json::json!({"ok": true, "files": f.len()})),
        other => Err(format!("неизвестная операция «{other}»")),
    };
    r.unwrap_or_else(|e| serde_json::json!({"ok": false, "error": e}))
}

pub fn serve(port: u16, root: PathBuf, files: PathBuf) -> std::io::Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    println!("vladx serve: папка {}", root.display());
    println!("  площадка: http://localhost:{port}/site/playground.html");
    println!("  прокси:   http://localhost:{port}/fetch?url=https://example.com");
    println!("  файлы программ: {} (пишутся сразу, как в Node)", files.display());
    match crate::llm::Polza::from_env() {
        Ok(_) => println!("  ИИ (.vlhmn): http://localhost:{port}/llm, модель {}", crate::llm::provider().model()),
        Err(e) => println!("  ИИ (.vlhmn) выключен: {e}"),
    }
    for stream in listener.incoming().flatten() {
        let (root, files) = (root.clone(), files.clone());
        let _ = std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(move || handle(stream, root, files));
    }
    Ok(())
}
