//! Конвейер .vlhmn на поддельной модели: без сети и без денег.

use std::collections::HashMap;
use vladx_hmn::{prompt, split_segments, translate, Cache, Llm, Msg, NoCache, Segment};
use vladx_runtime::{run_program, MemHost, Options, GLOBAL_NAMES};
use vladx_syntax::{Dictionary, Lang};

/// Модель-заглушка: отдаёт заранее заготовленные ответы и запоминает запросы.
struct Fake {
    answers: Vec<Result<String, String>>,
    seen: Vec<Vec<Msg>>,
}
impl Fake {
    fn new(answers: Vec<&str>) -> Self {
        Fake { answers: answers.into_iter().rev().map(|a| Ok(a.to_string())).collect(), seen: vec![] }
    }
}
impl Llm for Fake {
    fn model(&self) -> String {
        "fake".into()
    }
    fn chat(&mut self, m: &[Msg]) -> Result<String, String> {
        self.seen.push(m.to_vec());
        self.answers.pop().unwrap_or_else(|| Err("ответов больше нет".into()))
    }
}

#[derive(Default)]
struct MemCache(HashMap<String, String>);
impl Cache for MemCache {
    fn get(&mut self, k: &str) -> Option<String> {
        self.0.get(k).cloned()
    }
    fn put(&mut self, k: &str, v: &str) {
        self.0.insert(k.into(), v.into());
    }
}

const SHOPPING: &str = r#"{"confidence":0.97,"questions":[],"program":{"kind":"Program","schema_version":1,"lang":"ru","body":[
 {"kind":"Declare","name":"покупки","mutable":false,"value":{"kind":"ListLit","items":[
   {"kind":"StringLit","value":"хлеб","parts":null},{"kind":"StringLit","value":"молоко","parts":null},{"kind":"StringLit","value":"сыр","parts":null}]}},
 {"kind":"ForEach","item":"вещь","iter":{"kind":"Identifier","name":"покупки"},"body":{"kind":"Block","body":[
   {"kind":"Say","value":{"kind":"StringLit","value":"- {вещь}","parts":[{"kind":"Text","value":"- "},{"kind":"Hole","expr":{"kind":"Identifier","name":"вещь"}}]}}]}}]}}"#;

const TEXT: &str = "Сделай список покупок: хлеб, молоко и сыр.\nПотом выведи каждый пункт с тире в начале.\n";

fn run(p: &vladx_ast::Program) -> Vec<String> {
    let d = Dictionary::builtin();
    run_program(p, &d, &mut MemHost::default(), Options::default()).stdout
}

#[test]
fn good_answer_becomes_checked_program() {
    let d = Dictionary::builtin();
    let mut llm = Fake::new(vec![SHOPPING]);
    let t = translate(TEXT, Lang::Ru, &d, GLOBAL_NAMES, &mut llm, &mut NoCache).unwrap();
    assert_eq!(t.llm_calls, 1);
    assert_eq!(t.checked.text, "пусть покупки будет [\"хлеб\", \"молоко\", \"сыр\"]\nдля каждого вещь из покупки {\n  скажи \"- {вещь}\"\n}\n");
    assert_eq!(run(&t.checked.program), ["- хлеб", "- молоко", "- сыр"]);
    // §13.14: тот же AST, что у §13.4 (первые 4 строки)
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/ru/04-loops.vlstd")).unwrap();
    let first4: String = src.lines().take(5).collect::<Vec<_>>().join("\n");
    let want = vladx_syntax::parse(&first4, Lang::Ru, &d).unwrap();
    assert_eq!(vladx_ast::neutral_json(&want), vladx_ast::neutral_json(&t.checked.program));
    assert!(t.check_confidence(0.8).is_ok());
}

#[test]
fn json_inside_markdown_fence_is_accepted() {
    let d = Dictionary::builtin();
    let wrapped = format!("Вот программа:\n```json\n{SHOPPING}\n```");
    let mut llm = Fake::new(vec![&wrapped]);
    assert!(translate(TEXT, Lang::Ru, &d, GLOBAL_NAMES, &mut llm, &mut NoCache).is_ok());
}

#[test]
fn one_retry_with_the_list_of_problems() {
    let d = Dictionary::builtin();
    // первый ответ: выражение внутри интерполяции — недопустимо
    let bad = r#"{"confidence":0.9,"questions":[],"program":{"kind":"Program","schema_version":1,"lang":"ru","body":[
      {"kind":"Say","value":{"kind":"StringLit","value":"","parts":[{"kind":"Hole","expr":{"kind":"NumberLit","value":1}}]}}]}}"#;
    let mut llm = Fake::new(vec![bad, SHOPPING]);
    let t = translate(TEXT, Lang::Ru, &d, GLOBAL_NAMES, &mut llm, &mut NoCache).unwrap();
    assert_eq!(t.llm_calls, 2);
    let second = &llm.seen[1];
    assert_eq!(second.len(), 4, "system, user, assistant (плохой ответ), user (что исправить)");
    assert!(second[3].content.contains("E-AST-002"), "{}", second[3].content);
}

#[test]
fn two_bad_answers_stop_with_e_hmn_003() {
    let d = Dictionary::builtin();
    let mut llm = Fake::new(vec!["не JSON", r#"{"confidence":1,"program":{"kind":"Nope"}}"#]);
    let e = translate(TEXT, Lang::Ru, &d, GLOBAL_NAMES, &mut llm, &mut NoCache).unwrap_err();
    assert_eq!(e[0].code, "E-HMN-003");
    assert_eq!(llm.seen.len(), 2, "не больше одного повтора");
}

#[test]
fn undeclared_name_is_rejected_by_static_check() {
    let d = Dictionary::builtin();
    let bad = r#"{"confidence":1,"questions":[],"program":{"kind":"Program","schema_version":1,"lang":"ru","body":[
      {"kind":"Say","value":{"kind":"Identifier","name":"нигде"}}]}}"#;
    let mut llm = Fake::new(vec![bad, bad]);
    let e = translate(TEXT, Lang::Ru, &d, GLOBAL_NAMES, &mut llm, &mut NoCache).unwrap_err();
    assert!(e.iter().any(|x| x.code == "E-NAME-004"), "{e:?}");
}

#[test]
fn low_confidence_is_reported_with_questions() {
    let d = Dictionary::builtin();
    let unsure = SHOPPING.replace(r#""confidence":0.97,"questions":[]"#, r#""confidence":0.4,"questions":["Сколько раз повторить?"]"#);
    let mut llm = Fake::new(vec![&unsure]);
    let t = translate(TEXT, Lang::Ru, &d, GLOBAL_NAMES, &mut llm, &mut NoCache).unwrap();
    let e = t.check_confidence(0.8).unwrap_err();
    assert_eq!(e.code, "E-HMN-004");
    assert!(e.hint.unwrap().contains("Сколько раз повторить?"));
    assert!(t.check_confidence(0.0).is_ok());
}

#[test]
fn prose_and_code_blocks_mix() {
    let d = Dictionary::builtin();
    let text = "```\nпусть покупки будет [\"хлеб\", \"сыр\"]\n```\nВыведи количество покупок.\n";
    let answer = r#"{"confidence":0.95,"questions":[],"program":{"kind":"Program","schema_version":1,"lang":"ru","body":[
      {"kind":"Say","value":{"kind":"Call","callee":{"kind":"Identifier","name":"длина"},"args":[{"value":{"kind":"Identifier","name":"покупки"}}]}}]}}"#;
    let mut llm = Fake::new(vec![answer]);
    let t = translate(text, Lang::Ru, &d, GLOBAL_NAMES, &mut llm, &mut NoCache).unwrap();
    assert_eq!(run(&t.checked.program), ["2"]);
    let user = &llm.seen[0][1].content;
    assert!(user.contains("покупки") && !user.contains("хлеб"), "в LLM уходит проза и имена, но не код: {user}");
    assert_eq!(t.prose_segments, 1);
}

#[test]
fn code_only_file_needs_no_llm() {
    let d = Dictionary::builtin();
    let mut llm = Fake::new(vec![]);
    let t = translate("```\nскажи 1\n```\n", Lang::Ru, &d, GLOBAL_NAMES, &mut llm, &mut NoCache).unwrap();
    assert_eq!(t.llm_calls, 0);
    assert_eq!(t.confidence, 1.0);
}

#[test]
fn cache_avoids_second_call() {
    let d = Dictionary::builtin();
    let mut cache = MemCache::default();
    let mut llm = Fake::new(vec![SHOPPING]);
    translate(TEXT, Lang::Ru, &d, GLOBAL_NAMES, &mut llm, &mut cache).unwrap();
    let mut llm2 = Fake::new(vec![]);
    let t = translate(TEXT, Lang::Ru, &d, GLOBAL_NAMES, &mut llm2, &mut cache).unwrap();
    assert_eq!((t.llm_calls, t.cached_segments), (0, 1));
}

#[test]
fn provider_error_is_e_hmn_001() {
    let d = Dictionary::builtin();
    let mut llm = Fake { answers: vec![Err("402 недостаточно средств".into())], seen: vec![] };
    let e = translate(TEXT, Lang::Ru, &d, GLOBAL_NAMES, &mut llm, &mut NoCache).unwrap_err();
    assert_eq!(e[0].code, "E-HMN-001");
    assert!(e[0].message.contains("402"));
}

#[test]
fn segments_split_on_fences() {
    let s = split_segments("Текст раз.\n\n```vlstd\nскажи 1\n```\nТекст два.\n");
    assert_eq!(
        s,
        vec![
            Segment::Prose("Текст раз.\n".into(), 1),
            Segment::Code("скажи 1\n".into(), 4),
            Segment::Prose("Текст два.\n".into(), 6),
        ]
    );
}

#[test]
fn system_prompt_is_built_from_project_data() {
    let d = Dictionary::builtin();
    for l in Lang::ALL {
        let s = prompt::system_prompt(&d, l);
        assert!(s.contains(&format!("\"lang\" = \"{}\"", l.code())));
        assert!(s.contains(&d.form(l, "io.say")), "зарезервированные слова языка {} в промпте", l.code());
        assert_eq!(s.matches("Описание:").count(), prompt::fewshot().len());
    }
    // примеры промпта — рабочие программы
    for (name, _, code) in prompt::fewshot() {
        let p = vladx_syntax::parse(code, Lang::Ru, &d).unwrap();
        let mut h = MemHost::default();
        h.inputs.push_back("20".into());
        let r = run_program(&p, &d, &mut h, Options::default());
        assert!(r.ok, "{name}: {:?}", r.diagnostics);
    }
}
