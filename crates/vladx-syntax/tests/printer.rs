//! Печать AST в текст: форматирование, перевод между языками, проверка AST от ИИ.

use vladx_ast::neutral_json;
use vladx_syntax::{format_source, parse, validate::validate_json, Dictionary, Lang};

fn examples() -> Vec<(Lang, String, String)> {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
    let mut v = vec![];
    for l in Lang::ALL {
        let mut files: Vec<_> = std::fs::read_dir(format!("{root}/{}", l.code())).unwrap().flatten().map(|e| e.path()).collect();
        files.sort();
        for p in files {
            if p.extension().is_some_and(|x| x == "vlstd") {
                v.push((l, p.display().to_string(), std::fs::read_to_string(&p).unwrap()));
            }
        }
    }
    v
}

#[test]
fn format_is_idempotent_and_keeps_comments() {
    let d = Dictionary::builtin();
    for (l, name, src) in examples() {
        let once = format_source(&src, l, l, &d).unwrap_or_else(|e| panic!("{name}: {}", e.message));
        let twice = format_source(&once, l, l, &d).unwrap();
        assert_eq!(once, twice, "{name}: форматирование не идемпотентно");
        for c in src.lines().filter(|x| x.trim_start().starts_with("//")) {
            assert!(once.contains(c.trim()), "{name}: потерян комментарий {c}");
        }
    }
}

#[test]
fn canonical_examples_are_already_formatted() {
    // Примеры — эталон стиля: форматтер не должен их менять.
    let d = Dictionary::builtin();
    for (l, name, src) in examples() {
        let out = format_source(&src, l, l, &d).unwrap();
        assert_eq!(out, src, "{name}: пример не в каноническом виде");
    }
}

#[test]
fn stdlib_sources_are_canonical() {
    // stdlib на VladX — тоже эталон стиля (как lib/*.js в Node проходит линтер).
    let d = Dictionary::builtin();
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vladx-runtime/stdlib");
    let mut n = 0;
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let src = std::fs::read_to_string(e.path()).unwrap();
        let out = format_source(&src, Lang::Ru, Lang::Ru, &d).unwrap_or_else(|x| panic!("{:?}: {}", e.path(), x.message));
        assert_eq!(out, src, "{:?}: не в каноническом виде (vladx fmt --write)", e.path());
        n += 1;
    }
    assert_eq!(n, 12);
}

#[test]
fn translation_between_languages_keeps_ast() {
    let d = Dictionary::builtin();
    let mut translated = 0;
    for (from, name, src) in examples() {
        let orig = neutral_json(&parse(&src, from, &d).unwrap());
        for to in Lang::ALL {
            match format_source(&src, from, to, &d) {
                Ok(out) => {
                    let back = neutral_json(&parse(&out, to, &d).unwrap());
                    assert_eq!(orig, back, "{name} → {}", to.code());
                    translated += 1;
                }
                // У scn нет слов «больше/меньше или равно» — честная ошибка, а не порча.
                Err(e) => assert!(e.message.contains("нет слова"), "{name} → {}: {}", to.code(), e.message),
            }
        }
    }
    assert!(translated >= 40, "переведено {translated}");
}

#[test]
fn validator_accepts_good_ast_and_rejects_bad() {
    let d = Dictionary::builtin();
    let g = &["liste", "длина"];
    let good = r#"{"kind":"Program","schema_version":1,"lang":"ru","body":[
      {"kind":"Declare","name":"покупки","mutable":false,"value":{"kind":"ListLit","items":[
        {"kind":"StringLit","value":"хлеб","parts":null},{"kind":"NumberLit","value":-2}]}},
      {"kind":"ForEach","item":"вещь","iter":{"kind":"Identifier","name":"покупки"},"body":{"kind":"Block","body":[
        {"kind":"Say","value":{"kind":"StringLit","value":"","parts":[{"kind":"Text","value":"- "},{"kind":"Hole","expr":{"kind":"Identifier","name":"вещь"}}]}}]}}]}"#;
    let c = validate_json(good, &d, g).unwrap();
    assert_eq!(c.text, "пусть покупки будет [\"хлеб\", -2]\nдля каждого вещь из покупки {\n  скажи \"- {вещь}\"\n}\n");
    assert!(c.program.body[0].span().is_some(), "позиции указывают в напечатанный текст");

    let code = |j: &str| validate_json(j, &d, g).unwrap_err()[0].code.clone();
    assert_eq!(code(r#"{"kind":"Program","schema_version":1,"lang":"ru","body":[{"kind":"Say"}]}"#), "E-AST-001");
    assert_eq!(code(r#"{"kind":"Program","schema_version":2,"lang":"ru","body":[]}"#), "E-AST-001");
    // имя совпадает с ключевым словом
    assert_eq!(code(r#"{"kind":"Program","schema_version":1,"lang":"ru","body":[{"kind":"Declare","name":"если","mutable":false,"value":{"kind":"NullLit"}}]}"#), "E-AST-002");
    // выражение в интерполяции
    assert_eq!(code(r#"{"kind":"Program","schema_version":1,"lang":"ru","body":[{"kind":"Say","value":{"kind":"StringLit","value":"","parts":[{"kind":"Hole","expr":{"kind":"NumberLit","value":1}}]}}]}"#), "E-AST-002");
    // необъявленное имя — статическая проверка
    assert_eq!(code(r#"{"kind":"Program","schema_version":1,"lang":"ru","body":[{"kind":"Say","value":{"kind":"Identifier","name":"нигде"}}]}"#), "E-NAME-004");
}
