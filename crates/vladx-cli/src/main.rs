//! vladx — интерпретатор VladX (v0.1).

mod hmncmd;
mod host;
mod llm;
mod project;
mod serve;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;
use vladx_runtime::{Options, GLOBAL_NAMES};
use vladx_syntax::{Dictionary, Lang};

const USAGE: &str = "vladx — интерпретатор VladX (v0.1)

Использование:
  vladx run [файл] [--lang ru|it|scn] [--no-net] [--no-files] [--restrict-files]   выполнить программу
                                                              (без файла — main из vladx.toml)
  vladx init [папка] [--lang ru|it|scn]                       новый проект: vladx.toml, точка входа, модуль
  vladx parse <файл> [--lang ru|it|scn] [--keywords <путь>]   печать AST в JSON
  vladx check <файл> [--lang ru|it|scn]                       только диагностика
  vladx fmt <файл> [--lang …] [--to ru|it|scn] [--write]       канонический вид / перевод на другой язык
  vladx hmn-eval <папка> [--lang …]                           проверка перевода .vlhmn на наборе фраз
  vladx keywords [--lang ru|it|scn]                           словарь форм
  vladx serve [--port 8000] [--root <папка>] [--files <папка>] площадка, сеть, файлы и ИИ для неё

  по умолчанию открыто всё (как в Node): любые файлы и сеть;
  сузить: --no-files, --no-net, --restrict-files (только папка программы)
  лимиты: --max-steps N (10000000), --max-seconds N (10, 0 — без лимита)

  .vlhmn (свободный текст → ИИ → AST): ключ POLZA_API_KEY, модель VLADX_LLM_MODEL;
  --min-confidence X (0.8) — ниже порога программа не запускается; --show — показать перевод";

pub struct Args {
    pub cmd: String,
    pub file: Option<String>,
    pub lang: Option<Lang>,
    pub keywords: Option<String>,
    pub no_net: bool,
    pub no_files: bool,
    pub restrict: bool,
    pub port: u16,
    pub root: Option<String>,
    pub max_steps: u64,
    pub max_seconds: u64,
    pub to: Option<Lang>,
    pub write: bool,
    pub min_confidence: Option<f64>,
    pub show: bool,
    pub files: Option<String>,
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
        to: None,
        write: false,
        min_confidence: None,
        show: false,
        files: None,
    };
    while let Some(x) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("после {name} нужно значение"));
        match x.as_str() {
            "--lang" => {
                let v = val("--lang")?;
                a.lang = Some(Lang::from_code(&v).ok_or_else(|| format!("неизвестный язык «{v}»: ru, it или scn"))?);
            }
            "--keywords" => a.keywords = Some(val("--keywords")?),
            "--to" => {
                let v = val("--to")?;
                a.to = Some(Lang::from_code(&v).ok_or_else(|| format!("неизвестный язык «{v}»: ru, it или scn"))?);
            }
            "--write" => a.write = true,
            "--show" => a.show = true,
            "--min-confidence" => {
                a.min_confidence = Some(val("--min-confidence")?.parse().map_err(|_| "--min-confidence — число от 0 до 1".to_string())?)
            }
            "--no-net" => a.no_net = true,
            "--no-files" => a.no_files = true,
            "--restrict-files" => a.restrict = true,
            "--net" => {} // устаревший флаг: сеть и так открыта
            "--port" => a.port = val("--port")?.parse().map_err(|_| "порт — число".to_string())?,
            "--root" => a.root = Some(val("--root")?),
            "--files" => a.files = Some(val("--files")?),
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
        "init" => init_cmd(&a),
        "run" | "parse" | "check" | "fmt" | "hmn-eval" => {
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
            let files = a.files.clone().map(PathBuf::from).unwrap_or_else(|| root.join("vladx-files"));
            let files = std::fs::canonicalize(&files).unwrap_or(files);
            serve::serve(a.port, root, files).map(|_| true).map_err(|e| format!("сервер не запущен: {e}"))
        }
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            Ok(true)
        }
        "--version" | "-V" | "version" => {
            let commit = option_env!("VLADX_COMMIT").unwrap_or("локальная сборка");
            println!("vladx {} ({commit}, схема AST {})", env!("CARGO_PKG_VERSION"), vladx_ast::SCHEMA_VERSION);
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
    let file = match a.file.clone() {
        Some(f) => f,
        // Без файла — точка входа проекта (как `node .` с main в package.json).
        None => match project::find(Path::new(".")) {
            Some(Ok(p)) => match &p.main {
                Some(m) => p.root.join(m).display().to_string(),
                None => return Err(format!("в {} нет main — укажите файл программы", project::FILE)),
            },
            Some(Err(e)) => return Err(e),
            None => return Err(format!("нужен файл или папка (или {} с main)\n\n{USAGE}", project::FILE)),
        },
    };
    let dict = load_dict(&a)?;
    let proj = if a.cmd == "hmn-eval" { None } else { project::find(Path::new(&file).parent().unwrap_or(Path::new("."))).transpose()? };
    let lang = a.lang.or(proj.as_ref().and_then(|p| p.lang)).unwrap_or(dict.default_lang);
    if a.cmd == "hmn-eval" {
        return hmncmd::eval_cmd(&a, &file, &dict, lang);
    }
    let src = load_source(&file)?;
    if file.ends_with(".vlhmn") {
        return hmncmd::hmn_cmd(&a, &file, &src, &dict, lang);
    }
    match a.cmd.as_str() {
        "fmt" => {
            let to = a.to.unwrap_or(lang);
            match vladx_syntax::format_source(&src, lang, to, &dict) {
                Ok(out) => {
                    if a.write {
                        let target = if to == lang { file.clone() } else { translated_name(&file, to) };
                        std::fs::write(&target, &out).map_err(|e| format!("«{target}» не записан: {e}"))?;
                        eprintln!("записано: {target}");
                    } else {
                        print!("{out}");
                    }
                    Ok(true)
                }
                Err(d) => {
                    eprint!("{}", d.render(&file, &src));
                    Ok(false)
                }
            }
        }
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
            let (r, root) = run_program_file(&a, &file, Run::Source(&src), &dict, lang);
            for d in &r.diagnostics {
                eprint!("{}", render_diag(d, &file, &src, &root));
            }
            Ok(r.ok)
        }
    }
}

/// Что исполнять: исходник `.vlstd` или готовый AST (из `.vlhmn`).
pub enum Run<'a> {
    Source(&'a str),
    Ast(&'a vladx_ast::Program),
}

/// Исполнить программу с хостом CLI. Корень — папка с vladx.toml (иначе папка программы):
/// от него считаются пути file.*, модули — от папки импортирующего файла.
/// Возвращает и корень — по нему ищутся файлы модулей для сообщений об ошибках.
pub fn run_program_file(a: &Args, file: &str, what: Run, dict: &Dictionary, lang: Lang) -> (vladx_runtime::RunResult, PathBuf) {
    let dir = Path::new(file).parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let dir = if dir.as_os_str().is_empty() { PathBuf::from(".") } else { dir };
    let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
    let proj = project::find(&dir).and_then(|r| r.ok());
    let base = proj.as_ref().map(|p| p.root.clone()).unwrap_or_else(|| dir.clone());
    let main_dir = dir
        .strip_prefix(&base)
        .map(|r| r.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/"))
        .unwrap_or_default();
    let mut host = host::CliHost {
        base: base.clone(),
        restrict: a.restrict || proj.as_ref().and_then(|p| p.restrict_files).unwrap_or(false),
        deadline: if a.max_seconds > 0 { Some((Instant::now(), a.max_seconds)) } else { None },
        stream: true,
    };
    let opts = Options {
        max_steps: a.max_steps,
        files: !a.no_files && proj.as_ref().and_then(|p| p.files).unwrap_or(true),
        net: !a.no_net && proj.as_ref().and_then(|p| p.net).unwrap_or(true),
        main_dir,
        ..Options::default()
    };
    let r = match what {
        Run::Source(src) => vladx_runtime::run_source(src, lang, dict, &mut host, opts),
        Run::Ast(p) => vladx_runtime::run_program(p, dict, &mut host, opts),
    };
    (r, base)
}

/// Диагностика с исходником: ошибка в модуле показывается строкой из файла модуля.
pub fn render_diag(d: &vladx_ast::Diagnostic, file: &str, src: &str, root: &Path) -> String {
    match &d.file {
        Some(m) => {
            let path = root.join(m);
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            d.render(&path.display().to_string(), &text)
        }
        None => d.render(file, src),
    }
}

/// `программа.vlstd` → `программа.it.vlstd`.
fn translated_name(file: &str, to: Lang) -> String {
    match file.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}.{}.{ext}", to.code()),
        None => format!("{file}.{}", to.code()),
    }
}

fn init_cmd(a: &Args) -> Result<bool, String> {
    let dict = load_dict(a)?;
    let dir = PathBuf::from(a.file.clone().unwrap_or_else(|| ".".into()));
    let made = project::init(&dir, a.lang.unwrap_or(dict.default_lang), &dict)?;
    for m in &made {
        println!("создан {}", m.display());
    }
    println!("запуск: cd {} && vladx run", dir.display());
    Ok(true)
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
