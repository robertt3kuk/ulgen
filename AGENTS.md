# Ulgen Agent Rules

This file defines how agents and contributors must execute issues in this repository.

## 1) Issue Intake Is Mandatory (Before Coding)

For every issue, do this first:

1. Understand expected behavior (`what should work`).
2. Define implementation approach (`how we will build it`).
3. Confirm acceptance criteria (`done when`).
4. Capture unknowns and assumptions.

If the issue is ambiguous, add a clarification comment before implementation.
No coding starts until the issue has a clear implementation target.

## 2) One Issue, One Branch

- Branch naming format: `codex/ulg-<id>-<short-slug>`
- Example: `codex/ulg-17-workspace-sidebar`
- Keep branch scope limited to one issue.

## 3) Required Issue Linking

- Every commit message must include the issue id (for example `ULG-17`).
- Every PR must link:
  - Linear issue URL
  - GitHub issue number (if available)
- PR title format:
  - `[ULG-17][M3] short summary`

## 4) PR Timing Rules

1. Open a **Draft PR** after the first meaningful working slice.
2. Keep the PR updated with progress and assumptions.
3. Mark PR **Ready for review** only when:
   - Acceptance criteria are satisfied
   - Relevant tests pass
   - Docs are updated when contracts/behavior change

## 5) Milestone Handling

- Normal flow: one PR per issue.
- Milestone flow: after all issue PRs in a milestone are merged, open one milestone rollup PR:
  - Branch: `codex/m<id>-rollup`
  - Content: milestone notes, risk summary, follow-ups, release checklist updates.

## 6) Linear State Flow

Use this state flow for each issue:

1. `Backlog` -> when unstarted
2. `In Progress` -> when branch is created and work starts
3. `In Review` -> when Draft/Ready PR exists
4. `Done` -> after merge and post-merge checks

## 7) Definition of Done

An issue is done only when all are true:

- Code merged to `master`
- Linked PR exists
- Acceptance criteria verified
- Linear issue moved to `Done`
- Follow-up tasks captured (if needed)
