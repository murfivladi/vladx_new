//! Системный промпт для перевода `.vlhmn` в AST.
//!
//! Собирается из данных проекта: ключевые слова — из словаря, примеры —
//! из пар `fewshot/*.vlhmn` + `*.vlstd` (AST получается настоящим парсером),
//! поэтому промпт не расходится с языком.

use vladx_syntax::{Dictionary, Lang};

/// Меняется при любой правке промпта: входит в ключ кэша.
pub const PROMPT_VERSION: &str = "vlhmn-prompt-1";

const FEWSHOT: &[(&str, &str, &str)] = &[
    ("01-shopping", include_str!("../fewshot/01-shopping.vlhmn"), include_str!("../fewshot/01-shopping.vlstd")),
    ("02-square", include_str!("../fewshot/02-square.vlhmn"), include_str!("../fewshot/02-square.vlstd")),
    ("03-age", include_str!("../fewshot/03-age.vlhmn"), include_str!("../fewshot/03-age.vlstd")),
    ("04-sum", include_str!("../fewshot/04-sum.vlhmn"), include_str!("../fewshot/04-sum.vlstd")),
];

/// Пары «текст → программа» для примеров и тестов.
pub fn fewshot() -> &'static [(&'static str, &'static str, &'static str)] {
    FEWSHOT
}

const AST_REFERENCE: &str = r#"## Нейтральный AST VladX (JSON, поле "kind" — тип узла; позиции "span" не пиши)

Программа: {"kind":"Program","schema_version":1,"lang":"<язык>","body":[инструкции]}
Блок: {"kind":"Block","body":[инструкции]}

Инструкции:
- Declare {name, mutable, value} — объявление. mutable=true, только если значение потом меняется через Assign.
- Assign {target, value} — target: Identifier или Member.
- Say {value} — вывести значение строкой.
- Ask {prompt, name} — задать вопрос человеку; ответ (СТРОКА) попадает в переменную name. Для чисел потом число(name).
- If {test, then: Блок, else: Блок или null} — «иначе если» = else-блок из одного If.
- While {test, body}; Repeat {times, body}; ForEach {item, iter, body} — iter: список, строка или словарь (ключи).
- Break {}; Continue {} — только внутри цикла.
- FunctionDecl {name, params, body}; параметр: {name} или {name, "typeHint": number|string|bool|list|map|function, "default": выражение}.
- Return {value: выражение или null} — только внутри функции.
- Try {body, error_name, handler}; Throw {value}.
- ExprStmt {expr} — выражение как инструкция (обычно вызов функции).

Выражения:
- NumberLit {value} — число (float64; отрицательное — Unary neg); BoolLit {value}; NullLit {}.
- StringLit {value, parts}: без подстановок — {"value":"текст","parts":null};
  с подстановками — parts: [{"kind":"Text","value":"..."}, {"kind":"Hole","expr": Identifier | Member | SelfExpr}],
  value — тот же текст с {имя}. В Hole НЕЛЬЗЯ выражения и вызовы: сначала Declare переменной, потом {переменная}.
- ListLit {items}; MapLit {entries: [{key, value}]}.
- Identifier {name}; Member {object, property}; Call {callee, args: [{value} или {value, name}]}.
- Binary {op, left, right}, op ∈ add sub mul div mod gt lt ge le eq ne and or.
- Unary {op, operand}, op ∈ neg not.
- SelfExpr {} — «это» внутри метода объекта; FunctionExpr {params, body}; ObjectExpr {proto, entries}.

## Семантика
- Числа — только float64; деление на 0 — ошибка. Индексы списков и строк — с 0.
- Строка + число склеивается с предупреждением: для вывода лучше интерполяция в StringLit.
- Ask всегда даёт строку.
- Сравнение — Binary eq/ne/gt/lt/ge/le; логика — and/or и Unary not.

## Стандартная библиотека (других функций нет — не придумывай)
Глобальные функции: длина(x), размер(x) — строки, списка, словаря; строка(x); число(x).
numeri.корень(x), numeri.модуль(x), numeri.округли(x, знаков?), numeri.пол(x), numeri.потолок(x), numeri.случайное() / numeri.случайное(от, до)
testo.длина(s), testo.верхний(s), testo.нижний(s), testo.обрежь(s), testo.раздели(s, разделитель), testo.соедини(список, разделитель),
  testo.содержит(s, часть), testo.начинается(s, начало), testo.заканчивается(s, конец), testo.замени(s, что, на),
  testo.кусок(s, от, до) (до не включается), testo.символ(s, i), testo.повтори(s, n), testo.слова(s)
liste.добавь(список, x) (меняет список), liste.размер(л), liste.возьми(л, i), liste.сортировка(л) (копия), liste.переверни(л) (копия),
  liste.содержит(л, x), liste.найди(л, x) (индекс или -1), liste.кусок(л, от, до), liste.соедини(л, разделитель), liste.выбери(л)
mappe.ключи(м), mappe.значения(м), mappe.есть(м, ключ), mappe.размер(м), mappe.удали(м, ключ)
caso.дробь(), caso.целое(от, до) (включительно), caso.выбери(список), caso.перемешай(список)
tempo.сейчас() ("ЧЧ:ММ:СС"), tempo.миллисекунды(), tempo.часы(секунды)
json.запиши(значение), json.прочитай(строка)
file.прочитай(путь), file.запиши(путь, данные), file.есть(путь), file.список(), file.удали(путь)
rete.получи(url), rete.получиJson(url), rete.разрешено()
Модули и функции — это Member + Call: {"kind":"Call","callee":{"kind":"Member","object":{"kind":"Identifier","name":"liste"},"property":"добавь"},"args":[...]}.
"#;

const ANSWER_FORMAT: &str = r#"## Формат ответа
Ответь ОДНИМ JSON-объектом, без markdown и без пояснений вокруг:
{"confidence": число от 0 до 1, "questions": [строки], "program": Программа}
- confidence — насколько ты уверен, что программа делает именно то, что хотел человек.
  Если текст неоднозначен или чего-то не хватает, всё равно дай лучшую программу,
  но поставь confidence ниже 0.8 и перечисли в questions, что нужно уточнить.
- questions — пустой список, если всё ясно.
"#;

/// Слова языка, которые нельзя брать в имена (однословные ключевые слова).
fn reserved_words(dict: &Dictionary, lang: Lang) -> String {
    let mut ws: Vec<String> =
        dict.all_forms(lang).into_iter().map(|(_, f)| f).filter(|f| !f.contains(' ')).collect();
    ws.sort();
    ws.dedup();
    ws.join(", ")
}

fn lang_name(lang: Lang) -> &'static str {
    match lang {
        Lang::Ru => "русский (ru)",
        Lang::It => "итальянский (it)",
        Lang::Scn => "сицилийский (scn)",
    }
}

/// Системный промпт. Не зависит от текста запроса (хорошо кэшируется у провайдера).
pub fn system_prompt(dict: &Dictionary, lang: Lang) -> String {
    let mut s = String::new();
    s.push_str("Ты переводишь описание программы на естественном языке в программу на языке VladX ");
    s.push_str("в виде нейтрального AST (JSON). Программа должна делать ровно то, что описано, — не больше.\n\n");
    s.push_str(&format!(
        "Язык программы: {}. Поле \"lang\" = \"{}\". Имена переменных и функций, а также выводимые тексты пиши на языке описания; \
имена — одним словом (буквы, цифры, _), camelCase допустим.\n",
        lang_name(lang),
        lang.code()
    ));
    s.push_str(&format!("Эти слова зарезервированы, их нельзя брать в имена: {}.\n\n", reserved_words(dict, lang)));
    s.push_str(AST_REFERENCE);
    s.push('\n');
    s.push_str(ANSWER_FORMAT);
    s.push_str("\n## Примеры\n");
    for (i, (_, text, code)) in FEWSHOT.iter().enumerate() {
        let p = vladx_syntax::parse(code, Lang::Ru, dict).expect("пример промпта разбирается");
        let ast = vladx_ast::json_without_spans(&p);
        let answer = serde_json::json!({"confidence": 0.95, "questions": [], "program": ast});
        s.push_str(&format!("\nПример {} (ru).\nОписание:\n{}\nОтвет:\n{}\n", i + 1, text.trim(), answer));
    }
    s
}

/// Сообщение пользователя: текст + имена, объявленные выше по файлу.
pub fn user_prompt(text: &str, lang: Lang, known_names: &[String]) -> String {
    let mut s = String::new();
    if !known_names.is_empty() {
        s.push_str(&format!(
            "Выше в этом файле уже объявлены (их можно использовать, объявлять заново нельзя): {}.\n\n",
            known_names.join(", ")
        ));
    }
    s.push_str(&format!("Язык программы: {}.\nОписание:\n{}\n", lang.code(), text.trim()));
    s
}

/// Просьба исправить ответ после неудачной проверки.
pub fn repair_prompt(problems: &[String]) -> String {
    let mut s = String::from("Ответ не прошёл проверку:\n");
    for p in problems {
        s.push_str("- ");
        s.push_str(p);
        s.push('\n');
    }
    s.push_str("Исправь и пришли полный ответ заново — один JSON-объект в том же формате.");
    s
}
