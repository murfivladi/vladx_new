//! vladx — интерпретатор VladX (v0.1).

mod host;
mod serve;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;
use vladx_runtime::{Options, GLOBAL_NAMES};
use vladx_syntax::{Dictionary, Lang};

const USAGE: &str = "vladx — интерпретатор VladX (v0.1)

Использование:
  vladx run <файл> [--lang ru|it|scn] [--no-net] [--no-files] [--restrict-files]   выполнить программу
  vladx parse <файл> [--lang ru|it|scn] [--keywords <путь>]   печать AST в JSON
  vladx check <файл> [--lang ru|it|scn]                       только диагностика
  vladx keywords [--lang ru|it|scn]                           словарь форм
  vladx serve [--port 8000] [--root <папка>]                  площадка и прокси

  по умолчанию открыто всё (как в Node): любые файлы и сеть;
  сузить: --no-files, --no-net, --restrict-files (только папка программы)
  лимиты: --max-steps N (10000000), --max-seconds N (10, 0 — без лимита)";

struct Args {
    cmd: String,
    file: Option<String>,
    lang: Option<Lang>,
    keywords: Option<String>,
    no_net: bool,
    no_files: bool,
    restrict: bool,
    port: u16,
    root: Option<String>,
    max_steps: u64,
    max_seconds: u64,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().ok_or_else(|| USAGE.to_string())?;
    let mut a = Args {
        cmd,
        file: None,
        lang: None,
        keywords: None,
        no_net: false,
        no_files: false,
        restrict: false,
        port: 8000,
        root: None,
        max_steps: 10_000_000,
        max_seconds: 10,
    };
    while let Some(x) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("после {name} нужно значение"));
        match x.as_str() {
            "--lang" => {
                let v = val("--lang")?;
                a.lang = Some(Lang::from_code(&v).ok_or_else(|| format!("неизвестный язык «{v}»: ru, it или scn"))?);
            }
            "--keywords" => a.keywords = Some(val("--keywords")?),
            "--no-net" => a.no_net = true,
            "--no-files" => a.no_files = true,
            "--restrict-files" => a.restrict = true,
            "--net" => {} // устаревший флаг: сеть и так открыта
            "--port" => a.port = val("--port")?.parse().map_err(|_| "порт — число".to_string())?,
            "--root" => a.root = Some(val("--root")?),
            "--max-steps" => a.max_steps = val("--max-steps")?.parse().map_err(|_| "--max-steps — число".to_string())?,
            "--max-seconds" => a.max_seconds = val("--max-seconds")?.parse().map_err(|_| "--max-seconds — число".to_string())?,
            "-h" | "--help" => return Err(USAGE.to_string()),
            s if s.starts_with("--") => return Err(format!("неизвестный флаг {s}\n\n{USAGE}")),
            s => {
                if a.file.is_some() {
                    return Err(format!("лишний аргумент «{s}»"));
                }
                a.file = Some(s.to_string());
            }
        }
    }
    Ok(a)
}

fn load_dict(a: &Args) -> Result<Dictionary, String> {
    match &a.keywords {
        None => Ok(Dictionary::builtin()),
        Some(p) => {
            let s = std::fs::read_to_string(p).map_err(|e| format!("словарь «{p}» не прочитан: {e}"))?;
            Dictionary::from_toml(&s).map_err(|e| format!("E-DICT-001  {e}"))
        }
    }
}

fn load_source(file: &str) -> Result<String, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("файл «{file}» не прочитан: {e}"))?;
    vladx_syntax::decode_source(&bytes).map_err(|d| d.render(file, ""))
}

fn main() -> ExitCode {
    let a = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let res = match a.cmd.as_str() {
        "run" | "parse" | "check" => {
            // Интерпретатор рекурсивный: исполняем в потоке с большим стеком.
            let h = std::thread::Builder::new().stack_size(512 * 1024 * 1024).spawn(move || program_cmd(a));
            match h {
                Ok(h) => h.join().unwrap_or(Err("внутренняя ошибка интерпретатора".into())),
                Err(e) => Err(format!("не удалось запустить поток: {e}")),
            }
        }
        "keywords" => keywords_cmd(&a),
        "serve" => {
            let root = a.root.clone().map(PathBuf::from).unwrap_or_else(|| std::env::current_dir().unwrap());
            serve::serve(a.port, root).map(|_| true).map_err(|e| format!("сервер не запущен: {e}"))
        }
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            Ok(true)
        }
        "--version" | "version" => {
            println!("vladx 0.1.0 (схема AST {})", vladx_ast::SCHEMA_VERSION);
            Ok(true)
        }
        other => Err(format!("неизвестная команда «{other}»\n\n{USAGE}")),
    };
    match res {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn program_cmd(a: Args) -> Result<bool, String> {
    let file = a.file.clone().ok_or_else(|| format!("нужен файл программы\n\n{USAGE}"))?;
    let dict = load_dict(&a)?;
    let lang = a.lang.unwrap_or(dict.default_lang);
    let src = load_source(&file)?;
    match a.cmd.as_str() {
        "parse" => match vladx_syntax::parse(&src, lang, &dict) {
            Ok(p) => {
                println!("{}", p.to_json_pretty());
                Ok(true)
            }
            Err(d) => {
                eprint!("{}", d.render(&file, &src));
                Ok(false)
            }
        },
        "check" => {
            let (_, diags) = vladx_syntax::analyze(&src, lang, &dict, GLOBAL_NAMES);
            for d in &diags {
                eprint!("{}", d.render(&file, &src));
            }
            let errors = diags.iter().filter(|d| d.is_error()).count();
            if errors == 0 {
                println!("{file}: ошибок нет");
            } else {
                eprintln!("{file}: ошибок — {errors}");
            }
            Ok(errors == 0)
        }
        _ => {
            let base = Path::new(&file).parent().map(|p| p.to_path_buf()).unwrap_or_default();
            let base = if base.as_os_str().is_empty() { PathBuf::from(".") } else { base };
            let base = std::fs::canonicalize(&base).unwrap_or(base);
            let mut host = host::CliHost {
                base,
                restrict: a.restrict,
                deadline: if a.max_seconds > 0 { Some((Instant::now(), a.max_seconds)) } else { None },
                stream: true,
            };
            let opts = Options { max_steps: a.max_steps, files: !a.no_files, net: !a.no_net, ..Options::default() };
            let r = vladx_runtime::run_source(&src, lang, &dict, &mut host, opts);
            for d in &r.diagnostics {
                eprint!("{}", d.render(&file, &src));
            }
            Ok(r.ok)
        }
    }
}

fn keywords_cmd(a: &Args) -> Result<bool, String> {
    let dict = load_dict(a)?;
    let langs: Vec<Lang> = match a.lang {
        Some(l) => vec![l],
        None => Lang::ALL.to_vec(),
    };
    for l in langs {
        println!("[{}]", l.code());
        for (id, form) in dict.all_forms(l) {
            println!("  {id:<26} {form}");
        }
    }
    Ok(true)
}
