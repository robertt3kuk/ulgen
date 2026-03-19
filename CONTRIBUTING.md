# Contributing to Ulgen

Thanks for your interest in Ulgen.

## Workflow

1. Start from a Linear issue and clarify `what`, `how`, and `done when`.
2. Create one branch per issue from `master` (`codex/ulg-<id>-<slug>`).
3. Keep changes scoped to that single issue.
4. Add or update tests for behavior changes.
5. Open a Draft PR early, then move to Ready when acceptance criteria are met.
6. Link Linear issue and GitHub issue in the PR.

## Development Standards

- Keep platform-specific logic behind clear interfaces.
- Preserve settings compatibility and migration paths.
- Avoid breaking RPC contracts without versioning notes.
- Document public API changes in `docs/`.

## Pull Request Checklist

- [ ] Issue behavior and implementation approach were clarified before coding
- [ ] Linked to Linear issue and GitHub issue
- [ ] Added tests for changed behavior
- [ ] Updated docs if contracts changed
- [ ] Passed `cargo test`
- [ ] Ran CodeRabbit prompt review (`./scripts/coderabbit_codex_review.sh --base master --type all`) and addressed findings

See [AGENTS.md](AGENTS.md) for the full execution rules.

## AI Review Loop (Codex + CodeRabbit)

Use prompt-only mode for Codex-compatible review context:

- Branch/PR diff review: `./scripts/coderabbit_codex_review.sh --base master --type all`
- Pre-commit local review: `./scripts/coderabbit_codex_review.sh --type uncommitted`

If CodeRabbit responds with a rate-limit message, wait for the reported cooldown and rerun.

## Commit Style

Use focused commits with clear messages:
- `feat(muxd): add workspace selection RPC (ULG-13)`
- `fix(pty): handle resize race on unix (ULG-9)`
- `docs(readme): update quickstart (ULG-6)`
