#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  tracked=$(git ls-files)
  if printf '%s\n' "$tracked" | grep -E '(^|/)\.env$|\.db$|inscrieri\.json|(^|/)data/'; then
    echo "file di dati o segreti nel git"
    exit 1
  fi
fi
if grep -RIn --exclude-dir=target --exclude-dir=.git -E 'sk_live_[A-Za-z0-9]' .; then
  echo "possibile segreto nel sorgente"
  exit 1
fi
exit 0
