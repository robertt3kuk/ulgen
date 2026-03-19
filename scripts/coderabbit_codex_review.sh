#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

if command -v coderabbit >/dev/null 2>&1; then
  CR_BIN="coderabbit"
elif [[ -x "${HOME}/.local/bin/coderabbit" ]]; then
  CR_BIN="${HOME}/.local/bin/coderabbit"
else
  echo "CodeRabbit CLI not found. Install: curl -fsSL https://cli.coderabbit.ai/install.sh | sh"
  exit 1
fi

BASE_BRANCH="master"
REVIEW_TYPE="all"
EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base)
      [[ $# -ge 2 ]] || { echo "--base requires a value"; exit 1; }
      BASE_BRANCH="$2"
      shift 2
      ;;
    --type)
      [[ $# -ge 2 ]] || { echo "--type requires a value"; exit 1; }
      REVIEW_TYPE="$2"
      shift 2
      ;;
    *)
      EXTRA_ARGS+=("$1")
      shift
      ;;
  esac
done

if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
  exec "${CR_BIN}" review \
    --plain \
    --prompt-only \
    --base "${BASE_BRANCH}" \
    --type "${REVIEW_TYPE}" \
    -c AGENTS.md \
    "${EXTRA_ARGS[@]}"
else
  exec "${CR_BIN}" review \
    --plain \
    --prompt-only \
    --base "${BASE_BRANCH}" \
    --type "${REVIEW_TYPE}" \
    -c AGENTS.md
fi
