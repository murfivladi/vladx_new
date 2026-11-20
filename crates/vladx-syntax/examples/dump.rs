//! Отладка: `cargo run -p vladx-syntax --example dump <файл> [ru|it|scn]`.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let src = std::fs::read_to_string(&a[1]).unwrap();
    let lang = vladx_syntax::Lang::from_code(a.get(2).map(|s| s.as_str()).unwrap_or("ru")).unwrap();
    let d = vladx_syntax::Dictionary::builtin();
    match vladx_syntax::parse(&src, lang, &d) {
        Ok(p) => println!("{}", p.to_json_pretty()),
        Err(e) => print!("{}", e.render(&a[1], &src)),
    }
}
