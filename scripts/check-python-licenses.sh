#!/usr/bin/env bash
# MIT check for one Python project. Installs that project's required dependencies
# into a temporary venv. The root hooks package is installed against the local SDK.
set -euo pipefail

target="${1:?project path: . | sdks/python | sdks/aischemagate/python}"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

ignore="$(grep -v '^[[:space:]]*#' scripts/license/python-ignore.txt | grep -v '^[[:space:]]*$' | tr '\n' ' ')"
venv="$(mktemp -d)"
trap 'rm -rf "$venv"' EXIT

python3 -m venv "$venv"
"$venv/bin/pip" install -q -U pip

case "$target" in
  .|root)
    "$venv/bin/pip" install -q ./sdks/python
    "$venv/bin/pip" install -q .
    ;;
  sdks/aischemagate/python)
    "$venv/bin/pip" install -q ./sdks/python
    "$venv/bin/pip" install -q ./sdks/aischemagate/python
    ;;
  sdks/python)
    "$venv/bin/pip" install -q ./sdks/python
    ;;
  *)
    echo "unknown Python license target: $target" >&2
    exit 1
    ;;
esac

"$venv/bin/pip" install -q 'pip-licenses==5.5.5'
# shellcheck disable=SC2086
"$venv/bin/pip-licenses" --allow-only 'MIT;MIT License' --ignore-packages $ignore
