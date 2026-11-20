//! Соответствие спецификации: утверждения из spec/SPEC.md, а не «не упало».

use vladx_ast::neutral_json;
use vladx_syntax::{analyze, parse, Dictionary, Lang};

fn dict() -> Dictionary {
    Dictionary::builtin()
}

fn err_code(src: &str, lang: Lang) -> String {
    let d = dict();
    let (_, diags) = analyze(src, lang, &d, &["numeri", "liste"]);
    diags.into_iter().find(|d| d.is_error()).map(|d| d.code).unwrap_or_else(|| "OK".into())
}

#[test]
fn dictionary_is_valid_and_has_21_operations() {
    let d = dict();
    let errors: Vec<String> = d.validate().into_iter().filter(|p| !p.starts_with("предупреждение")).collect();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(d.op_count(), 21);
    for l in Lang::ALL {
        assert!(!d.forms(l, "io.say").is_empty());
    }
}

#[test]
fn parser_sources_contain_no_word_forms() {
    // Правило №1: слова живут только в keywords.toml.
    let d = dict();
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    for f in ["parser.rs", "lexer.rs", "semantic.rs"] {
        let src = std::fs::read_to_string(format!("{dir}/{f}")).unwrap();
        for l in Lang::ALL {
            for (id, form) in d.all_forms(l) {
                for quoted in [format!("\"{form}\""), format!("«{form}»")] {
                    assert!(!src.contains(&quoted), "{f}: словоформа {quoted} ({id}) зашита в код");
                }
            }
        }
    }
}

#[test]
fn three_languages_give_identical_ast() {
    let ru = r#"
пусть имя будет "мир"
изменяй n станет 0
скажи "привет, {имя}"
функция сумма(a, b) {
  верни a + b
}
если сумма(2, 3) больше 4 и не ложь то {
  скажи истина
} иначе {
  скажи ничего
}
для каждого x из [1, 2] {
  n станет n + x
}
пока n меньше 10 {
  n станет n + 1
  прерви
}
пусть о будет { м: функция() {
  скажи "я {это.м}"
} }
"#;
    let it = r#"
sia имя = "мир"
variabile n diventa 0
dì "привет, {имя}"
funzione сумма(a, b) {
  rendi a + b
}
se сумма(2, 3) maggiore 4 e non falso allora {
  dì vero
} altrimenti {
  dì niente
}
per ogni x in [1, 2] {
  n diventa n + x
}
finché n minore 10 {
  n diventa n + 1
  interrompi
}
sia о = { м: funzione() {
  dì "я {questo.м}"
} }
"#;
    let scn = r#"
fussi имя = "мир"
variàbbili n diventa 0
dici "привет, {имя}"
funzioni сумма(a, b) {
  arritorna a + b
}
si сумма(2, 3) cchiù granni 4 e nun farsu allura {
  dici veru
} sinò {
  dici nenti
}
pi ogni x di [1, 2] {
  n diventa n + x
}
mentri n cchiù nicu 10 {
  n diventa n + 1
  ntirrumpi
}
fussi о = { м: funzioni() {
  dici "я {chistu.м}"
} }
"#;
    let d = dict();
    let a = neutral_json(&parse(ru, Lang::Ru, &d).unwrap());
    let b = neutral_json(&parse(it, Lang::It, &d).unwrap());
    let c = neutral_json(&parse(scn, Lang::Scn, &d).unwrap());
    assert_eq!(a, b, "ru ≠ it");
    assert_eq!(a, c, "ru ≠ scn");
    assert_eq!(serde_json::to_string(&a).unwrap(), serde_json::to_string(&c).unwrap());
    assert!(serde_json::to_string(&a).unwrap().contains("SelfExpr"), "в тесте есть «это» внутри строки");
}

#[test]
fn ast_of_hello_matches_contract() {
    let d = dict();
    let p = parse("пусть имя будет \"мир\"\nскажи \"привет, {имя}\"\n", Lang::Ru, &d).unwrap();
    let got = serde_json::to_value(&p).unwrap();
    let want: serde_json::Value = serde_json::from_str(
        r#"{
  "kind": "Program", "schema_version": 1, "lang": "ru",
  "body": [
    { "kind": "Declare", "span": {"line":1,"col":1,"len":5},
      "name": "имя", "mutable": false,
      "value": { "kind": "StringLit", "span": {"line":1,"col":17,"len":5},
                 "value": "мир", "parts": null } },
    { "kind": "Say", "span": {"line":2,"col":1,"len":5},
      "value": { "kind": "StringLit", "span": {"line":2,"col":7,"len":15},
                 "value": "привет, {имя}",
                 "parts": [ {"kind":"Text","value":"привет, "},
                            {"kind":"Hole","expr":{"kind":"Identifier","name":"имя"}} ] } }
  ]
}"#,
    )
    .unwrap();
    assert_eq!(got, want);
    // и обратно: JSON → AST без потерь
    let back: vladx_ast::Program = serde_json::from_value(got).unwrap();
    assert_eq!(back, p);
}

#[test]
fn keywords_are_case_insensitive_and_bom_nfc_ok() {
    let d = dict();
    let a = neutral_json(&parse("скажи 1", Lang::Ru, &d).unwrap());
    let b = neutral_json(&parse("\u{feff}СКАЖИ 1", Lang::Ru, &d).unwrap());
    assert_eq!(a, b);
    // «dì» из двух кодовых точек (d, i, U+0300) сводится NFC к ключевому слову.
    let c = neutral_json(&parse("di\u{0300} 1", Lang::It, &d).unwrap());
    assert_eq!(a, c);
}

#[test]
fn word_order_is_fixed() {
    assert_eq!(err_code("имя пусть будет 5", Lang::Ru), "E-ORDER-001");
    assert_eq!(err_code("пусть x 5", Lang::Ru), "E-ORDER-001");
    assert_eq!(err_code("пусть будет x 5", Lang::Ru), "E-ORDER-001");
    assert_eq!(err_code("спроси \"имя?\"", Lang::Ru), "E-ORDER-002");
    assert_eq!(err_code("повторяй 3 {\n}", Lang::Ru), "E-ORDER-004");
    assert_eq!(err_code("для каждого x [1] {\n}", Lang::Ru), "E-ORDER-005");
    assert_eq!(err_code("попробуй {\n}\nскажи 1", Lang::Ru), "E-ORDER-006");
    assert_eq!(err_code("скажи если", Lang::Ru), "E-ORDER-009");
}

#[test]
fn syntax_errors_have_stable_codes() {
    assert_eq!(err_code("скажи привет мир", Lang::Ru), "E-SYNTAX-016");
    assert_eq!(err_code("если 1 больше 0 то {\n скажи 1\n", Lang::Ru), "E-SYNTAX-003");
    assert_eq!(err_code("если 1 больше 0 то скажи 1", Lang::Ru), "E-SYNTAX-002");
    assert_eq!(err_code("скажи [1, 2", Lang::Ru), "E-SYNTAX-009");
    assert_eq!(err_code("скажи (1 + 2", Lang::Ru), "E-SYNTAX-008");
    assert_eq!(err_code("скажи \"абв", Lang::Ru), "E-LEX-001");
    assert_eq!(err_code("скажи 3раз", Lang::Ru), "E-LEX-002");
    assert_eq!(err_code("скажи 1 > 0", Lang::Ru), "E-LEX-003");
    assert_eq!(err_code("пусть если будет 1", Lang::Ru), "E-IDENT-002");
    assert_eq!(err_code("скажи 1\n2 станет 3", Lang::Ru), "E-TYPE-004");
}

#[test]
fn string_interpolation_only_names_and_paths() {
    let d = dict();
    assert_eq!(err_code("пусть a будет 1\nскажи \"{a + 1}\"", Lang::Ru), "E-STRING-003");
    assert_eq!(err_code("скажи \"{}\"", Lang::Ru), "E-STRING-002");
    assert_eq!(err_code("скажи \"{a\"", Lang::Ru), "E-STRING-001");
    // \{ — литеральная скобка, интерполяции нет
    let p = parse("скажи \"\\{x}\"", Lang::Ru, &d).unwrap();
    let j = serde_json::to_value(&p).unwrap();
    assert_eq!(j["body"][0]["value"]["value"], "{x}");
    assert!(j["body"][0]["value"]["parts"].is_null());
    // путь по полям
    let p = parse("пусть ч будет { имя: 1 }\nскажи \"{ч.имя}\"", Lang::Ru, &d).unwrap();
    let j = serde_json::to_value(&p).unwrap();
    assert_eq!(j["body"][1]["value"]["parts"][0]["expr"]["kind"], "Member");
}

#[test]
fn static_checks_follow_spec() {
    assert_eq!(err_code("пусть x будет 1\nx станет 2", Lang::Ru), "E-NAME-002");
    assert_eq!(err_code("пусть x будет 1\nпусть x будет 2", Lang::Ru), "E-NAME-001");
    assert_eq!(err_code("скажи y", Lang::Ru), "E-NAME-004");
    assert_eq!(err_code("скажи это.имя", Lang::Ru), "E-NAME-005");
    assert_eq!(err_code("прерви", Lang::Ru), "E-CTRL-001");
    assert_eq!(err_code("продолжи", Lang::Ru), "E-CTRL-002");
    assert_eq!(err_code("верни 1", Lang::Ru), "E-CTRL-003");
    assert_eq!(err_code("импортируй nonesiste", Lang::Ru), "E-MODULE-001");
    // функции поднимаются; ссылка вперёд из тела функции разрешена
    assert_eq!(err_code("f()\nфункция f() {\n  верни g\n}\nпусть g будет 1", Lang::Ru), "OK");
    // пусть внутри блока затеняет внешнее имя — это не повторное объявление
    assert_eq!(err_code("пусть x будет 1\nесли истина то {\n  пусть x будет 2\n}", Lang::Ru), "OK");
}

#[test]
fn all_examples_parse() {
    let d = dict();
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
    let mut n = 0;
    for l in Lang::ALL {
        let dir = format!("{root}/{}", l.code());
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "vlstd") {
                let src = std::fs::read_to_string(&p).unwrap();
                parse(&src, l, &d).unwrap_or_else(|e| panic!("{}: {}", p.display(), e.render("", &src)));
                n += 1;
            }
        }
    }
    assert!(n >= 15, "примеров слишком мало: {n}");
}
