use std::collections::BTreeSet;
use vladx_pm::semver::{Range, Version};
use vladx_pm::*;
use vladx_syntax::{Dictionary, Lang};

fn m(r: &str, v: &str) -> bool {
    Range::parse(r).unwrap().matches(&Version::parse(v).unwrap())
}

#[test]
fn semver_ranges() {
    assert!(m("^1.2.0", "1.9.9") && !m("^1.2.0", "2.0.0") && !m("^1.2.0", "1.1.0"));
    assert!(m("^0.2.3", "0.2.9") && !m("^0.2.3", "0.3.0"));
    assert!(m("^0.0.3", "0.0.3") && !m("^0.0.3", "0.0.4"));
    assert!(m("~1.2.3", "1.2.9") && !m("~1.2.3", "1.3.0"));
    assert!(m(">=1.0.0 <2.0.0", "1.5.0") && !m(">=1.0.0 <2.0.0", "2.0.0"));
    assert!(m(">= 1.0", "1.0.0"));
    assert!(m("1.x", "1.4.0") && !m("1.x", "2.0.0"));
    assert!(m("*", "3.1.4") && m("", "0.0.1"));
    assert!(m("1.2.3 - 2.0.0", "2.0.0") && !m("1.2.3 - 2.0.0", "2.0.1"));
    assert!(m("1.2.3 - 2", "2.9.0") && !m("1.2.3 - 2", "3.0.0"));
    assert!(m("^1.0.0 || ^3.0.0", "3.2.0") && !m("^1.0.0 || ^3.0.0", "2.0.0"));
    assert!(m("1.2.3", "1.2.3") && !m("1.2.3", "1.2.4"));
    // предрелизы — только по явному запросу
    assert!(!m("^1.0.0", "1.1.0-бета.1"));
    assert!(m("^1.1.0-бета.1", "1.1.0-бета.2"));
    assert!(Version::parse("1.0.0-alpha").unwrap() < Version::parse("1.0.0").unwrap());
    assert!(Version::parse("1.0.0-alpha.2").unwrap() < Version::parse("1.0.0-alpha.10").unwrap());
    assert!(Range::parse("^abc").is_err() && Range::parse("1.2.3.4").is_err());
    assert_eq!(Range::parse("^1.0.0").unwrap().best(["1.0.0", "1.4.2", "2.0.0", "1.10.0"]), Some("1.10.0".into()));
}

fn d() -> Dictionary {
    Dictionary::builtin()
}

#[test]
fn manifest_localized_and_english() {
    let ru = "[проект]\nимя = \"@влад/строки\"\nверсия = \"1.2.0\"\nязык = \"ru\"\nглавный = \"главная.vlstd\"\n\n# комментарий\n[зависимости]\n@влад/утилиты = \"^1.0.0\"\nцвета = \"~2.1\"\n\n[скрипты]\nтест = \"vladx run тесты.vlstd\"\n\n[права]\nсеть = false\n";
    let a = manifest::parse(ru, &d()).unwrap();
    assert_eq!(a.name.as_deref(), Some("@влад/строки"));
    assert_eq!(a.lang, Some(Lang::Ru));
    assert_eq!(a.net, Some(false));
    assert_eq!(a.dependencies.get("@влад/утилиты").map(|s| s.as_str()), Some("^1.0.0"));
    assert_eq!(a.scripts.get("тест").map(|s| s.as_str()), Some("vladx run тесты.vlstd"));
    let it = "[progetto]\nnome = \"@влад/строки\"\nversione = \"1.2.0\"\nlingua = \"ru\"\nprincipale = \"главная.vlstd\"\n[dipendenze]\n\"@влад/утилиты\" = \"^1.0.0\"\nцвета = \"~2.1\"\n[script]\nтест = \"vladx run тесты.vlstd\"\n[permessi]\nrete = false\n";
    assert_eq!(manifest::parse(it, &d()).unwrap(), a, "все языки читаются одинаково");
    let en = "[project]\nname = \"x\"\nlang = \"it\"\nmain = \"a.vlstd\"\n[permissions]\nfiles = true\n";
    let e = manifest::parse(en, &d()).unwrap();
    assert_eq!(e.lang, Some(Lang::It));
    assert_eq!(e.files, Some(true));
    // запись на языке проекта и обратно
    for l in Lang::ALL {
        let out = manifest::render(&a, l, &d());
        assert_eq!(manifest::parse(&out, &d()).unwrap(), a, "{out}");
    }
    assert!(manifest::render(&a, Lang::It, &d()).contains("[dipendenze]"));
    assert!(manifest::parse("[проект]\nцвет = \"x\"\n", &d()).unwrap_err().contains("цвет"));
    assert!(manifest::parse("[зависимости]\nа = \"^^1\"\n", &d()).is_err());
    assert_eq!(manifest::modules_dir(&d(), Lang::Ru), "пакеты");
    assert_eq!(manifest::modules_dir(&d(), Lang::It), "pacchetti");
}

#[test]
fn names() {
    for ok in ["строки", "@влад/строки", "colori-utili", "a.b_c", "π"] {
        manifest::valid_name(ok).unwrap();
    }
    for bad in ["", "@влад", "../x", "a/b", "-x", "Строки", "a b", "@/x"] {
        assert!(manifest::valid_name(bad).is_err(), "{bad}");
    }
    assert_eq!(install::split_spec("@влад/строки@^1.0"), ("@влад/строки".into(), Some("^1.0".into())));
    assert_eq!(install::split_spec("@влад/строки"), ("@влад/строки".into(), None));
    assert_eq!(install::split_spec("цвета@2.0.0"), ("цвета".into(), Some("2.0.0".into())));
}

#[test]
fn set_dependency_keeps_comments() {
    let src = "[проект]\nимя = \"x\"\n\n# мои пакеты\n[зависимости]\nа = \"^1.0.0\" # важно\n\n[скрипты]\nт = \"x\"\n";
    let s = manifest::set_dependency(src, &d(), Lang::Ru, "@б/в", Some("^2.0.0"));
    assert!(s.contains("# мои пакеты") && s.contains("# важно"));
    let m = manifest::parse(&s, &d()).unwrap();
    assert_eq!(m.dependencies.len(), 2);
    assert_eq!(m.scripts.len(), 1);
    let s2 = manifest::set_dependency(&s, &d(), Lang::Ru, "а", Some("^1.5.0"));
    assert_eq!(manifest::parse(&s2, &d()).unwrap().dependencies["а"], "^1.5.0");
    let s3 = manifest::set_dependency(&s2, &d(), Lang::Ru, "а", None);
    assert!(!manifest::parse(&s3, &d()).unwrap().dependencies.contains_key("а"));
    let s4 = manifest::set_dependency("[progetto]\nnome = \"x\"\n", &d(), Lang::It, "c", Some("^1.0.0"));
    assert!(s4.contains("[dipendenze]\nc = \"^1.0.0\""), "{s4}");
}

fn pkg(reg: &mut MemRegistry, name: &str, ver: &str, deps: &[(&str, &str)], body: &str) {
    let mut toml = format!("[проект]\nимя = \"{name}\"\nверсия = \"{ver}\"\nязык = \"ru\"\n\n[зависимости]\n");
    for (n, r) in deps {
        toml.push_str(&format!("\"{n}\" = \"{r}\"\n"));
    }
    let files = vec![
        ("vladx.toml".to_string(), toml),
        ("главная.vlstd".to_string(), body.to_string()),
        ("README.md".to_string(), "# пакет".to_string()),
        (".секрет".to_string(), "нет".to_string()),
        ("пакеты/чужое/главная.vlstd".to_string(), "скажи 1".to_string()),
    ];
    let (b, m) = bundle::pack(files, &d()).unwrap();
    assert!(!b.files.contains_key(".секрет") && !b.files.keys().any(|k| k.starts_with("пакеты/")));
    reg.publish(b, &m);
}

#[test]
fn install_resolve_lock_update_remove() {
    let mut reg = MemRegistry::default();
    pkg(&mut reg, "цвета", "1.0.0", &[], "пусть красный = 1");
    pkg(&mut reg, "цвета", "1.3.0", &[], "пусть красный = 2");
    pkg(&mut reg, "цвета", "2.0.0", &[], "пусть красный = 3");
    pkg(&mut reg, "@влад/кисть", "0.1.0", &[("цвета", "^1.0.0")], "пусть к = 1");
    let mut fs = MemFs::default();
    fs.write("vladx.toml", "[проект]\nимя = \"моё\"\nязык = \"ru\"\n").unwrap();
    let r = add(&mut fs, &mut reg, &d(), &["@влад/кисть".into()]).unwrap();
    assert_eq!(r.packages["цвета"], "1.3.0", "наибольшая подходящая");
    assert_eq!(r.packages["@влад/кисть"], "0.1.0");
    assert!(fs.files.contains_key("пакеты/@влад/кисть/главная.vlstd"));
    assert_eq!(fs.files["пакеты/цвета/главная.vlstd"], "пусть красный = 2");
    let lock = Lock::parse(&fs.files["vladx.lock"]).unwrap();
    assert_eq!(lock.packages["цвета"].version, "1.3.0");
    assert!(fs.files["vladx.toml"].contains("@влад/кисть = \"^0.1.0\""));
    // повторная установка ничего не качает
    let before = reg.fetches;
    let r2 = install(&mut fs, &mut reg, &d(), &BTreeSet::new(), false).unwrap();
    assert_eq!(reg.fetches, before);
    assert_eq!(r2.unchanged, 2);
    // новая версия в реестре: lock держит старую, update — берёт новую
    pkg(&mut reg, "цвета", "1.4.0", &[], "пусть красный = 4");
    let r3 = install(&mut fs, &mut reg, &d(), &BTreeSet::new(), false).unwrap();
    assert_eq!(r3.packages["цвета"], "1.3.0");
    let r4 = install(&mut fs, &mut reg, &d(), &BTreeSet::new(), true).unwrap();
    assert_eq!(r4.packages["цвета"], "1.4.0");
    assert_eq!(r4.updated, vec!["цвета 1.3.0 → 1.4.0".to_string()]);
    // прямая зависимость с другим диапазоном — конфликт
    let e = add(&mut fs, &mut reg, &d(), &["цвета@^2.0.0".into()]).unwrap_err();
    assert!(e.contains("конфликт"), "{e}");
    fs.write("vladx.toml", "[проект]\nимя = \"моё\"\nязык = \"ru\"\n\n[зависимости]\n@влад/кисть = \"^0.1.0\"\n").unwrap();
    // удаление: лишнее убирается из папки пакетов
    let r5 = remove(&mut fs, &mut reg, &d(), &["@влад/кисть".into()]).unwrap();
    assert_eq!(r5.removed.len(), 2);
    assert!(!fs.files.keys().any(|k| k.starts_with("пакеты/цвета/")));
    assert!(Lock::parse(&fs.files["vladx.lock"]).unwrap().packages.is_empty());
    // подмена пакета ловится
    let mut bad = reg.clone();
    bad.packages.get_mut("цвета").unwrap().1.get_mut("2.0.0").unwrap().files.insert("x.txt".into(), "вирус".into());
    let mut fs2 = MemFs::default();
    fs2.write("vladx.toml", "[progetto]\nnome = \"io\"\nlingua = \"it\"\n").unwrap();
    let e = add(&mut fs2, &mut bad, &d(), &["цвета@2".into()]).unwrap_err();
    assert!(e.contains("повреждён"), "{e}");
    // итальянский проект кладёт пакеты в pacchetti/
    let r6 = add(&mut fs2, &mut reg, &d(), &["цвета@2".into()]).unwrap();
    assert_eq!(r6.packages["цвета"], "2.0.0");
    assert!(fs2.files.contains_key("pacchetti/цвета/главная.vlstd"));
}

#[test]
fn pack_validates() {
    let toml = "[проект]\nимя = \"x\"\nверсия = \"1.0.0\"\n".to_string();
    let e = bundle::pack(vec![("vladx.toml".into(), toml.clone())], &d()).unwrap_err();
    assert!(e.contains("главная.vlstd"), "{e}");
    let e = bundle::pack(vec![("vladx.toml".into(), toml.clone()), ("главная.vlstd".into(), "если то".into())], &d()).unwrap_err();
    assert!(e.starts_with("главная.vlstd:1:"), "{e}");
    let e = bundle::pack(vec![("vladx.toml".into(), "[проект]\nимя = \"X\"\nверсия = \"1.0.0\"\n".into())], &d()).unwrap_err();
    assert!(e.contains("строчными"), "{e}");
    let ok = bundle::pack(vec![("vladx.toml".into(), toml), ("главная.vlstd".into(), "скажи 1".into())], &d()).unwrap();
    ok.0.verify().unwrap();
}

#[test]
fn bump_and_set_field() {
    assert_eq!(semver::bump("1.2.3", "patch").unwrap(), "1.2.4");
    assert_eq!(semver::bump("1.2.3", "minor").unwrap(), "1.3.0");
    assert_eq!(semver::bump("1.2.3", "major").unwrap(), "2.0.0");
    assert_eq!(semver::bump("1.3.0-бета.1", "patch").unwrap(), "1.3.0");
    let src = "# мой пакет\n[проект]\nимя = \"x\"\nверсия = \"1.0.0\" # сюда\n\n[зависимости]\n";
    let s = manifest::set_project_field(src, &d(), Lang::Ru, "version", "1.0.1");
    assert!(s.contains("# мой пакет") && s.contains("версия = \"1.0.1\""), "{s}");
    assert_eq!(manifest::parse(&s, &d()).unwrap().version.as_deref(), Some("1.0.1"));
    let s = manifest::set_project_field(src, &d(), Lang::Ru, "description", "Строки");
    let m = manifest::parse(&s, &d()).unwrap();
    assert_eq!((m.description.as_deref(), m.version.as_deref()), (Some("Строки"), Some("1.0.0")));
    let s = manifest::set_project_field("[dipendenze]\n", &d(), Lang::It, "name", "io");
    assert!(s.starts_with("[progetto]\nnome = \"io\"\n"), "{s}");
    let s = manifest::set_project_field("[progetto]\nnome = \"io\"\n[dipendenze]\na = \"1\"\n", &d(), Lang::It, "version", "2.0.0");
    assert_eq!(manifest::parse(&s, &d()).unwrap().version.as_deref(), Some("2.0.0"), "{s}");
}
