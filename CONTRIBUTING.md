# Contributing to Ulgen

Thanks for your interest in Ulgen.

## Workflow

1. Create a branch from `master`.
2. Keep changes scoped to one issue or sub-issue.
3. Add or update tests for behavior changes.
4. Open a pull request with milestone and issue references.

## Development Standards

- Keep platform-specific logic behind clear interfaces.
- Preserve settings compatibility and migration paths.
- Avoid breaking RPC contracts without versioning notes.
- Document public API changes in `docs/`.

## Pull Request Checklist

- [ ] Linked to Linear issue and GitHub issue
- [ ] Added tests for changed behavior
- [ ] Updated docs if contracts changed
- [ ] Passed `cargo test`

## Commit Style

Use focused commits with clear messages:
- `feat(muxd): add workspace selection RPC`
- `fix(pty): handle resize race on unix`
- `docs(readme): update quickstart`
