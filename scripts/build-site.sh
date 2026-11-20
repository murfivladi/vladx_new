#!/usr/bin/env bash
# Сборка площадки: WASM → base64 → site/playground.html (один файл).
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build -q -p vladx-wasm --target wasm32-unknown-unknown --release
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import sys,json;print(json.load(sys.stdin)["target_directory"])')"
WASM="$TARGET_DIR/wasm32-unknown-unknown/release/vladx_wasm.wasm"
python3 - "$WASM" <<'PY'
import base64, json, pathlib, sys
wasm = pathlib.Path(sys.argv[1]).read_bytes()
examples = {}
for lang in ("ru", "it", "scn"):
    items = []
    for f in sorted(pathlib.Path("examples", lang).glob("*.vlstd")):
        items.append({"name": f.stem, "code": f.read_text(encoding="utf-8")})
    examples[lang] = items
tpl = pathlib.Path("site/playground.template.html").read_text(encoding="utf-8")
ex_json = json.dumps(examples, ensure_ascii=False).replace("</", "<\\/")
html = tpl.replace("__WASM_B64__", base64.b64encode(wasm).decode()).replace("__EXAMPLES_JSON__", ex_json)
pathlib.Path("site/playground.html").write_text(html, encoding="utf-8")
print(f"site/playground.html: WASM {len(wasm)//1024} КБ, страница {len(html.encode())//1024} КБ, "
      f"примеры: " + ", ".join(f"{k} {len(v)}" for k, v in examples.items()))
PY
