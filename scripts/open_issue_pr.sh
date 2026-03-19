#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 3 ]]; then
  echo "Usage: $0 ULG-<id> M<id> \"PR summary title\" [github_issue]"
  exit 1
fi

issue="$1"
milestone="$2"
title="$3"
github_issue="${4:-N/A}"

pr_title="[${issue}][${milestone}] ${title}"

tmp_body="$(mktemp)"
cat > "${tmp_body}" <<EOF
## Summary

Workflow and repository process update for ${issue}.

## Linked Issues

- Linear: https://linear.app/ulgen-term/issue/${issue}
- GitHub: ${github_issue}
- Milestone: ${milestone}

## Issue Clarification (Before Coding)

- Expected behavior (\`what should work\`): clear issue intake process and branch/PR linking discipline.
- Implementation approach (\`how\`): add repo rules, templates, and scripts.
- Acceptance criteria (\`done when\`): documented workflow is actionable and enforced by templates/scripts.
- Assumptions/unknowns: GitHub issue linking may be N/A for some tasks.

## Validation

- Tests run: \`cargo test --workspace\`
- Manual checks: scripts syntax checks and branch/PR conventions verified.
EOF

gh pr create --draft --title "${pr_title}" --body-file "${tmp_body}"
rm -f "${tmp_body}"
