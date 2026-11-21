//! vladpm — менеджер пакетов VladX (как npm). Ядро — `vladx-pm`, здесь — диск,
//! HTTP к реестру, ~/.vladpmrc и команды. Вызывается как `vladpm …` и `vladx pm …`.

use crate::host::agent_for;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use vladx_pm::{Bundle, Fs, Packument, Registry};
use vladx_syntax::{Dictionary, Lang};

pub const USAGE: &str = "vladpm — менеджер пакетов VladX (как npm). То же: vladx pm …

  vladpm init [папка] [--lang ru|it|scn]   новый проект с vladx.toml
  vladpm install | i                       поставить всё из vladx.toml (держится за vladx.lock)
  vladpm add <имя>[@диапазон] …            добавить зависимость: vladpm add @влад/строки@^1.2
  vladpm remove | rm <имя> …               убрать зависимость
  vladpm update | up [имя …]               обновить в пределах диапазонов
  vladpm list | ls                         что установлено
  vladpm run <скрипт> [-- аргументы]       скрипт из раздела [скрипты]; без имени — список
  vladpm search <слово>                    поиск в реестре
  vladpm info <имя>                        карточка пакета: версии, описание, README
  vladpm adduser | login | logout | whoami учётная запись реестра (токен — в ~/.vladpmrc)
  vladpm publish                           опубликовать проект как пакет
  vladpm unpublish <имя>[@версия]          снять версию (в течение 72 часов)
  vladpm deprecate <имя>[@диапазон] \"текст\" пометить устаревшим (\"\" — снять пометку)

  --registry <адрес>   реестр (иначе VLADPM_REGISTRY, поле «реестр» в vladx.toml,
                       ~/.vladpmrc; по умолчанию https://vladinc.ru/vladx)
  пакеты ставятся в папку на языке проекта: пакеты/ · pacchetti/";

struct Args {
    cmd: String,
    pos: Vec<String>,
    rest: Vec<String>,
    registry: Option<String>,
    lang: Option<Lang>,
}

fn parse_args(argv: Vec<String>) -> Result<Args, String> {
    let mut it = argv.into_iter();
    let cmd = it.next().ok_or_else(|| USAGE.to_string())?;
    let mut a = Args { cmd, pos: vec![], rest: vec![], registry: None, lang: None };
    while let Some(x) = it.next() {
        match x.as_str() {
            "--" => {
                a.rest = it.by_ref().collect();
            }
            "--registry" => a.registry = Some(it.next().ok_or("после --registry нужен адрес")?),
            "--lang" => {
                let v = it.next().ok_or("после --lang нужен язык")?;
                a.lang = Some(Lang::from_code(&v).ok_or_else(|| format!("неизвестный язык «{v}»: ru, it или scn"))?);
            }
            "-h" | "--help" => return Err(USAGE.to_string()),
            s if s.starts_with("--") && a.cmd != "run" => return Err(format!("неизвестный флаг {s}\n\n{USAGE}")),
            s => a.pos.push(s.to_string()),
        }
    }
    Ok(a)
}

// ───────────── ~/.vladpmrc ─────────────

fn rc_path() -> PathBuf {
    if let Ok(p) = std::env::var("VLADPM_CONFIG") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".vladpmrc")
}

fn rc_load() -> Value {
    std::fs::read_to_string(rc_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| json!({}))
}

fn rc_save(v: &Value) -> Result<(), String> {
    let p = rc_path();
    std::fs::write(&p, serde_json::to_string_pretty(v).unwrap_or_default() + "\n").map_err(|e| format!("{} не записан: {e}", p.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

// ───────────── HTTP-реестр ─────────────

/// Адрес реестра → адрес его PHP-точки.
pub fn endpoint(registry: &str) -> String {
    let r = registry.trim_end_matches('/');
    if r.ends_with(".php") {
        r.to_string()
    } else {
        format!("{r}/vladpm-registry.php")
    }
}

fn enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

pub struct HttpRegistry {
    pub url: String,
    pub token: Option<String>,
}

impl HttpRegistry {
    fn answer(r: Result<ureq::Response, ureq::Error>) -> Result<Value, String> {
        let (code, body) = match r {
            Ok(r) => (r.status(), r.into_string().map_err(|e| format!("ответ реестра не прочитан: {e}"))?),
            Err(ureq::Error::Status(c, r)) => (c, r.into_string().unwrap_or_default()),
            Err(e) => return Err(format!("реестр недоступен: {e}")),
        };
        let v: Value = serde_json::from_str(&body).map_err(|_| {
            format!("реестр ответил не JSON (код {code}): {}", body.chars().take(200).collect::<String>())
        })?;
        if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
            return Err(e.to_string());
        }
        if code >= 400 {
            return Err(format!("реестр ответил {code}"));
        }
        Ok(v)
    }

    pub fn get(&self, op: &str, params: &[(&str, &str)]) -> Result<Value, String> {
        let mut u = format!("{}?op={op}", self.url);
        for (k, v) in params {
            u.push_str(&format!("&{k}={}", enc(v)));
        }
        let mut req = agent_for(&u, 30).get(&u);
        if let Some(t) = &self.token {
            req = req.set("Authorization", &format!("Bearer {t}")).set("X-VladPM-Token", t);
        }
        Self::answer(req.call())
    }

    pub fn post(&self, op: &str, body: Value) -> Result<Value, String> {
        let u = format!("{}?op={op}", self.url);
        let mut req = agent_for(&u, 60).post(&u).set("Content-Type", "application/json").set("X-VladPM", "1");
        if let Some(t) = &self.token {
            req = req.set("Authorization", &format!("Bearer {t}")).set("X-VladPM-Token", t);
        }
        Self::answer(req.send_string(&body.to_string()))
    }
}

impl Registry for HttpRegistry {
    fn packument(&mut self, name: &str) -> Result<Packument, String> {
        let v = self.get("info", &[("name", name)])?;
        serde_json::from_value(v).map_err(|e| format!("карточка «{name}» не прочитана: {e}"))
    }
    fn fetch(&mut self, name: &str, version: &str) -> Result<Bundle, String> {
        let v = self.get("fetch", &[("name", name), ("version", version)])?;
        serde_json::from_value(v).map_err(|e| format!("пакет {name}@{version} не прочитан: {e}"))
    }
}

// ───────────── диск проекта ─────────────

pub struct DiskFs {
    pub root: PathBuf,
}

impl DiskFs {
    fn p(&self, path: &str) -> PathBuf {
        let mut p = self.root.clone();
        for s in path.split('/') {
            p.push(s);
        }
        p
    }
}

impl Fs for DiskFs {
    fn read(&mut self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.p(path)).ok()
    }
    fn write(&mut self, path: &str, content: &str) -> Result<(), String> {
        let p = self.p(path);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("папка {} не создана: {e}", d.display()))?;
        }
        std::fs::write(&p, content).map_err(|e| format!("{} не записан: {e}", p.display()))
    }
    fn remove_dir(&mut self, path: &str) -> Result<(), String> {
        let p = self.p(path);
        match std::fs::remove_dir_all(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{} не удалена: {e}", p.display())),
            _ => {
                // пустая папка области @автор/ тоже уходит
                if let Some(parent) = p.parent() {
                    if parent.file_name().is_some_and(|n| n.to_string_lossy().starts_with('@')) {
                        let _ = std::fs::remove_dir(parent);
                    }
                }
                Ok(())
            }
        }
    }
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>, skip: &[String]) -> Result<(), String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Ok(()) };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let path = e.path();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if dir == root && (skip.contains(&name) || matches!(name.as_str(), "target" | "vladx-files" | "node_modules" | "vladx_modules")) {
                continue;
            }
            walk(&path, root, out, skip)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            let rel = rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/");
            if !vladx_pm::bundle::publishable(&rel, skip) {
                continue;
            }
            let text = std::fs::read_to_string(&path).map_err(|_| format!("«{rel}» — не текст в UTF-8; в пакет попадают только текстовые файлы"))?;
            out.push((rel, text));
        }
    }
    Ok(())
}

// ───────────── команды ─────────────

struct Ctx {
    dict: Dictionary,
    root: Option<PathBuf>,
    manifest: Option<vladx_pm::Manifest>,
    registry: String,
    rc: Value,
}

impl Ctx {
    fn new(a: &Args) -> Result<Ctx, String> {
        let dict = Dictionary::builtin();
        let proj = crate::project::find(Path::new(".")).transpose()?;
        let rc = rc_load();
        let registry = a
            .registry
            .clone()
            .or_else(|| std::env::var("VLADPM_REGISTRY").ok().filter(|s| !s.is_empty()))
            .or_else(|| proj.as_ref().and_then(|p| p.manifest.registry.clone()))
            .or_else(|| rc.get("registry").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .unwrap_or_else(|| vladx_pm::DEFAULT_REGISTRY.to_string());
        Ok(Ctx { dict, root: proj.as_ref().map(|p| p.root.clone()), manifest: proj.map(|p| p.manifest), registry, rc })
    }

    fn reg(&self) -> HttpRegistry {
        let url = endpoint(&self.registry);
        let token = self.rc.get("tokens").and_then(|t| t.get(&url)).and_then(|t| t.get("token")).and_then(|t| t.as_str()).map(|s| s.to_string());
        HttpRegistry { url, token }
    }

    fn fs(&self) -> Result<DiskFs, String> {
        match &self.root {
            Some(r) => Ok(DiskFs { root: r.clone() }),
            None => Err("здесь нет vladx.toml — создайте проект: vladpm init".into()),
        }
    }

    fn need_login(&self) -> Result<HttpRegistry, String> {
        let r = self.reg();
        if r.token.is_none() {
            return Err(format!("вы не вошли в реестр {} — vladpm login (или vladpm adduser)", self.registry));
        }
        Ok(r)
    }
}

fn prompt(q: &str, secret: bool) -> Result<String, String> {
    eprint!("{q}");
    let _ = std::io::stderr().flush();
    let tty = std::io::stdin().is_terminal();
    #[cfg(unix)]
    let hide = secret && tty && std::process::Command::new("stty").arg("-echo").stdin(std::process::Stdio::inherit()).status().is_ok();
    #[cfg(not(unix))]
    let hide = { let _ = (secret, tty); false };
    let mut s = String::new();
    let r = std::io::stdin().lock().read_line(&mut s);
    #[cfg(unix)]
    if hide {
        let _ = std::process::Command::new("stty").arg("echo").stdin(std::process::Stdio::inherit()).status();
        eprintln!();
    }
    let _ = hide;
    r.map_err(|e| e.to_string())?;
    Ok(s.trim_end_matches(['\n', '\r']).to_string())
}

fn credentials() -> Result<(String, String), String> {
    let user = match std::env::var("VLADPM_USER") {
        Ok(u) if !u.is_empty() => u,
        _ => prompt("имя пользователя: ", false)?,
    };
    let pass = match std::env::var("VLADPM_PASSWORD") {
        Ok(p) if !p.is_empty() => p,
        _ => prompt("пароль: ", true)?,
    };
    if user.trim().is_empty() || pass.is_empty() {
        return Err("нужны имя и пароль".into());
    }
    Ok((user.trim().to_string(), pass))
}

fn save_token(c: &mut Ctx, url: &str, user: &str, token: &str) -> Result<(), String> {
    if !c.rc.is_object() {
        c.rc = json!({});
    }
    let tokens = c.rc.as_object_mut().unwrap().entry("tokens").or_insert_with(|| json!({}));
    tokens[url] = json!({ "user": user, "token": token });
    rc_save(&c.rc)
}

fn print_report(r: &vladx_pm::Report) {
    print!("{}", r.summary());
}

fn show_list(c: &Ctx) -> Result<bool, String> {
    let mut fs = c.fs()?;
    let m = c.manifest.clone().unwrap_or_default();
    let lock = fs.read(vladx_pm::lock::FILE).map(|s| vladx_pm::Lock::parse(&s)).transpose()?.unwrap_or_default();
    println!("{}@{}", m.name.as_deref().unwrap_or("(проект)"), m.version.as_deref().unwrap_or("0.0.0"));
    let mdir = vladx_pm::manifest::modules_dir(&c.dict, m.lang.unwrap_or(Lang::Ru));
    fn tree(lock: &vladx_pm::Lock, deps: &BTreeMap<String, String>, depth: usize, seen: &mut BTreeSet<String>, fs: &mut DiskFs, mdir: &str) {
        let n = deps.len();
        for (i, (name, range)) in deps.iter().enumerate() {
            let last = i + 1 == n;
            let pre = format!("{}{}", "│  ".repeat(depth), if last { "└─ " } else { "├─ " });
            match lock.packages.get(name) {
                Some(e) => {
                    let missing = fs.read(&format!("{mdir}/{name}/vladx.toml")).is_none();
                    println!("{pre}{name}@{}{}", e.version, if missing { "  (не установлен — vladpm install)" } else { "" });
                    if seen.insert(name.clone()) {
                        tree(lock, &e.dependencies, depth + 1, seen, fs, mdir);
                    }
                }
                None => println!("{pre}{name} \"{range}\"  (не установлен — vladpm install)"),
            }
        }
    }
    if m.dependencies.is_empty() {
        println!("└─ (зависимостей нет)");
    }
    tree(&lock, &m.dependencies, 0, &mut BTreeSet::new(), &mut fs, &mdir);
    Ok(true)
}

fn run_script(c: &Ctx, a: &Args) -> Result<bool, String> {
    let m = c.manifest.clone().ok_or("здесь нет vladx.toml")?;
    let Some(name) = a.pos.first() else {
        if m.scripts.is_empty() {
            println!("скриптов нет: добавьте раздел [{}] в vladx.toml", c.dict.manifest_word(m.lang.unwrap_or(Lang::Ru), "scripts"));
        }
        for (k, v) in &m.scripts {
            println!("  {k}\n    {v}");
        }
        return Ok(true);
    };
    let cmd = m.scripts.get(name).ok_or_else(|| {
        let have: Vec<&String> = m.scripts.keys().collect();
        format!("скрипта «{name}» нет; есть: {}", if have.is_empty() { "ничего".into() } else { have.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ") })
    })?;
    let mut extra: Vec<String> = a.pos[1..].to_vec();
    extra.extend(a.rest.iter().cloned());
    let full = if extra.is_empty() { cmd.clone() } else { format!("{cmd} {}", extra.join(" ")) };
    eprintln!("> {full}");
    // PATH: сначала папка, где лежит vladpm (там же vladx), как node_modules/.bin у npm
    let mut path = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())).map(|p| vec![p]).unwrap_or_default();
    path.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    let path = std::env::join_paths(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let mut command = {
        let mut x = std::process::Command::new("cmd");
        x.arg("/C").arg(&full);
        x
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut x = std::process::Command::new("sh");
        x.arg("-c").arg(&full);
        x
    };
    let st = command
        .current_dir(c.root.clone().unwrap_or_else(|| PathBuf::from(".")))
        .env("PATH", path)
        .env("VLADPM_SCRIPT", name)
        .status()
        .map_err(|e| format!("скрипт не запущен: {e}"))?;
    Ok(st.success())
}

fn info(c: &Ctx, name: &str) -> Result<bool, String> {
    let r = c.reg();
    let v = r.get("info", &[("name", name)])?;
    let p: Packument = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    let latest = p.latest().unwrap_or_default();
    println!("{}@{latest}", p.name);
    if let Some(d) = p.description.as_ref().or_else(|| p.versions.get(&latest).and_then(|m| m.description.as_ref())) {
        println!("{d}");
    }
    if let Some(l) = p.versions.get(&latest).and_then(|m| m.lang.clone()) {
        println!("язык: {l}");
    }
    if !p.owners.is_empty() {
        println!("владельцы: {}", p.owners.join(", "));
    }
    let deps = p.versions.get(&latest).map(|m| m.dependencies.clone()).unwrap_or_default();
    if !deps.is_empty() {
        println!("зависимости:");
        for (k, v) in deps {
            println!("  {k} {v}");
        }
    }
    println!("версии:");
    let mut vs: Vec<(&String, &vladx_pm::VersionMeta)> = p.versions.iter().collect();
    vs.sort_by(|a, b| {
        let pa = vladx_pm::semver::Version::parse(a.0).ok();
        let pb = vladx_pm::semver::Version::parse(b.0).ok();
        pb.cmp(&pa)
    });
    for (ver, m) in vs {
        match &m.deprecated {
            Some(d) => println!("  {ver}  — устарел: {d}"),
            None => println!("  {ver}"),
        }
    }
    println!("установка: vladpm add {}", p.name);
    if let Some(readme) = v.get("readme").and_then(|r| r.as_str()).filter(|r| !r.is_empty()) {
        println!("\n{readme}");
    }
    Ok(true)
}

pub fn main_from(argv: Vec<String>) -> Result<bool, String> {
    let a = parse_args(argv)?;
    if matches!(a.cmd.as_str(), "help" | "-h" | "--help") {
        println!("{USAGE}");
        return Ok(true);
    }
    if a.cmd == "init" {
        let dict = Dictionary::builtin();
        let dir = PathBuf::from(a.pos.first().cloned().unwrap_or_else(|| ".".into()));
        for m in crate::project::init(&dir, a.lang.unwrap_or(dict.default_lang), &dict)? {
            println!("создан {}", m.display());
        }
        return Ok(true);
    }
    let mut c = Ctx::new(&a)?;
    match a.cmd.as_str() {
        "install" | "i" | "ci" if a.pos.is_empty() => {
            let mut fs = c.fs()?;
            let r = vladx_pm::install(&mut fs, &mut c.reg(), &c.dict, &BTreeSet::new(), false)?;
            print_report(&r);
            Ok(true)
        }
        "install" | "i" | "add" => {
            if a.pos.is_empty() {
                return Err("какой пакет добавить? vladpm add <имя>".into());
            }
            let mut fs = c.fs()?;
            let r = vladx_pm::add(&mut fs, &mut c.reg(), &c.dict, &a.pos)?;
            print_report(&r);
            Ok(true)
        }
        "remove" | "rm" | "uninstall" | "un" => {
            if a.pos.is_empty() {
                return Err("какой пакет убрать? vladpm remove <имя>".into());
            }
            let mut fs = c.fs()?;
            let r = vladx_pm::remove(&mut fs, &mut c.reg(), &c.dict, &a.pos)?;
            print_report(&r);
            Ok(true)
        }
        "update" | "up" | "upgrade" => {
            let mut fs = c.fs()?;
            let names: BTreeSet<String> = a.pos.iter().cloned().collect();
            let all = names.is_empty();
            let r = vladx_pm::install(&mut fs, &mut c.reg(), &c.dict, &names, all)?;
            print_report(&r);
            Ok(true)
        }
        "list" | "ls" => show_list(&c),
        "run" | "run-script" => run_script(&c, &a),
        "search" | "s" | "find" => {
            let q = a.pos.join(" ");
            let v = c.reg().get("search", &[("q", &q)])?;
            let res = v.get("results").and_then(|r| r.as_array()).cloned().unwrap_or_default();
            if res.is_empty() {
                println!("ничего не найдено");
            }
            for p in res {
                let s = |k: &str| p.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                println!("{}@{}  {}{}", s("name"), s("version"), s("description"), if s("deprecated").is_empty() { "" } else { "  (устарел)" });
            }
            Ok(true)
        }
        "info" | "view" | "show" => info(&c, a.pos.first().ok_or("какой пакет? vladpm info <имя>")?),
        "adduser" | "signup" | "register" => {
            let r = c.reg();
            let (user, pass) = credentials()?;
            let v = r.post("adduser", json!({ "user": user, "password": pass }))?;
            let token = v["token"].as_str().ok_or("реестр не выдал токен")?.to_string();
            let user = v["user"].as_str().unwrap_or(&user).to_string();
            save_token(&mut c, &r.url, &user, &token)?;
            println!("зарегистрирован «{user}» в {}; токен сохранён в {}", c.registry, rc_path().display());
            Ok(true)
        }
        "login" => {
            let r = c.reg();
            let (user, pass) = credentials()?;
            let v = r.post("login", json!({ "user": user, "password": pass }))?;
            let token = v["token"].as_str().ok_or("реестр не выдал токен")?.to_string();
            let user = v["user"].as_str().unwrap_or(&user).to_string();
            save_token(&mut c, &r.url, &user, &token)?;
            println!("вход выполнен: «{user}» в {}", c.registry);
            Ok(true)
        }
        "logout" => {
            let r = c.reg();
            if r.token.is_some() {
                let _ = r.post("logout", json!({}));
            }
            if let Some(t) = c.rc.get_mut("tokens").and_then(|t| t.as_object_mut()) {
                t.remove(&r.url);
            }
            rc_save(&c.rc)?;
            println!("выход выполнен");
            Ok(true)
        }
        "whoami" => {
            let r = c.need_login()?;
            let v = r.get("whoami", &[])?;
            println!("{}", v["user"].as_str().unwrap_or("?"));
            Ok(true)
        }
        "publish" => {
            let r = c.need_login()?;
            let root = c.root.clone().ok_or("здесь нет vladx.toml — нечего публиковать")?;
            let mdirs = vladx_pm::manifest::modules_dir_names(&c.dict);
            let mut files = vec![];
            walk(&root, &root, &mut files, &mdirs)?;
            let (b, m) = vladx_pm::bundle::pack(files, &c.dict)?;
            let readme = b.files.iter().find(|(k, _)| k.to_uppercase().starts_with("README")).map(|(_, v)| v.clone()).unwrap_or_default();
            let meta = json!({
                "dependencies": m.dependencies,
                "description": m.description,
                "lang": m.lang.unwrap_or(Lang::Ru).code(),
                "main": m.main,
                "license": m.license,
                "author": m.author,
                "readme": readme,
            });
            let n = b.files.len();
            let kb = b.size().div_ceil(1024);
            let v = r.post("publish", json!({ "bundle": b, "meta": meta }))?;
            println!("+ {}@{} ({n} файл., {kb} КБ) → {}", b.name, b.version, c.registry);
            if let Some(u) = v.get("page").and_then(|p| p.as_str()) {
                println!("страница: {u}");
            }
            Ok(true)
        }
        "unpublish" => {
            let r = c.need_login()?;
            let spec = a.pos.first().ok_or("что снять? vladpm unpublish <имя>@<версия>")?;
            let (name, ver) = vladx_pm::install::split_spec(spec);
            let v = r.post("unpublish", json!({ "name": name, "version": ver }))?;
            let gone: Vec<String> = v["removed"].as_array().cloned().unwrap_or_default().iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect();
            println!("- {name}: снято {}", gone.join(", "));
            Ok(true)
        }
        "deprecate" => {
            let r = c.need_login()?;
            let spec = a.pos.first().ok_or("vladpm deprecate <имя>[@диапазон] \"почему\"")?;
            let msg = a.pos.get(1).cloned().ok_or("нужен текст: vladpm deprecate имя \"используйте другое\" (\"\" — снять)")?;
            let (name, range) = vladx_pm::install::split_spec(spec);
            let mut reg = c.reg();
            let p = reg.packument(&name)?;
            let rg = vladx_pm::semver::Range::parse(range.as_deref().unwrap_or("*"))?;
            let versions: Vec<String> = p
                .versions
                .keys()
                .filter(|v| vladx_pm::semver::Version::parse(v).map(|x| rg.matches(&x) || range.as_deref() == Some(v.as_str())).unwrap_or(false))
                .cloned()
                .collect();
            if versions.is_empty() {
                return Err(format!("у «{name}» нет версий под «{}»", range.unwrap_or_default()));
            }
            r.post("deprecate", json!({ "name": name, "versions": versions, "message": msg }))?;
            println!("{} {name}: {}", if msg.is_empty() { "пометка снята" } else { "помечено устаревшим" }, versions.join(", "));
            Ok(true)
        }
        other => Err(format!("неизвестная команда «{other}»\n\n{USAGE}")),
    }
}

/// Для `vladx pm …` и `vladpm …`: печать ошибок и код выхода.
pub fn exit(argv: Vec<String>) -> std::process::ExitCode {
    match main_from(argv) {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::from(1),
        Err(e) => {
            eprintln!("{e}");
            std::process::ExitCode::from(1)
        }
    }
}
