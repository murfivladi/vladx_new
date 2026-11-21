#!/usr/bin/env bash
# Все проверки: cargo test + словарь + все примеры + негативные случаи + CLI-флаги.
# Сеть проверяется, если доступна (VLADX_NET=0 — пропустить).
set -uo pipefail
cd "$(dirname "$0")/.."
FAIL=0
ok()   { printf '  \033[32mok\033[0m   %s\n' "$1"; }
bad()  { printf '  \033[31mFAIL\033[0m %s\n' "$1"; FAIL=1; }
has()  { case "$1" in *"$2"*) return 0 ;; *) return 1 ;; esac; }

echo "== cargo test"
if cargo test -q 2>&1 | tee /tmp/vladx-test.log | grep -q 'FAILED\|error\['; then bad "cargo test"; cat /tmp/vladx-test.log; else
  ok "cargo test: $(grep -o '[0-9]* passed' /tmp/vladx-test.log | awk '{s+=$1} END {print s}') тестов"; fi

echo "== словарь"
if cargo run -q -p xtask >/dev/null; then ok "keywords.toml"; else bad "keywords.toml"; fi

cargo build -q -p vladx-cli
VX="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import sys,json;print(json.load(sys.stdin)["target_directory"])')/debug/vladx"

echo "== примеры"
TMP="$(mktemp -d)"
for lang in ru it scn; do
  for f in examples/$lang/*.vlstd; do
    name="$(basename "$f")"
    if [ "$name" = "12-net.vlstd" ]; then
      "$VX" check "$f" --lang "$lang" >/dev/null 2>&1 && ok "$f (только check: сеть ниже)" || bad "$f"
      continue
    fi
    # примеры с файлами пишут во временную папку, а не в репозиторий
    cp "$f" "$TMP/$name"
    if echo "Аня" | "$VX" run "$TMP/$name" --lang "$lang" >/dev/null 2>"$TMP/err"; then ok "$f"; else bad "$f"; cat "$TMP/err"; fi
  done
done
[ -f "$TMP/заметка.txt" ] && [ -f "$TMP/папка/вложенный.txt" ] && ok "11-files создал настоящие файлы" || bad "11-files не создал файлы"

echo "== схема AST"
if python3 -c 'import jsonschema' 2>/dev/null; then
  if msg="$(python3 scripts/validate-ast.py "$VX")"; then ok "$msg"; else bad "$msg"; fi
else echo "       (пропущено: нет python-пакета jsonschema)"; fi

echo "== форматтер и перевод"
fmtok=1
for lang in ru it scn; do for f in examples/$lang/*.vlstd; do
  [ "$("$VX" fmt "$f" --lang "$lang")" = "$(cat "$f")" ] || { bad "$f не в каноническом виде (vladx fmt)"; fmtok=0; }
done; done
[ $fmtok = 1 ] && ok "все примеры в каноническом виде"
it_out="$("$VX" fmt examples/ru/04-loops.vlstd --to it 2>&1)"
has "$it_out" "per ogni вещь in покупки" && ok "перевод ru → it" || bad "перевод ru → it: $it_out"

echo "== .vlhmn"
printf '```\nскажи "только код"\n```\n' > "$TMP/code.vlhmn"
[ "$("$VX" run "$TMP/code.vlhmn" 2>&1)" = "только код" ] && ok ".vlhmn из одного блока кода работает без ИИ" || bad ".vlhmn только с кодом"
if [ -n "${POLZA_API_KEY:-}${VLADX_LLM_KEY:-}" ]; then
  for f in examples/*/*.vlhmn; do
    lang="$(basename "$(dirname "$f")")"
    "$VX" check "$f" --lang "$lang" >/dev/null 2>"$TMP/err" && ok "$f (ИИ)" || { bad "$f"; cat "$TMP/err"; }
  done
else
  echo "       (перевод .vlhmn через ИИ пропущен: нет POLZA_API_KEY; конвейер проверен тестами vladx-hmn)"
fi

if command -v php >/dev/null; then
  for f in deploy/shared/*.php; do php -l "$f" >/dev/null || bad "$f: синтаксис PHP"; done
  ok "deploy/shared/*.php: синтаксис PHP ($(ls deploy/shared/*.php | wc -l) файлов)"
fi

echo "== проекты и модули"
for l in ru it scn; do
  rm -rf "$TMP/proj-$l"
  "$VX" init "$TMP/proj-$l" --lang "$l" >/dev/null
  out="$(cd "$TMP/proj-$l" && echo "Аня" | "$VX" run 2>&1)"
  has "$out" "Аня!" && ok "vladx init --lang $l → vladx run без файла, импорт модуля" || bad "проект $l: $out"
done
mkdir -p "$TMP/proj-ru/lib"
printf 'из "../утилиты" возьми приветствие\nфункция сломай() {\n  верни 1 / 0\n}\n' > "$TMP/proj-ru/lib/м.vlstd"
printf 'из "lib/м" возьми сломай\nсломай()\n' > "$TMP/proj-ru/т.vlstd"
out="$("$VX" run "$TMP/proj-ru/т.vlstd" 2>&1)"
has "$out" "lib/м.vlstd:3:11  E-MATH-001" && ok "ошибка в модуле — с файлом и строкой модуля" || bad "ошибка модуля: $out"

echo "== негативные случаи"
for pair in "01-order:E-ORDER-001" "02-unclosed-block:E-SYNTAX-003" "03-mutate-const:E-NAME-002" "04-unknown-word:E-SYNTAX-016"; do
  f="tests/negative/${pair%%:*}.vlstd"; want="${pair##*:}"
  got="$("$VX" check "$f" 2>&1 | grep -o 'E-[A-Z]*-[0-9]*' | head -1)"
  [ "$got" = "$want" ] && ok "$f → $want" || bad "$f: ждали $want, получили ${got:-ничего}"
done

echo "== файлы: пути и флаги"
mkdir -p "$TMP/p/sub"
printf 'file.запиши("../вверх.txt", "через ..")\nскажи file.прочитай("%s/вверх.txt")\n' "$TMP/p" > "$TMP/p/sub/a.vlstd"
[ "$("$VX" run "$TMP/p/sub/a.vlstd" 2>&1)" = "через .." ] && ok "абсолютный путь и .." || bad "абсолютный путь и .."
has "$("$VX" run "$TMP/p/sub/a.vlstd" --restrict-files 2>&1)" 'restrict-files' && ok "--restrict-files отказывает с объяснением" || bad "--restrict-files"
has "$("$VX" run "$TMP/p/sub/a.vlstd" --no-files 2>&1)" 'E-FILE-001' && ok "--no-files → E-FILE-001" || bad "--no-files"

echo "== сеть"
has "$("$VX" run examples/ru/12-net.vlstd --no-net 2>&1)" 'E-NET-001' && ok "--no-net → E-NET-001" || bad "--no-net"
if [ "${VLADX_NET:-1}" != "0" ]; then
  printf 'пусть с будет rete.получи("https://example.com")\nскажи testo.содержит(с, "Example Domain")\n' > "$TMP/net.vlstd"
  [ "$("$VX" run "$TMP/net.vlstd" 2>&1)" = "истина" ] && ok "настоящий HTTPS без флагов (example.com)" || bad "HTTPS к example.com"
  if out="$("$VX" run examples/ru/12-net.vlstd 2>&1)"; then ok "12-net: $(echo "$out" | head -1)"; else
    echo "       (12-net: $(echo "$out" | head -1) — api.github.com может быть закрыт сетью окружения)"; fi
fi
rm -rf "$TMP"

echo
if [ $FAIL = 0 ]; then echo "ВСЁ ЗЕЛЁНОЕ"; else echo "ЕСТЬ ОШИБКИ"; exit 1; fi
