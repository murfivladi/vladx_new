//! `.vlhmn` в CLI: перевод через ИИ, показ перевода, запуск, набор фраз для оценки.

use crate::llm::{self, FileCache};
use crate::{run_program_file, Args, Run};
use std::path::{Path, PathBuf};
use vladx_ast::Diagnostic;
use vladx_hmn::{Translation, DEFAULT_MIN_CONFIDENCE};
use vladx_runtime::{MemHost, Options, GLOBAL_NAMES};
use vladx_syntax::{Dictionary, Lang};

fn cache_dir(file: &Path) -> PathBuf {
    let base = file.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let base = if base.as_os_str().is_empty() { PathBuf::from(".") } else { base };
    base.join(".vladx-cache").join("hmn")
}

/// Перевести текст `.vlhmn` (кэш — рядом с файлом).
pub fn translate_file(file: &Path, src: &str, lang: Lang, dict: &Dictionary) -> Result<Translation, Vec<Diagnostic>> {
    let mut provider = llm::provider();
    let mut cache = FileCache { dir: cache_dir(file) };
    vladx_hmn::translate(src, lang, dict, GLOBAL_NAMES, provider.as_mut(), &mut cache)
}

fn min_confidence(a: &Args) -> f64 {
    a.min_confidence
        .or_else(|| std::env::var("VLADX_MIN_CONFIDENCE").ok().and_then(|v| v.parse().ok()))
        .unwrap_or(DEFAULT_MIN_CONFIDENCE)
}

/// Перевод с номерами строк: так человек видит, что именно будет исполнено.
pub fn render_translation(file: &str, t: &Translation) -> String {
    let source = if t.prose_segments == 0 {
        "только код".to_string()
    } else if t.llm_calls == 0 {
        "из кэша".to_string()
    } else {
        format!("запросов к ИИ: {}", t.llm_calls)
    };
    let mut s = format!("── ИИ понял «{file}» так (уверенность {:.2}, {source}) ──\n", t.confidence);
    let lines: Vec<&str> = t.checked.text.lines().collect();
    let w = lines.len().to_string().len();
    for (i, l) in lines.iter().enumerate() {
        s.push_str(&format!("{:>w$} │ {l}\n", i + 1));
    }
    if !t.questions.is_empty() {
        s.push_str("вопросы ИИ:\n");
        for q in &t.questions {
            s.push_str(&format!("  • {q}\n"));
        }
    }
    s.push_str("──\n");
    s
}

pub fn hmn_cmd(a: &Args, file: &str, src: &str, dict: &Dictionary, lang: Lang) -> Result<bool, String> {
    let t = match translate_file(Path::new(file), src, lang, dict) {
        Ok(t) => t,
        Err(ds) => {
            for d in &ds {
                eprint!("{}", d.render(file, src));
            }
            return Ok(false);
        }
    };
    let label = format!("{file} (перевод)");
    match a.cmd.as_str() {
        "parse" => {
            println!("{}", t.checked.program.to_json_pretty());
            Ok(true)
        }
        "check" => {
            print!("{}", render_translation(file, &t));
            match t.check_confidence(min_confidence(a)) {
                Ok(()) => {
                    println!("{file}: ошибок нет");
                    Ok(true)
                }
                Err(d) => {
                    eprint!("{}", d.render(file, src));
                    Ok(false)
                }
            }
        }
        "fmt" => {
            let to = a.to.unwrap_or(lang);
            let out = vladx_syntax::printer::print_program(&t.checked.program, dict, to, None)
                .map_err(|e| format!("не удалось записать на {}: {e}", to.code()))?;
            if a.write {
                let stem = file.trim_end_matches(".vlhmn");
                let target = if to == lang { format!("{stem}.vlstd") } else { format!("{stem}.{}.vlstd", to.code()) };
                std::fs::write(&target, &out).map_err(|e| format!("«{target}» не записан: {e}"))?;
                eprintln!("записано: {target}");
            } else {
                print!("{out}");
            }
            Ok(true)
        }
        _ => {
            if a.show || t.llm_calls > 0 {
                eprint!("{}", render_translation(file, &t));
            }
            if let Err(d) = t.check_confidence(min_confidence(a)) {
                if !(a.show || t.llm_calls > 0) {
                    eprint!("{}", render_translation(file, &t));
                }
                eprint!("{}", d.render(file, src));
                return Ok(false);
            }
            let (r, root) = run_program_file(a, file, Run::Ast(&t.checked.program), dict, lang);
            for d in &r.diagnostics {
                eprint!("{}", crate::render_diag(d, &label, &t.checked.text, &root));
            }
            Ok(r.ok)
        }
    }
}

/// Набор фраз: `NN-имя.vlhmn` + ожидаемый вывод `NN-имя.out` (+ ввод `NN-имя.in`).
/// Критерий фазы 5 (§15): ≥ 90 % фраз → корректная программа, не больше одного повтора.
pub fn eval_cmd(a: &Args, dir: &str, dict: &Dictionary, default_lang: Lang) -> Result<bool, String> {
    let dirp = Path::new(dir);
    let lang = a
        .lang
        .or_else(|| dirp.file_name().and_then(|n| n.to_str()).and_then(Lang::from_code))
        .unwrap_or(default_lang);
    let mut files: Vec<PathBuf> = std::fs::read_dir(dirp)
        .map_err(|e| format!("папка «{dir}» не прочитана: {e}"))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "vlhmn"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("в «{dir}» нет файлов .vlhmn"));
    }
    let (mut ok, mut first_try, mut total, mut calls_sum) = (0, 0, 0, 0);
    for f in &files {
        let name = f.file_stem().and_then(|s| s.to_str()).unwrap_or("?").to_string();
        let Ok(expected) = std::fs::read_to_string(f.with_extension("out")) else {
            println!("  —    {name}: нет файла .out с ожидаемым выводом, пропущено");
            continue;
        };
        total += 1;
        let src = std::fs::read_to_string(f).map_err(|e| e.to_string())?;
        let t = match translate_file(f, &src, lang, dict) {
            Ok(t) => t,
            Err(ds) => {
                let first = ds.first().map(|d| format!("{} {}", d.code, d.message)).unwrap_or_default();
                println!("  FAIL {name}: перевод не удался — {first}");
                continue;
            }
        };
        calls_sum += t.llm_calls;
        let mut host = MemHost::default();
        let inputs: Vec<String> = std::fs::read_to_string(f.with_extension("in"))
            .map(|s| s.lines().map(|l| l.to_string()).collect())
            .unwrap_or_default();
        host.inputs = inputs.iter().cloned().collect();
        let r = vladx_runtime::run_program(&t.checked.program, dict, &mut host, Options { seed: Some(1), ..Options::default() });
        // Эхо «вопрос ответ» от «спроси» не сравнивается: формулировку вопроса выбирает ИИ.
        let mut lines = r.stdout.clone();
        for ans in &inputs {
            if let Some(i) = lines.iter().position(|l| l.ends_with(&format!(" {ans}")) || l == ans) {
                lines.remove(i);
            }
        }
        let got = lines.join("\n");
        let tries = if t.llm_calls == 0 { "кэш".to_string() } else { format!("{} запр.", t.llm_calls) };
        if r.ok && got.trim() == expected.trim() {
            ok += 1;
            if t.llm_calls <= 1 {
                first_try += 1;
            }
            println!("  ok   {name} (уверенность {:.2}, {tries})", t.confidence);
        } else {
            let why = r.diagnostics.first().map(|d| format!("{} {}", d.code, d.message)).unwrap_or_else(|| "другой вывод".into());
            println!("  FAIL {name} (уверенность {:.2}, {tries}): {why}", t.confidence);
            println!("{}", render_translation(&name, &t).lines().map(|l| format!("       {l}")).collect::<Vec<_>>().join("\n"));
            println!("       ожидалось: {:?}\n       получено:  {:?}", expected.trim(), got.trim());
        }
    }
    let pct = if total > 0 { ok as f64 * 100.0 / total as f64 } else { 0.0 };
    println!(
        "\nкорректно: {ok}/{total} ({pct:.0} %), с первой попытки: {first_try}, запросов к ИИ: {calls_sum}; модель: {}",
        llm::provider().model()
    );
    println!("критерий фазы 5 (≥ 90 %, не больше одного повтора): {}", if pct >= 90.0 { "выполнен" } else { "не выполнен" });
    Ok(pct >= 90.0)
}

