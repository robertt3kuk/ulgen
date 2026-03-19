#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "Usage: $0 ULG-<id> short-slug"
  exit 1
fi

issue_upper="$1"
slug="$2"
issue_lower="$(echo "${issue_upper}" | tr '[:upper:]' '[:lower:]')"
branch="codex/${issue_lower}-${slug}"

git switch -c "${branch}"

echo "Created branch: ${branch}"
echo "Next steps:"
echo "1) Clarify issue what/how/done-when in Linear comment"
echo "2) Implement scoped changes for ${issue_upper}"
echo "3) Open draft PR with linked issue"
