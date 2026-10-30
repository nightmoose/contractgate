#!/usr/bin/env bash
# License check for one Python project. Installs that project's required dependencies
# into a temporary venv. The root hooks package is installed against the local SDK.
# Allow list uses the pip-licenses classifier names, plus the SPDX strings the
# tool prints when a distribution sets License-Expression.
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
# Classifier tails from PyPI, and the SPDX forms pip-licenses emits instead
# when License-Expression is set (idna and httpcore report BSD-3-Clause).
allow='MIT;MIT License;BSD License;BSD-2-Clause;BSD-3-Clause;ISC License (ISCL);ISC;Zero-Clause BSD (0BSD);0BSD'
# shellcheck disable=SC2086
"$venv/bin/pip-licenses" --allow-only "$allow" --ignore-packages $ignore
