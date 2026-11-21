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

В архивах для систем — `vladx` и `vladpm`.

| Система | Файл |
|---|---|
| Linux x86_64 (статический) | `vladx-x86_64-unknown-linux-musl.tar.gz` |
| macOS Apple Silicon | `vladx-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `vladx-x86_64-apple-darwin.tar.gz` |
| Windows x86_64 | `vladx-x86_64-pc-windows-msvc.zip` |
| Площадка и ядро WASM | `vladx-playground.html`, `vladx.wasm` |
| Для shared-хостинга (площадка «как Node» + PHP: файлы, сеть, ИИ, реестр пакетов) | `vladx-shared.zip` |

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

и откройте <http://localhost:8000/site/playground.html> — площадка устроена как IDE: проводник с примерами и файлами, вкладки, подсветка синтаксиса и живая проверка ошибок, терминал, где программа задаёт вопросы `спроси`, а ответ вводится прямо в строке (Enter), панели «Проблемы» и «AST». Горячие клавиши: Ctrl+Enter — запуск, Esc — остановить, Ctrl+S — сохранить, Shift+Alt+F — формат. Страницу нужно открывать через `http://`, а не как файл: браузер не даёт странице с `file://` фоновый поток и хранилище, где работает ядро.

Проект из нескольких файлов — как в Node:

```bash
vladx init мой-проект
```

```bash
cd мой-проект && vladx run
```

`vladx.toml` задаёт язык, точку входа, права и зависимости — разделы пишутся на языке проекта (`[проект]`, `[зависимости]`; по-итальянски `[progetto]`, `[dipendenze]`). Свои файлы подключаются с `./`, как в Node: `из "./утилиты" возьми приветствие`.

## Стандартная библиотека

Устроена как в Node.js: модули написаны на самом VladX (`crates/vladx-runtime/stdlib/*.vlstd`), а на Rust — только маленькое `ядро` для файлов, сети, времени и горячих операций. Модули подключаются явно, имена — на языке программы:

```vladx
импортируй списки
из "vladx:строки" возьми верхний

скажи списки.отбери([1, 2, 3, 4], функция(x) {
  верни x % 2 равно 0
})
скажи верхний("готово")
```

```vladx
importa liste
dì liste.ordina([3, 1, 2])
```

Модули: `числа`, `строки`, `списки`, `словари`, `случай`, `время`, `json`, `файлы`, `сеть`, `путь`, `события`, `проверка` (по-итальянски `numeri`, `testo`, `liste`, …). Прежние `numeri.корень(…)` без подключения работают, но с предупреждением. Подробно — `spec/SPEC.md`, §6.

## Пакеты: vladpm

Менеджер пакетов, как npm, — `vladpm` (или `vladx pm`), реестр и его сайт — на <https://vladinc.ru/vladx/vladpm.html>:

```bash
vladpm add @влад/строки
```

```vladx
из "@влад/строки" возьми крик
скажи крик("привет")
```

Пакет ложится в `пакеты/` (в итальянском проекте — `pacchetti/`), точные версии — в `vladx.lock`. Свой пакет: `vladpm adduser`, затем `vladpm publish`. Скрипты из раздела `[скрипты]` запускает `vladpm run имя`. В площадке есть панель «Пакеты» (📦): поиск по реестру и установка в хранилище страницы, а ещё публикация без командной строки — регистрация, «vladx.toml» (создаёт пакет), «Опубликовать» и «↑ версия».

```
vladpm init [папка] [--lang …]          новый проект
vladpm install | i                      поставить всё из vladx.toml (держится за vladx.lock)
vladpm add имя[@диапазон] …             добавить зависимость (semver: ^1.2, ~1.2.3, >=1 <2, 1.x)
vladpm remove | rm имя …                убрать
vladpm update | up [имя …]              обновить в пределах диапазонов
vladpm list | ls                        дерево установленного
vladpm run скрипт [-- аргументы]        скрипт из [скрипты]
vladpm search слово · info имя          реестр
vladpm adduser | login | logout | whoami
vladpm publish · unpublish имя@версия (72 ч) · deprecate имя@диапазон "почему"
```

Реестр по умолчанию — `https://vladinc.ru/vladx`; свой — `--registry`, `VLADPM_REGISTRY` или поле «реестр» в `vladx.toml`.

## Команды

```
vladx run [файл] [--lang ru|it|scn] [--no-net] [--no-files] [--restrict-files]
vladx init [папка] [--lang ru|it|scn]                        новый проект (vladx.toml, точка входа, модуль)
vladx parse <файл> [--lang ru|it|scn] [--keywords <путь>]    AST в JSON
vladx check <файл> [--lang ru|it|scn]                        только диагностика
vladx keywords [--lang ru|it|scn]                            словарь форм
vladx fmt <файл> [--to ru|it|scn] [--write]                  канонический вид / перевод на другой язык
vladx serve [--port 8000]                                    площадка, прокси и ИИ для .vlhmn
vladx pm <команда>                                           менеджер пакетов (= vladpm)
vladx hmn-eval <папка>                                       оценка перевода .vlhmn на наборе фраз
```

### Площадка на shared-хостинге — как Node.js

Положите `vladx-shared.zip` из релиза на хостинг с PHP — и площадка работает максимально похоже на Node: вывод по ходу, `спроси` ждёт ответа в терминале, файлы пишутся на сервер сразу, `rete` читает любые сайты без CORS, время идёт. Там же лежит посредник ИИ для `.vlhmn`: ключ Polza.ai и модель хранятся только на сервере. Установка — `deploy/shared/README.md`. Локально то же делает `vladx serve` (файлы — прямо на диск, в `vladx-files/`).

CLI ходит к ИИ через тот же сервер:

```bash
export VLADX_LLM_PROXY=https://ваш-сайт/vladx/vladx-llm.php
```

```bash
vladx run examples/ru/14-shopping.vlhmn
```

Перед запуском `vladx` показывает, как ИИ понял текст, и не запускает программу, если уверенность ниже 0.8. Переводы кэшируются: повторный запуск запросов не тратит.

## Проверки

```bash
cargo test
```

```bash
bash scripts/test-all.sh
```

Скрипт прогоняет `cargo test`, проверку словаря, все примеры, схему AST, негативные случаи, флаги файлов и сети, полный цикл vladpm с реестром под `php -S` и печатает «ВСЁ ЗЕЛЁНОЕ». Браузерный сценарий в Chromium (OPFS, перезагрузка, `спроси`, сеть напрямую и через прокси):

```bash
node tests/browser/files-net.mjs
```

## Устройство

```
crates/
  vladx-ast       нейтральный AST (serde) и диагностика
  vladx-syntax    словарь → лексер → парсер → статические проверки
  vladx-runtime   интерпретатор, трейт Host, ядро stdlib (Rust) и stdlib на VladX (stdlib/*.vlstd, 12 модулей)
  vladx-hmn       .vlhmn: промпт, проверка ответа ИИ, повтор, кэш
  vladx-pm        vladpm: манифест, semver, зависимости, vladx.lock, установка
  vladx-wasm      C-ABI для браузера (без wasm-bindgen)
  vladx-cli       vladx run/parse/check/keywords/serve, vladpm
  xtask           проверка словаря: cargo run -p xtask
spec/
  keywords.toml   единственный источник ключевых слов
  SPEC.md         спецификация языка
  ast.schema.json контракт AST (JSON Schema 2020-12)
site/             площадка: один HTML со встроенным WASM
examples/         примеры на ru, it, scn (проверяются тестами)
tests/            негативные случаи и браузерный сценарий
deploy/shared/    PHP для shared-хостинга: файлы, сеть и ИИ площадки, реестр пакетов vladpm и его сайт
docs/HANDOVER.md  исходное техническое задание
```

Слова языка живут только в `spec/keywords.toml`; в парсере нет ни одной словоформы (это проверяет тест). Добавить синоним — добавить строку в словарь.

## Чего пока нет

Импорт модулей `.vlhmn`. Слова `it` и `scn` (в том числе слова `vladx.toml`) ждут проверки носителем. Подробнее — `spec/SPEC.md`, §13.

Лицензия — MIT.
