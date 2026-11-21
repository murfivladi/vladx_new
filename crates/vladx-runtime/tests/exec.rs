//! Исполнение: каждое утверждение спецификации — отдельный тест.

use vladx_runtime::{run_source, MemHost, Options, Perm, RunResult};
use vladx_syntax::{Dictionary, Lang};

/// Запуск в потоке с большим стеком (интерпретатор рекурсивный).
fn run_full(src: &str, lang: Lang, host: MemHost, opts: Options) -> (RunResult, MemHost) {
    let src = src.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let mut host = host;
            let d = Dictionary::builtin();
            let r = run_source(&src, lang, &d, &mut host, opts);
            (r, host)
        })
        .unwrap()
        .join()
        .unwrap()
}

fn run(src: &str) -> RunResult {
    run_full(src, Lang::Ru, MemHost::default(), Options { seed: Some(7), ..Options::default() }).0
}

fn out(src: &str) -> Vec<String> {
    let r = run(src);
    assert!(r.ok, "ошибка: {:?}", r.diagnostics);
    r.stdout
}

fn code(src: &str) -> String {
    let r = run(src);
    assert!(!r.ok, "ожидалась ошибка, вывод: {:?}", r.stdout);
    r.diagnostics[0].code.clone()
}

fn warn_codes(r: &RunResult) -> Vec<String> {
    r.warnings.iter().map(|w| w.code.clone()).collect()
}

// ───────────── базовое ─────────────

#[test]
fn hello_world() {
    assert_eq!(out("пусть имя будет \"мир\"\nскажи \"привет, {имя}\""), ["привет, мир"]);
}

#[test]
fn functions_positional_and_named_args() {
    let src = "функция сумма(a, b) {\n  верни a - b\n}\nскажи сумма(5, 3)\nскажи сумма(b = 3, a = 5)";
    assert_eq!(out(src), ["2", "2"]);
}

#[test]
fn missing_return_gives_null_and_defaults_evaluated_each_call() {
    let src = "функция f() {\n}\nскажи f()\nфункция g(x = []) {\n  liste.добавь(x, 1)\n  верни liste.размер(x)\n}\nскажи g()\nскажи g()";
    assert_eq!(out(src), ["ничего", "1", "1"]);
}

#[test]
fn argument_count_errors() {
    assert_eq!(code("функция f(a) {\n}\nf(1, 2)"), "E-ARGS-001");
    assert_eq!(code("функция f(a, b) {\n}\nf(1)"), "E-ARGS-002");
    assert_eq!(code("скажи длина(x = 4)"), "E-ARGS-003");
    assert_eq!(code("функция f(a: число) {\n}\nf(\"а\")"), "E-TYPE-002");
}

#[test]
fn let_binding_is_immutable() {
    assert_eq!(code("пусть x будет 1\nx станет 2"), "E-NAME-002");
    assert_eq!(out("изменяй x станет 1\nx станет 2\nскажи x"), ["2"]);
    // содержимое списка меняется, привязка — нет
    assert_eq!(out("пусть л будет [1]\nliste.добавь(л, 2)\nскажи л"), ["[1, 2]"]);
}

#[test]
fn break_outside_loop_is_error() {
    assert_eq!(code("прерви"), "E-CTRL-001");
    assert_eq!(code("верни 1"), "E-CTRL-003");
}

#[test]
fn loops_break_continue() {
    let src = "изменяй i станет 0\nпока истина {\n  i станет i + 1\n  если i равно 2 то {\n    продолжи\n  }\n  если i больше 3 то {\n    прерви\n  }\n  скажи i\n}";
    assert_eq!(out(src), ["1", "3"]);
    assert_eq!(out("для каждого б из \"аб\" {\n  скажи б\n}"), ["а", "б"]);
    assert_eq!(code("повторяй -1 раз {\n}"), "E-VALUE-001");
    assert_eq!(code("для каждого x из 5 {\n}"), "E-TYPE-006");
}

#[test]
fn infinite_loop_stopped_by_step_limit() {
    let (r, _) = run_full("пока истина {\n}", Lang::Ru, MemHost::default(), Options { max_steps: 10_000, ..Options::default() });
    assert_eq!(r.diagnostics[0].code, "E-LIMIT-001");
    assert_eq!(r.diagnostics[0].line, 1);
}

#[test]
fn deep_recursion_is_limited() {
    assert_eq!(code("функция f(n) {\n  верни f(n + 1)\n}\nf(0)"), "E-LIMIT-002");
    // а 900 уровней — можно
    let src = "функция f(n) {\n  если n равно 0 то {\n    верни 0\n  }\n  верни 1 + f(n - 1)\n}\nскажи f(900)";
    assert_eq!(out(src), ["900"]);
}

// ───────────── приведение типов §4.3 — по тесту на строку ─────────────

#[test]
fn coerce_string_plus_number() {
    let r = run("скажи \"5\" + 3");
    assert_eq!(r.stdout, ["53"]);
    assert_eq!(warn_codes(&r), ["W-COERCE-001"]);
}

#[test]
fn coerce_number_plus_string() {
    let r = run("скажи 5 + \"3\"");
    assert_eq!(r.stdout, ["53"]);
    assert_eq!(warn_codes(&r), ["W-COERCE-001"]);
}

#[test]
fn coerce_string_times_number() {
    let r = run("скажи \"5\" * 2");
    assert_eq!(r.stdout, ["10"]);
    assert_eq!(warn_codes(&r), ["W-COERCE-002"]);
}

#[test]
fn coerce_null_plus_number() {
    let r = run("скажи ничего + 1");
    assert_eq!(r.stdout, ["1"]);
    assert_eq!(warn_codes(&r), ["W-COERCE-003"]);
}

#[test]
fn coerce_true_plus_number() {
    let r = run("скажи истина + 1");
    assert_eq!(r.stdout, ["2"]);
    assert!(r.warnings.is_empty());
}

#[test]
fn coerce_null_equals_null() {
    let r = run("скажи ничего равно ничего");
    assert_eq!(r.stdout, ["истина"]);
    assert!(r.warnings.is_empty());
}

#[test]
fn coerce_zero_equals_string_zero() {
    let r = run("скажи 0 равно \"0\"");
    assert_eq!(r.stdout, ["истина"]);
    assert_eq!(warn_codes(&r), ["W-COERCE-004"]);
}

// ───────────── объекты, замыкания, ошибки ─────────────

#[test]
fn prototypes_and_self() {
    let src = "пусть кот будет {\n  имя: \"Барсик\",\n  мяукать: функция() {\n    скажи \"мяу, я {это.имя}\"\n  }\n}\nкот.мяукать()\nпусть котёнок будет создай объект с прототипом кот {\n  имя: \"Мурзик\"\n}\nкотёнок.мяукать()\nскажи котёнок.нет";
    assert_eq!(out(src), ["мяу, я Барсик", "мяу, я Мурзик", "ничего"]);
    assert_eq!(code("пусть н будет ничего\nскажи н.поле"), "E-NULL-001");
    assert_eq!(code("пусть к будет {}\nк.лаять()"), "E-PROP-001");
}

#[test]
fn closures_capture_by_reference() {
    let src = "функция счётчик() {\n  изменяй n станет 0\n  верни функция() {\n    n станет n + 1\n    верни n\n  }\n}\nпусть с будет счётчик()\nс()\nс()\nскажи с()";
    assert_eq!(out(src), ["3"]);
}

#[test]
fn exceptions_thrown_and_runtime_errors_caught() {
    assert_eq!(
        out("попробуй {\n  брось \"что-то пошло не так\"\n} поймай ошибку {\n  скажи \"поймали: {ошибку}\"\n}"),
        ["поймали: что-то пошло не так"]
    );
    assert_eq!(out("попробуй {\n  скажи 1 / 0\n} поймай е {\n  скажи е.код\n}"), ["E-MATH-001"]);
    let r = run("скажи 1\nбрось \"беда\"");
    assert_eq!(r.stdout, ["1"]);
    assert_eq!(r.diagnostics[0].code, "E-UNCAUGHT");
    assert!(r.diagnostics[0].message.contains("беда"));
}

#[test]
fn runtime_error_codes() {
    assert_eq!(code("скажи 1 % 0"), "E-MATH-002");
    assert_eq!(code("скажи numeri.корень(-1)"), "E-MATH-003");
    assert_eq!(code("скажи liste.возьми([1], 5)"), "E-INDEX-001");
    assert_eq!(code("скажи numeri.нет(1)"), "E-MODULE-002");
    assert_eq!(code("пусть x будет 5\nx()"), "E-TYPE-003");
    assert_eq!(code("скажи json.прочитай(\"\\{плохо\")"), "E-JSON-001");
    assert_eq!(code("скажи caso.целое(5, 1)"), "E-VALUE-002");
    assert_eq!(code("скажи длина(5)"), "E-TYPE-001");
}

// ───────────── stdlib ─────────────

#[test]
fn numbers_print_without_trailing_zero() {
    assert_eq!(out("скажи 5\nскажи 2.5\nскажи 10 / 4\nскажи -0"), ["5", "2.5", "2.5", "0"]);
}

#[test]
fn stdlib_behaviour_from_spec() {
    let src = r#"пусть человек будет { имя: "Аня", город: "Палермо", теги: ["язык", "код"] }
скажи json.запиши(человек)
пусть назад будет json.прочитай(json.запиши(человек))
скажи "имя: {назад.имя}"
пусть числа будет [5, 2, 9, 1]
скажи liste.сортировка(числа)
скажи liste.кусок(числа, 1, 3)
скажи numeri.округли(3.7)
скажи tempo.часы(3661)
скажи testo.кусок("привет", 1, 3)
скажи testo.раздели("а,б", ",")
скажи liste.соедини(["а", "б"], "-")
скажи mappe.ключи(человек)
скажи числа"#;
    assert_eq!(
        out(src),
        [
            r#"{"имя":"Аня","город":"Палермо","теги":["язык","код"]}"#,
            "имя: Аня",
            "[1, 2, 5, 9]",
            "[2, 9]",
            "4",
            "01:01:01",
            "ри",
            "[\"а\", \"б\"]",
            "а-б",
            "[\"имя\", \"город\", \"теги\"]",
            "[5, 2, 9, 1]",
        ]
    );
}

#[test]
fn ask_reads_input() {
    let mut h = MemHost::default();
    h.inputs.push_back("Аня".into());
    let (r, _) = run_full("спроси \"как тебя зовут?\" и сохрани в имя\nскажи \"здравствуй, {имя}\"", Lang::Ru, h, Options::default());
    assert_eq!(r.stdout, ["как тебя зовут? Аня", "здравствуй, Аня"]);
}

#[test]
fn interactive_ask_stops_and_replays_with_answers() {
    let src = "спроси \"как тебя зовут?\" и сохрани в имя\nизменяй лет станет \"\"\nпопробуй {\n  спроси \"сколько лет?\" и сохрани в лет\n} поймай е {\n  скажи \"не должно ловиться\"\n}\nскажи \"{имя}, {лет}\"";
    let opts = Options { interactive: true, ..Options::default() };
    // без ответов — ждёт первый вопрос
    let (r, _) = run_full(src, Lang::Ru, MemHost::default(), opts.clone());
    assert_eq!((r.diagnostics[0].code.as_str(), r.diagnostics[0].message.as_str()), ("E-ASK-WAIT", "как тебя зовут?"));
    // с одним ответом — ждёт второй, «попробуй» не перехватывает ожидание
    let mut h = MemHost::default();
    h.inputs.push_back("Аня".into());
    let (r, _) = run_full(src, Lang::Ru, h, opts.clone());
    assert_eq!(r.diagnostics[0].message, "сколько лет?");
    assert_eq!(r.stdout, ["как тебя зовут? Аня"]);
    // со всеми ответами — доходит до конца
    let mut h = MemHost::default();
    h.inputs.extend(["Аня".to_string(), "30".to_string()]);
    let (r, _) = run_full(src, Lang::Ru, h, opts);
    assert!(r.ok, "{:?}", r.diagnostics);
    assert_eq!(r.stdout.last().unwrap(), "Аня, 30");
}

// ───────────── файлы и сеть ─────────────

#[test]
fn defaults_like_node_files_and_net_open() {
    let mut h = MemHost::default();
    h.net.insert("https://x.test/a".into(), Ok("{\"n\": 3}".into()));
    let src = "file.запиши(\"папка/ф.txt\", \"данные\")\nскажи file.прочитай(\"папка/ф.txt\")\nскажи file.список()\nскажи rete.получиJson(\"https://x.test/a\").n\nскажи rete.разрешено()";
    let (r, h) = run_full(src, Lang::Ru, h, Options::default());
    assert!(r.ok, "{:?}", r.diagnostics);
    assert_eq!(r.stdout, ["данные", "[\"папка/ф.txt\"]", "3", "истина"]);
    assert_eq!(h.files["папка/ф.txt"], "данные"); // запись дошла до хоста
    assert!(h.asks.is_empty(), "по умолчанию ничего не спрашивают");
}

#[test]
fn files_denied_without_permission() {
    let opts = Options { files: false, ..Options::default() };
    let (r, h) = run_full("file.запиши(\"a\", \"b\")", Lang::Ru, MemHost::default(), opts);
    assert_eq!(r.diagnostics[0].code, "E-FILE-001");
    assert!(r.diagnostics[0].message.contains("разрешение человека"));
    assert_eq!(h.asks, [Perm::Files]);
    assert!(h.files.is_empty());
}

#[test]
fn ask_instead_of_refusal_asks_once_and_continues() {
    let h = MemHost { grant_on_ask: true, ..MemHost::default() };
    let opts = Options { files: false, ..Options::default() };
    let src = "file.запиши(\"a.txt\", \"1\")\nfile.запиши(\"b.txt\", \"2\")\nскажи file.список()";
    let (r, h) = run_full(src, Lang::Ru, h, opts);
    assert!(r.ok, "{:?}", r.diagnostics);
    assert_eq!(h.asks, [Perm::Files], "спросить ровно один раз");
    assert_eq!(r.stdout, ["[\"a.txt\", \"b.txt\"]"]);
}

#[test]
fn net_denied_and_bad_url() {
    let opts = Options { net: false, ..Options::default() };
    let (r, _) = run_full("скажи rete.разрешено()\nrete.получи(\"https://example.com\")", Lang::Ru, MemHost::default(), opts);
    assert_eq!(r.stdout, ["ложь"]);
    assert_eq!(r.diagnostics[0].code, "E-NET-001");
    assert_eq!(code("rete.получи(\"ftp://x\")"), "E-NET-001");
}

#[test]
fn delete_and_host_errors_reach_program() {
    let mut h = MemHost::default();
    h.files.insert("x.txt".into(), "1".into());
    let (r, h) = run_full("file.удали(\"x.txt\")\nскажи file.есть(\"x.txt\")", Lang::Ru, h, Options::default());
    assert_eq!(r.stdout, ["ложь"]);
    assert!(h.files.is_empty(), "удаление дошло до хоста");

    let h = MemHost { fail_writes: Some("диск полон".into()), ..MemHost::default() };
    let src = "попробуй {\n  file.запиши(\"a\", \"b\")\n} поймай е {\n  скажи е.код\n  скажи е.сообщение\n}";
    let (r, _) = run_full(src, Lang::Ru, h, Options::default());
    assert_eq!(r.stdout, ["E-FILE-002", "диск полон"]);
    assert_eq!(code("file.прочитай(\"нет.txt\")"), "E-FILE-001");
}

// ───────────── три языка, примеры, негативные случаи ─────────────

#[test]
fn same_program_three_languages_same_output() {
    let ru = "пусть л будет [1, 2, 3]\nдля каждого x из л {\n  если x больше 1 то {\n    скажи x * 10\n  } иначе {\n    скажи ложь\n  }\n}";
    let it = "sia л = [1, 2, 3]\nper ogni x in л {\n  se x maggiore 1 allora {\n    dì x * 10\n  } altrimenti {\n    dì falso\n  }\n}";
    let scn = "fussi л = [1, 2, 3]\npi ogni x nni л {\n  si x cchiù granni 1 allura {\n    dici x * 10\n  } sinò {\n    dici farsu\n  }\n}";
    let a = run_full(ru, Lang::Ru, MemHost::default(), Options::default()).0;
    let b = run_full(it, Lang::It, MemHost::default(), Options::default()).0;
    let c = run_full(scn, Lang::Scn, MemHost::default(), Options::default()).0;
    assert_eq!(a.stdout, ["ложь", "20", "30"]);
    // «ложь» печатается словом языка программы
    assert_eq!(b.stdout, ["falso", "20", "30"]);
    assert_eq!(c.stdout, ["farsu", "20", "30"]);
}

#[test]
fn all_examples_run() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
    let mut n = 0;
    for l in Lang::ALL {
        for e in std::fs::read_dir(format!("{root}/{}", l.code())).unwrap().flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "vlstd") {
                continue;
            }
            let src = std::fs::read_to_string(&p).unwrap();
            let mut h = MemHost::default();
            h.inputs.push_back("Аня".into());
            h.net.insert("https://api.github.com/repos/rust-lang/rust".into(), Ok(r#"{"stargazers_count": 100000, "language": "Rust"}"#.into()));
            h.net.insert("https://example.com".into(), Ok("<!doctype html><html><head><title>Example Domain</title>".into()));
            let (r, _) = run_full(&src, l, h, Options { seed: Some(1), ..Options::default() });
            assert!(r.ok, "{}: {:?}", p.display(), r.diagnostics);
            assert!(!r.stdout.is_empty(), "{}: пустой вывод", p.display());
            n += 1;
        }
    }
    assert!(n >= 15);
}

#[test]
fn negative_cases_give_expected_codes() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/negative");
    let expected = [
        ("01-order.vlstd", "E-ORDER-001"),
        ("02-unclosed-block.vlstd", "E-SYNTAX-003"),
        ("03-mutate-const.vlstd", "E-NAME-002"),
        ("04-unknown-word.vlstd", "E-SYNTAX-016"),
    ];
    for (f, c) in expected {
        let src = std::fs::read_to_string(format!("{dir}/{f}")).unwrap();
        assert_eq!(code(&src), c, "{f}");
    }
}

// ───────────── модули проекта: из "путь" возьми … ─────────────

fn project(files: &[(&str, &str)]) -> MemHost {
    let mut h = MemHost::default();
    for (p, src) in files {
        h.files.insert(p.to_string(), src.to_string());
    }
    h
}

#[test]
fn import_names_from_project_files() {
    let h = project(&[
        ("утилиты.vlstd", "функция удвой(x) {\n  верни x * 2\n}\nпусть версия будет \"1.0\""),
        ("главная.vlstd", ""),
    ]);
    let src = "из \"./утилиты\" возьми удвой, версия\nскажи удвой(21)\nскажи версия";
    let (r, _) = run_full(src, Lang::Ru, h, Options::default());
    assert!(r.ok, "{:?}", r.diagnostics);
    assert_eq!(r.stdout, ["42", "1.0"]);
}

#[test]
fn import_paths_are_relative_to_the_importing_file() {
    // lib/строки.vlstd импортирует ../общее.vlstd; модуль выполняется один раз
    let h = project(&[
        ("общее.vlstd", "скажи \"загружено общее\"\nпусть разделитель будет \" | \""),
        ("lib/строки.vlstd", "из \"../общее.vlstd\" возьми разделитель\nфункция склей(а, б) {\n  верни а + разделитель + б\n}"),
    ]);
    let src = "из \"./lib/строки\" возьми склей\nиз \"./общее\" возьми разделитель\nскажи склей(\"а\", \"б\")";
    let (r, _) = run_full(src, Lang::Ru, h, Options::default());
    assert!(r.ok, "{:?}", r.diagnostics);
    assert_eq!(r.stdout, ["загружено общее", "а | б"]);
}

#[test]
fn import_errors_are_clear() {
    let h = || project(&[
        ("а.vlstd", "из \"./б\" возьми y\nпусть x будет 1"),
        ("б.vlstd", "из \"./а\" возьми x\nпусть y будет 2"),
        ("м.vlstd", "пусть открыто будет 1\nпусть _скрыто будет 2"),
        ("плохой.vlstd", "пусть z будет\n"),
    ]);
    let code = |src: &str| run_full(src, Lang::Ru, h(), Options::default()).0.diagnostics[0].clone();
    let d = code("из \"./нет\" возьми x");
    assert_eq!(d.code, "E-MODULE-001");
    let d = code("из \"./а\" возьми x");
    assert_eq!(d.code, "E-MODULE-003");
    assert!(d.message.contains("а.vlstd → б.vlstd → а.vlstd"), "{}", d.message);
    let d = code("из \"./м\" возьми нигде");
    assert_eq!((d.code.as_str(), d.hint.as_deref()), ("E-MODULE-002", Some("есть: открыто")));
    assert_eq!(code("из \"./м\" возьми _скрыто").code, "E-MODULE-002");
    // синтаксическая ошибка в модуле указывает на его файл
    let d = code("из \"./плохой\" возьми z");
    assert_eq!(d.file.as_deref(), Some("плохой.vlstd"));
}

#[test]
fn runtime_error_inside_module_points_to_module_file() {
    let h = project(&[("мат.vlstd", "функция дели(а, б) {\n  верни а / б\n}")]);
    let (r, _) = run_full("из \"./мат\" возьми дели\nскажи дели(1, 0)", Lang::Ru, h, Options::default());
    let d = &r.diagnostics[0];
    assert_eq!((d.code.as_str(), d.file.as_deref(), d.line), ("E-MATH-001", Some("мат.vlstd"), 2));
    // а ошибка в главной программе — без файла
    let (r, _) = run_full("скажи 1 / 0", Lang::Ru, MemHost::default(), Options::default());
    assert_eq!(r.diagnostics[0].file, None);
}

#[test]
fn modules_have_their_own_globals_and_need_no_file_permission() {
    let h = project(&[("счёт.vlstd", "изменяй n станет 0\nфункция следующий() {\n  n станет n + 1\n  верни n\n}")]);
    // n в главной программе — другое имя; разрешение «файлы» для кода не нужно (как require в Node)
    let src = "из \"./счёт\" возьми следующий\nизменяй n станет 100\nследующий()\nскажи следующий()\nскажи n";
    let opts = Options { files: false, ..Options::default() };
    let (r, h) = run_full(src, Lang::Ru, h, opts);
    assert!(r.ok, "{:?}", r.diagnostics);
    assert_eq!(r.stdout, ["2", "100"]);
    assert!(h.asks.is_empty());
}

// ───────────── пакеты: из "имя-пакета" возьми … ─────────────

#[test]
fn bare_import_finds_package_walking_up() {
    let h = project(&[
        ("пакеты/цвета/vladx.toml", "[проект]\nимя = \"цвета\"\nверсия = \"1.0.0\"\nглавный = \"цвета.vlstd\"\n"),
        ("пакеты/цвета/цвета.vlstd", "из \"./внутри\" возьми база\nиз \"@влад/кисть\" возьми кисть\nфункция красный() {\n  верни база + \"красный \" + кисть\n}"),
        ("пакеты/цвета/внутри.vlstd", "пусть база будет \"цвет: \""),
        ("пакеты/цвета/доп/серый.vlstd", "пусть серый будет \"серый\""),
        // итальянский пакет в русском проекте: у пакета свой язык
        ("пакеты/@влад/кисть/vladx.toml", "[progetto]\nnome = \"@влад/кисть\"\nversione = \"0.1.0\"\nlingua = \"it\"\nprincipale = \"pennello.vlstd\"\n"),
        ("пакеты/@влад/кисть/pennello.vlstd", "sia kist = \"(кисть)\"\nfunzione kisti() {\n  rendi kist\n}\nsia кисть = kisti()"),
        ("src/главная.vlstd", ""),
    ]);
    let src = "из \"цвета\" возьми красный\nиз \"цвета/доп/серый\" возьми серый\nскажи красный()\nскажи серый";
    let opts = Options { main_dir: "src".into(), ..Options::default() };
    let (r, _) = run_full(src, Lang::Ru, h, opts);
    assert!(r.ok, "{:?}", r.diagnostics);
    assert_eq!(r.stdout, ["цвет: красный (кисть)", "серый"]);
}

#[test]
fn bare_import_is_strict_like_node() {
    let h = || project(&[("утилиты.vlstd", "пусть x будет 1")]);
    let d = run_full("из \"утилиты\" возьми x", Lang::Ru, h(), Options::default()).0.diagnostics[0].clone();
    assert_eq!(d.code, "E-MODULE-001");
    assert!(d.message.contains("Пакет «утилиты» не найден"), "{}", d.message);
    assert!(d.hint.as_deref().unwrap_or("").contains("\"./утилиты\""), "{:?}", d.hint);
    let d = run_full("из \"нет-такого\" возьми x", Lang::Ru, h(), Options::default()).0.diagnostics[0].clone();
    assert!(d.hint.as_deref().unwrap_or("").contains("vladpm add нет-такого"), "{:?}", d.hint);
    let ok = run_full("из \"./утилиты\" возьми x\nскажи x", Lang::Ru, h(), Options::default()).0;
    assert_eq!(ok.stdout, ["1"]);
}

// ───────────── stdlib на VladX: vladx:модули ─────────────

#[test]
fn stdlib_matches_dictionary() {
    let p = vladx_runtime::stdlib_problems(&Dictionary::builtin());
    assert!(p.is_empty(), "stdlib и словарь расходятся:\n{}", p.join("\n"));
}

#[test]
fn stdlib_modules_are_imported_like_node() {
    let src = "импортируй строки\nиз \"vladx:списки\" возьми отбери, диапазон\nскажи строки.верхний(\"а\")\nскажи отбери(диапазон(6), функция(x) {\n  верни x % 2 равно 0\n})";
    assert_eq!(out(src), ["А", "[0, 2, 4]"]);
    // ядро программе недоступно
    let r = run("из \"vladx:ядро\" возьми тип");
    assert_eq!(r.diagnostics[0].code, "E-MODULE-001");
    assert_eq!(code("импортируй нечто"), "E-MODULE-001");
    assert_eq!(code("импортируй числа\nскажи числа.нечто(1)"), "E-MODULE-002");
}

#[test]
fn stdlib_names_work_in_every_language() {
    let src = "importa testo\nimporta liste\nda \"vladx:numeri\" prendi radice\ndì testo.maiuscolo(\"ciao\")\ndì liste.ordina([3, 1, 2])\ndì radice(81)\ndì testo.верхний(\"mix\")";
    let (r, _) = run_full(src, Lang::It, MemHost::default(), Options::default());
    assert!(r.ok, "{:?}", r.diagnostics);
    assert_eq!(r.stdout, ["CIAO", "[1, 2, 3]", "9", "MIX"]);
}

#[test]
fn old_global_modules_still_work_with_a_warning() {
    let r = run("скажи numeri.корень(16)\nскажи numeri.корень(25)\nскажи liste.размер([1, 2])");
    assert!(r.ok, "{:?}", r.diagnostics);
    assert_eq!(r.stdout, ["4", "5", "2"]);
    let w: Vec<_> = r.warnings.iter().filter(|w| w.code == "W-DEPRECATED-001").collect();
    assert_eq!(w.len(), 2, "одно предупреждение на модуль: {:?}", r.warnings);
    assert!(w[0].message.contains("импортируй numeri") && w[0].message.contains("«числа»"), "{}", w[0].message);
    // с подключением — без предупреждений
    let r = run("импортируй numeri\nскажи numeri.корень(16)");
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn stdlib_errors_point_to_the_call_in_the_program() {
    let r = run("импортируй строки\n\nскажи строки.символ(\"аб\", 5)");
    let d = &r.diagnostics[0];
    assert_eq!((d.code.as_str(), d.line, d.file.as_deref()), ("E-INDEX-001", 3, None), "{d:?}");
    // неверный тип аргумента — по подсказке типа в stdlib
    let r = run("импортируй строки\nскажи строки.верхний(5)");
    assert_eq!((r.diagnostics[0].code.as_str(), r.diagnostics[0].line), ("E-TYPE-002", 2));
    // ошибка внутри функции программы, вызванной из stdlib, — в программе
    let r = run("импортируй списки\nскажи списки.преобразуй([1], функция(x) {\n  верни x / 0\n})");
    assert_eq!((r.diagnostics[0].code.as_str(), r.diagnostics[0].line), ("E-MATH-001", 3));
    // и ловится
    assert_eq!(out("импортируй строки\nпопробуй {\n  строки.символ(\"а\", 3)\n} поймай е {\n  скажи е.код\n}"), ["E-INDEX-001"]);
}

#[test]
fn new_modules_path_events_assert() {
    let src = "импортируй путь\nскажи путь.соедини(\"a/b\", \"../c/./d.txt\")\nскажи путь.нормализуй(\"/x//y/../z\")\nскажи путь.имя(\"a/b/c.tar.gz\")\nскажи путь.расширение(\"a/b/c.tar.gz\")\nскажи путь.без_расширения(\"c.txt\")\nскажи путь.папка(\"a/b/c.txt\")\nскажи путь.папка(\"c.txt\")";
    assert_eq!(out(src), ["a/c/d.txt", "/x/z", "c.tar.gz", ".gz", "c", "a/b", "."]);
    let src = "импортируй события\nпусть к будет события.источник()\nфункция громко(кто) {\n  скажи \"нажал {кто}\"\n}\nк.на(\"нажата\", громко).один_раз(\"нажата\", функция(кто) {\n  скажи \"впервые {кто}\"\n})\nк.сообщи(\"нажата\", \"Аня\")\nк.сообщи(\"нажата\", \"Боря\")\nк.сними(\"нажата\", громко)\nскажи к.сообщи(\"нажата\", \"Вера\")\nскажи к.слушатели(\"нажата\")";
    assert_eq!(out(src), ["нажал Аня", "впервые Аня", "нажал Боря", "ложь", "0"]);
    let r = run("импортируй проверка\nпроверка.одинаково([1, 2], [1, 2])\nпроверка.одинаково(1 + 1, 3)");
    let d = &r.diagnostics[0];
    assert_eq!((d.code.as_str(), d.line), ("E-ASSERT-001", 3));
    assert!(d.message.contains("ожидалось «3», получено «2»"), "{}", d.message);
    assert_eq!(out("импортируй проверка\nпусть е будет проверка.бросает(функция() {\n  брось \"ой\"\n})\nскажи е"), ["ой"]);
}

#[test]
fn list_and_text_helpers() {
    let src = "импортируй списки\nимпортируй строки\nимпортируй словари\nскажи списки.отсортируй([3, 1, 2], функция(а, б) {\n  верни б - а\n})\nскажи списки.сверни([1, 2, 3, 4], функция(с, x) {\n  верни с + x\n})\nскажи списки.диапазон(5, 0, -2)\nскажи списки.сумма([1, 2, 3])\nскажи строки.обрежь(\"  да  \") + \"|\"\nскажи строки.дополни(7, 3, \"0\")\nскажи строки.слова(\" раз  два \")\nскажи словари.значение({а: 1}, \"б\", 0)\nскажи словари.объедини({а: 1}, {б: 2})";
    assert_eq!(out(src), ["[3, 2, 1]", "10", "[5, 3, 1]", "6", "да|", "007", "[\"раз\", \"два\"]", "0", "{а: 1, б: 2}"]);
}
