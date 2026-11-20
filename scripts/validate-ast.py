#!/usr/bin/env python3
"""AST каждого примера (vladx parse) проверяется по spec/ast.schema.json."""
import json, pathlib, subprocess, sys
from jsonschema import Draft202012Validator

vx = sys.argv[1]
schema = json.loads(pathlib.Path("spec/ast.schema.json").read_text(encoding="utf-8"))
v = Draft202012Validator(schema)
bad = 0
files = sorted(pathlib.Path("examples").glob("*/*.vlstd"))
for f in files:
    out = subprocess.run([vx, "parse", str(f), "--lang", f.parent.name], capture_output=True, text=True)
    errs = list(v.iter_errors(json.loads(out.stdout)))
    if errs:
        bad += 1
        print(f"{f}: {errs[0].message} @ {list(errs[0].absolute_path)}")
print(f"схема AST: {len(files) - bad}/{len(files)} примеров соответствуют")
sys.exit(1 if bad else 0)
