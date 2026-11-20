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

fn handle(mut s: TcpStream, root: PathBuf) {
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
    if path == "/hmn" {
        let reply = if method == "POST" {
            if content_length > 256 * 1024 {
                serde_json::json!({"ok": false, "diagnostics": [{"code": "E-HMN-001", "message": "текст слишком длинный (больше 256 КБ)"}]})
            } else {
                let mut body = vec![0u8; content_length];
                if reader.read_exact(&mut body).is_err() {
                    return;
                }
                hmn_reply(&body, &root)
            }
        } else {
            // GET /hmn — есть ли ИИ на этом сервере
            let p = crate::llm::Polza::from_env();
            serde_json::json!({"available": p.is_ok(), "model": crate::llm::provider().model(), "reason": p.err()})
        };
        respond(&mut s, "200 OK", "application/json; charset=utf-8", reply.to_string().as_bytes(), "");
        return;
    }
    if method != "GET" && method != "HEAD" {
        respond(&mut s, "405 Method Not Allowed", "text/plain; charset=utf-8", "только GET (и POST /hmn)".as_bytes(), "");
        return;
    }
    if path == "/fetch" {
        let url = query
            .split('&')
            .find_map(|kv| kv.strip_prefix("url="))
            .map(percent_decode)
            .unwrap_or_default();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            respond(&mut s, "400 Bad Request", "text/plain; charset=utf-8", "адрес должен начинаться с http:// или https://".as_bytes(), "");
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
            Err(e) => respond(&mut s, "502 Bad Gateway", "text/plain; charset=utf-8", format!("запрос не удался: {e}").as_bytes(), ""),
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
        Ok(body) if file.is_file() => respond(&mut s, "200 OK", mime(&file), if method == "HEAD" { &[] } else { &body }, ""),
        _ => respond(&mut s, "404 Not Found", "text/plain; charset=utf-8", format!("нет файла «{rel}»").as_bytes(), ""),
    }
}

fn diag_json(d: &vladx_ast::Diagnostic) -> serde_json::Value {
    serde_json::json!({"code": d.code, "message": d.message, "line": d.line, "col": d.col, "len": d.len, "hint": d.hint})
}

/// POST /hmn {text, lang} → перевод `.vlhmn` ключом сервера (браузеру ключ не нужен).
fn hmn_reply(body: &[u8], root: &Path) -> serde_json::Value {
    let req: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return serde_json::json!({"ok": false, "diagnostics": [{"code": "E-HMN-002", "message": format!("запрос не JSON: {e}")}]}),
    };
    let text = req["text"].as_str().unwrap_or("");
    let lang = req["lang"].as_str().and_then(vladx_syntax::Lang::from_code).unwrap_or(vladx_syntax::Lang::Ru);
    let dict = vladx_syntax::Dictionary::builtin();
    let mut provider = crate::llm::provider();
    let mut cache = crate::llm::FileCache { dir: root.join(".vladx-cache").join("hmn") };
    match vladx_hmn::translate(text, lang, &dict, vladx_runtime::GLOBAL_NAMES, provider.as_mut(), &mut cache) {
        Ok(t) => serde_json::json!({
            "ok": true,
            "program": vladx_ast::json_without_spans(&t.checked.program),
            "vlstd": t.checked.text,
            "confidence": t.confidence,
            "questions": t.questions,
            "llm_calls": t.llm_calls,
            "cached": t.llm_calls == 0 && t.prose_segments > 0,
            "model": provider.model(),
        }),
        Err(ds) => serde_json::json!({"ok": false, "diagnostics": ds.iter().map(diag_json).collect::<Vec<_>>()}),
    }
}

pub fn serve(port: u16, root: PathBuf) -> std::io::Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    println!("vladx serve: папка {}", root.display());
    println!("  площадка: http://localhost:{port}/site/playground.html");
    println!("  прокси:   http://localhost:{port}/fetch?url=https://example.com");
    match crate::llm::Polza::from_env() {
        Ok(_) => println!("  ИИ (.vlhmn): POST http://localhost:{port}/hmn, модель {}", crate::llm::provider().model()),
        Err(e) => println!("  ИИ (.vlhmn) выключен: {e}"),
    }
    for stream in listener.incoming().flatten() {
        let root = root.clone();
        let _ = std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(move || handle(stream, root));
    }
    Ok(())
}
