# VladX

Интерпретируемый язык, где код читается как фраза на живом языке. Ключевые слова — на русском, итальянском или сицилийском; один и тот же код на трёх языках даёт одинаковое нейтральное дерево (AST). Ядро написано на Rust и компилируется в WebAssembly: в браузере работает всё, включая настоящие файлы и сеть.

```vladx
пусть имя будет "мир"
скажи "привет, {имя}"

функция сумма(a, b) {
  верни a + b
}

если сумма(2, 3) больше 4 то {
  скажи "работает"
}
```

```vladx
sia nome = "mondo"
dì "ciao, {nome}"
```

## Быстрый старт

```bash
cargo build --release
```

```bash
target/release/vladx run examples/ru/01-hello.vlstd
```

```bash
target/release/vladx run examples/it/01-ciao.vlstd --lang it
```

Площадка в браузере (редактор, примеры, файлы, сеть):

```bash
bash scripts/serve.sh
```

и откройте <http://localhost:8000/site/playground.html>. Страницу нужно открывать через `http://`, а не как файл: браузер не даёт странице с `file://` фоновый поток и хранилище, где работает ядро.

## Команды

```
vladx run <файл> [--lang ru|it|scn] [--no-net] [--no-files] [--restrict-files]
vladx parse <файл> [--lang ru|it|scn] [--keywords <путь>]    AST в JSON
vladx check <файл> [--lang ru|it|scn]                        только диагностика
vladx keywords [--lang ru|it|scn]                            словарь форм
vladx serve [--port 8000]                                    площадка и прокси
```

По умолчанию открыто всё, как в Node: любые файлы и сеть без флагов. Сузить: `--no-files`, `--no-net`, `--restrict-files` (только папка программы).

## Проверки

```bash
cargo test
```

```bash
bash scripts/test-all.sh
```

Скрипт прогоняет `cargo test`, проверку словаря, все примеры, схему AST, негативные случаи, флаги файлов и сети и печатает «ВСЁ ЗЕЛЁНОЕ». Браузерный сценарий в Chromium (OPFS, перезагрузка, `спроси`, сеть напрямую и через прокси):

```bash
node tests/browser/files-net.mjs
```

## Устройство

```
crates/
  vladx-ast       нейтральный AST (serde) и диагностика
  vladx-syntax    словарь → лексер → парсер → статические проверки
  vladx-runtime   интерпретатор, stdlib (9 модулей), трейт Host
  vladx-wasm      C-ABI для браузера (без wasm-bindgen)
  vladx-cli       vladx run/parse/check/keywords/serve
  xtask           проверка словаря: cargo run -p xtask
spec/
  keywords.toml   единственный источник ключевых слов
  SPEC.md         спецификация языка
  ast.schema.json контракт AST (JSON Schema 2020-12)
site/             площадка: один HTML со встроенным WASM
examples/         примеры на ru, it, scn (проверяются тестами)
tests/            негативные случаи и браузерный сценарий
docs/HANDOVER.md  исходное техническое задание
```

Слова языка живут только в `spec/keywords.toml`; в парсере нет ни одной словоформы (это проверяет тест). Добавить синоним — добавить строку в словарь.

## Чего пока нет

`.vlhmn` (свободный текст → LLM → AST), импорт файлов проекта, форматтер, `vladx.toml`, подсветка синтаксиса. Слова `it` и `scn` ждут проверки носителем. Подробнее — `spec/SPEC.md`, §11.

Лицензия — MIT.
