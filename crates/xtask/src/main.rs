//! `cargo run -p xtask [путь к keywords.toml]` — проверка словаря:
//! дубли словоформ, пустые операции, конфликты служебных слов.

use vladx_syntax::{Dictionary, Lang};

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../../spec/keywords.toml").to_string());
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("словарь «{path}» не прочитан: {e}");
            std::process::exit(1);
        }
    };
    let d = match Dictionary::from_toml(&src) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("E-DICT-001  {e}");
            std::process::exit(1);
        }
    };
    let problems = d.validate();
    let (warns, errors): (Vec<_>, Vec<_>) = problems.iter().partition(|p| p.starts_with("предупреждение"));
    for w in &warns {
        println!("  {w}");
    }
    let drafts = d.entries.iter().filter(|e| e.is_op && e.status == "draft").count();
    println!("словарь: {} операций ({} draft), {} слов выражений, {} типов", d.op_count(), drafts, d.entries.len() - d.op_count(), d.types.len());
    let mods = d.stdlib_modules();
    println!("stdlib: {} модулей, {} функций", mods.len(), d.stdlib.len() - mods.len());
    for l in Lang::ALL {
        println!("  {:<3} — {} словоформ", l.code(), d.all_forms(l).len());
    }
    if !errors.is_empty() {
        for e in &errors {
            eprintln!("ошибка: {e}");
        }
        std::process::exit(1);
    }
    if d.op_count() != 21 {
        eprintln!("ошибка: ожидалась 21 операция, найдено {}", d.op_count());
        std::process::exit(1);
    }
    println!("словарь в порядке");
}
