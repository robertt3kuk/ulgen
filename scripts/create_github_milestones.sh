#!/usr/bin/env bash
set -euo pipefail

: "${GITHUB_TOKEN:?Set GITHUB_TOKEN to a token with repo scope.}"

OWNER="${OWNER:-robertt3kuk}"
REPO="${REPO:-ulgen}"
API="https://api.github.com/repos/${OWNER}/${REPO}"

read -r -d '' MILESTONES <<'EOF' || true
M0 - OSS & Tracking Bootstrap|2026-03-26|Repository foundation, OSS docs, tracking setup in Linear/GitHub.
M1 - Native Platform Core|2026-04-16|GPUI app shell bootstrap, PTY/ConPTY abstraction, notification service, CI matrix.
M2 - Hybrid ctmux/cmux Engine|2026-05-07|mux daemon, socket control API, tmux-like detach/attach behaviors, keyboard baseline.
M3 - Essential Block UX + Sidebar Navigation|2026-05-28|Block engine, workspace sidebar hierarchy, command palette, block-linked notifications.
M4 - ACP Host + Terminal App Control|2026-06-25|ACP lifecycle support, ACP terminal bridge, interactive app mode, permission gates.
M5 - Themes, Pointer, Polish, Beta|2026-07-23|Theme system, pointer/input modes, launch layouts, performance hardening.
EOF

echo "Fetching existing milestones for ${OWNER}/${REPO}..."
existing="$(curl -fsSL \
  -H "Accept: application/vnd.github+json" \
  -H "Authorization: Bearer ${GITHUB_TOKEN}" \
  -H "X-GitHub-Api-Version: 2022-11-28" \
  "${API}/milestones?state=all&per_page=100")"

while IFS='|' read -r title due description; do
  [[ -z "${title}" ]] && continue

  if grep -Fq "\"title\": \"${title}\"" <<<"${existing}"; then
    echo "Skipping existing milestone: ${title}"
    continue
  fi

  payload=$(cat <<JSON
{"title":"${title}","description":"${description}","due_on":"${due}T23:59:59Z"}
JSON
)

  echo "Creating milestone: ${title}"
  curl -fsSL -X POST \
    -H "Accept: application/vnd.github+json" \
    -H "Authorization: Bearer ${GITHUB_TOKEN}" \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    "${API}/milestones" \
    -d "${payload}" >/dev/null
done <<<"${MILESTONES}"

echo "Done."
