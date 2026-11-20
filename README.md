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

А можно и без кода — файл `.vlhmn` переводит в ту же программу ИИ:

```text
Сделай список покупок: хлеб, молоко и сыр.
Потом выведи каждый пункт с тире в начале.
```

## Скачать

Готовый `vladx` собирается на GitHub при каждом push и лежит в релизе **nightly**:
<https://github.com/murfivladi/vladx_new/releases/tag/nightly>

| Система | Файл |
|---|---|
| Linux x86_64 (статический) | `vladx-x86_64-unknown-linux-musl.tar.gz` |
| macOS Apple Silicon | `vladx-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `vladx-x86_64-apple-darwin.tar.gz` |
| Windows x86_64 | `vladx-x86_64-pc-windows-msvc.zip` |
| Площадка и ядро WASM | `vladx-playground.html`, `vladx.wasm` |

Распакуйте архив и запустите `vladx run examples/ru/01-hello.vlstd`. На macOS при первом запуске может понадобиться `xattr -d com.apple.quarantine vladx`. Тег `v*` (например, `v0.1.0`) создаёт постоянный релиз с тем же набором файлов. Сборкой управляет `.github/workflows/release.yml`.

## Сборка из исходников

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
vladx fmt <файл> [--to ru|it|scn] [--write]                  канонический вид / перевод на другой язык
vladx serve [--port 8000]                                    площадка, прокси и ИИ для .vlhmn
vladx hmn-eval <папка>                                       оценка перевода .vlhmn на наборе фраз
```

Для `.vlhmn` нужен ключ Polza.ai:

```bash
export POLZA_API_KEY=…
```

```bash
vladx run examples/ru/14-shopping.vlhmn
```

Перед запуском `vladx` показывает, как ИИ понял текст, и не запускает программу, если уверенность ниже 0.8. Переводы кэшируются: повторный запуск запросов не тратит. Модель меняется переменной `VLADX_LLM_MODEL` (по умолчанию `anthropic/claude-haiku-5.5`).

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
  vladx-hmn       .vlhmn: промпт, проверка ответа ИИ, повтор, кэш
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

Импорт файлов проекта, `vladx.toml`, подсветка синтаксиса. Слова `it` и `scn` ждут проверки носителем. Подробнее — `spec/SPEC.md`, §13.

Лицензия — MIT.
