#!/usr/bin/env bash
# Build dist/function.zip for the validate Lambda (pure-Python contractgate).
# boto3 is provided by the Lambda runtime and is not bundled.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="$ROOT/dist/build"
ZIP="$ROOT/dist/function.zip"
rm -rf "$BUILD" "$ZIP"
mkdir -p "$BUILD"
cp "$ROOT/lambda/handler.py" "$ROOT/lambda/gates.py" "$BUILD/"
cp -a "$ROOT/lambda/contracts" "$BUILD/contracts"
python3 -m pip install --upgrade -q "contractgate>=0.4.1" -t "$BUILD"
# Drop pip metadata and caches to keep the zip small.
find "$BUILD" -type d \( -name "*.dist-info" -o -name "__pycache__" -o -name "tests" \) -prune -exec rm -rf {} +
find "$BUILD" -type f -name "*.pyc" -delete
python3 - "$BUILD" "$ZIP" << 'ENDPY'
import sys, zipfile
from pathlib import Path
build, dest = Path(sys.argv[1]), Path(sys.argv[2])
with zipfile.ZipFile(dest, "w", compression=zipfile.ZIP_DEFLATED) as z:
    for path in sorted(build.rglob("*")):
        if path.is_file():
            z.write(path, path.relative_to(build).as_posix())
    names = z.namelist()
need = ["handler.py", "gates.py", "contracts/stripe_charges_raw.yaml"]
missing = [n for n in need if n not in names]
if missing:
    raise SystemExit(f"zip missing {missing}")
print(f"wrote {dest} ({dest.stat().st_size} bytes, {len(names)} files)")
ENDPY
