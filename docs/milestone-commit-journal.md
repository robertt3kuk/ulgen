# Milestone Commit Journal

This file tracks commit-level history by milestone so we can audit what changed, why it changed, and how it was implemented.

## Update Rules

1. Add entries in chronological order.
2. Group entries by milestone (`M0`, `M1`, ...).
3. For each commit, capture: `what`, `why`, `how`, and `validation`.
4. Include reverts/rollbacks explicitly so history stays explainable.
5. In `validation`, prefer concrete evidence (`cargo test ...`, GitHub Actions run URL, PR link).

## Entry Format

| Commit | Issue | What | Why | How | Validation |
|---|---|---|---|---|---|

## M0 - OSS & Tracking Bootstrap

| Commit | Issue | What | Why | How | Validation |
|---|---|---|---|---|---|
| `bb92c5a` | `N/A` | Initial repository bootstrap. | Establish base workspace and project structure. | Added initial crates/apps/docs scaffold and git history baseline. | Initial repository bootstrapped successfully. |
| `d2d7bb7` | `ULG-7` | Added issue intake and PR workflow rules. | Standardize contribution and execution flow. | Added/updated process docs and templates. | Workflow docs present and usable in repo. |
| `e8374e3` | `ULG-7` | Set default issue PR base to `master`. | Align with branch strategy used in this project. | Updated helper workflow behavior for PR creation defaults. | PR creation flow targets `master` correctly. |
| `553f194` | `ULG-6` | Expanded README as product landing page. | Improve onboarding and roadmap clarity for contributors. | Added architecture, roadmap, quickstart, and tracking context. | README supports project understanding and first run. |

## M1 - Native Platform Core

| Commit | Issue | What | Why | How | Validation |
|---|---|---|---|---|---|
| `9313aae` | `ULG-8` | Added app-shell bootstrap/restore and command routing core. | Deliver M1-1 initial app shell behavior. | Implemented startup state model, restore path, command handlers. | App shell startup and restore tests added. |
| `b74ec6a` | `ULG-8` | Moved default state path to user-scoped OS directories. | Prevent cwd-coupled state loss and repo pollution. | Implemented platform-specific default state directories. | State path behavior verified by tests and manual run. |
| `b66da24` | `ULG-8` | Hardened `--command` parsing. | Avoid panic/crash on missing or option-like values. | Added strict argument validation and controlled error path. | CLI parser tests pass. |
| `dbf07c9` | `ULG-8` | Hardened `--new-workspace` parsing and state-write flow. | Prevent silent bad state mutation and partial-write corruption. | Added argument validation and safer save/replacement behavior. | CLI and state persistence tests pass. |
| `469a262` | `ULG-8` | Temporarily narrowed CI matrix to macOS only. | Stabilize failing path faster during active debugging. | Reduced CI OS matrix scope in workflow. | macOS lanes used for stabilization runs. |
| `5f23f59` | `ULG-8` | Isolated test temp paths to fix flakiness. | Eliminate path collisions in parallel tests. | Added unique temp naming with pid/time/sequence. | CI and local tests stabilized. |
| `f12598a` | `ULG-9` | Introduced PTY backend factory and adapter contract tests. | Establish M1-2 abstraction foundation. | Added backend kind/factory, platform adapter surfaces, contract tests. | `ulgen-pty` and workspace tests pass. |
| `6a46cff` | `ULG-9` | Refined runtime backend semantics and path safety. | Address review risks around misleading defaults and cwd handling. | Split contract/runtime semantics and switched cwd to `PathBuf`. | Targeted + workspace tests pass. |
| `5d5dcae` | `ULG-9` | Added temporary CodeRabbit wrapper workflow. | Trial prompt-only local review workflow. | Added script and docs references. | Script/docs were added successfully. |
| `9a35215` | `ULG-9` | Removed temporary CodeRabbit workflow. | Team chose to defer CodeRabbit integration for now. | Reverted script/docs workflow changes. | Repo returned to non-CodeRabbit flow. |
| `069296d` | `ULG-9` | Default backend set to contract backend; Unix shell helper uses `sh -c`; portable cwd fixture updates. | Ensure default backend is usable now and avoid login-shell pitfalls. | Adjusted backend selection, shell invocation, and test portability. | `ulgen-pty` and workspace tests pass. |
| `957b3e0` | `ULG-9` | Restored real newline assertion in PTY write smoke test. | Ensure newline behavior is actually tested, not escaped literal. | Replaced `\\n` literal check with actual newline assertion. | `ulgen-pty` and workspace tests pass. |
| `6752c65` | `ULG-9` | Added backend semantics docs and ACP downstream unsupported-runtime guard test. | Improve clarity and ensure graceful unsupported propagation. | Added function-level docs and ACP test around runtime backend errors. | `ulgen-acp`, `ulgen-pty`, and workspace tests pass. |
