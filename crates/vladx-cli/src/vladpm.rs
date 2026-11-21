//! vladpm — менеджер пакетов VladX. То же самое: `vladx pm …`.
#![allow(dead_code)]

#[path = "host.rs"]
mod host;
#[path = "pm.rs"]
mod pm;
#[path = "project.rs"]
mod project;

fn main() -> std::process::ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        eprintln!("{}", pm::USAGE);
        return std::process::ExitCode::from(2);
    }
    pm::exit(argv)
}
