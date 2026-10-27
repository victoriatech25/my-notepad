#!/usr/bin/env bash
# 배포 스크립트
set -euo pipefail

TARGET="${1:-staging}"
if [ $# -gt 1 ]; then
  echo "인자가 너무 많습니다: $#"
  exit 1
fi

for f in dist/*.js; do
  echo "업로드: $f -> $TARGET"
done
