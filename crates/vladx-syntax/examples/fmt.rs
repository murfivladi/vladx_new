//! Отладка: `cargo run -p vladx-syntax --example fmt <файл> <из> <в>` — печать без проверки.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let src = std::fs::read_to_string(&a[1]).unwrap();
    let from = vladx_syntax::Lang::from_code(&a[2]).unwrap();
    let to = vladx_syntax::Lang::from_code(&a[3]).unwrap();
    let d = vladx_syntax::Dictionary::builtin();
    let (p, t) = vladx_syntax::parse_with_trivia(&src, from, &d).unwrap();
    print!("{}", vladx_syntax::printer::print_program(&p, &d, to, Some(&t)).unwrap());
}
