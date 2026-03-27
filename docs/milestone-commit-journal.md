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

## M2 - Hybrid ctmux/cmux Engine

| Commit | Issue | What | Why | How | Validation |
|---|---|---|---|---|---|
| `23765fc` | `ULG-12` | Added persistent mux daemon journal and restore flow. | Establish restart-safe multiplexer state as M2 foundation. | Implemented journal persistence, daemon restore path, and deterministic state transitions. | `cargo test --workspace` |
| `630166c` | `ULG-12` | Hardened journal recovery and restore durability paths. | Prevent corrupted state from breaking startup and recovery. | Added backup restore fallback, corruption quarantine behavior, and restore-policy hardening tests. | `cargo test --workspace` |
| `cea7115` | `ULG-12` | Updated agent workflow guardrails for dual-analysis merge gate. | Enforce stricter quality controls before milestone merges. | Extended `AGENTS.md` rules to require two xhigh subagent reviews and merge ordering constraints. | Process rule audited in merged PR workflow. |
| `f8fdf25` | `ULG-13` | Added versioned mux socket control API and Unix transport hardening. | Deliver external control surface for mux state and layouts. | Implemented NDJSON v0 API, request limits, socket path safety checks, and integration tests. | `cargo test --workspace`, PR #11 checks green |
| `PENDING` | `ULG-14` | Implemented keyboard navigation baseline with dual keymap profiles and remap conflict handling. | Enable keyboard-only workspace/tab/pane operations for M2-4 acceptance. | Added command ID catalog, Warp/Tmux defaults, override resolver + deterministic conflict policy, app-shell key-chord routing/cache, backward-compatible settings aliases, and coverage tests. | `cargo clippy -p ulgen-command -p ulgen-settings -p ulgen-app --all-targets -- -D warnings`; `cargo test --workspace` |
| `PENDING` | `ULG-15` | Implemented tmux-like mux semantics for detach/attach continuity and scoped synchronized input fanout. | Deliver M2-3 lifecycle and scope behavior needed for persistent session workflows. | Added idempotent detach/attach lifecycle handling, scope-based target resolution with detached-session filtering, direction-aware pane split insertion semantics, stale detached-session cleanup on restore, expanded RPC tests, and updated contract docs. | `cargo clippy -p ulgen-muxd --all-targets -- -D warnings`; `cargo test -p ulgen-muxd`; `cargo test --workspace` |

## M3 - Essential Block UX + Sidebar Navigation

| Commit | Issue | What | Why | How | Validation |
|---|---|---|---|---|---|
| `cf75ef0` | `ULG-16` | Added persisted app-shell block engine lifecycle with indexing, replay, rerun/edit, and CLI execution entrypoint (`--run-command`). | Deliver M3-1 acceptance so command executions are first-class blocks with revisit/replay semantics. | Extended app state with versioned block history (`v2` + `v1` migration), added duplicate-id restore guards and lifecycle APIs (`start/append/finish/rerun/edit`), wired command-run flow in `main`, added regression tests for lifecycle, persistence, invalid transitions, duplicate ids, and version handling, plus architecture notes. | `cargo test --workspace`; dual xhigh subagent analysis loop (implementation-risk + architecture/tradeoff) with blocking feedback addressed. |
| `PENDING` | `ULG-17` | Added sidebar navigation primitives and command/keymap wiring for `workspace -> tabs -> panes` traversal with persistent left/right position. | Deliver M3-2 acceptance for essential sidebar navigation and keyboard-first workflow parity. | Implemented sidebar domain model/tree APIs in app shell, command routing + registry entries, Warp/Tmux default sidebar chords, selection cache invalidation on non-sidebar navigation, persistence/docs updates, and hardening tests for traversal, restore, fuzzy jump, missing-node error path, and keybindings. | `cargo test --workspace`; dual-loop xhigh subagent review process (Loop A risk pass + Loop B architecture/tradeoff pass). |
| `PENDING` | `ULG-18` | Added command palette and quick-switch core with unified command/entity search, deterministic ranking, execution routing, and persisted recent history. | Deliver M3-3 acceptance so core actions and context switching are discoverable and executable from one keyboard-first palette surface. | Introduced palette item model (`cmd:` and `node:` ids), candidate indexing from command registry + sidebar graph, match scoring (exact/prefix/substring) with recency boost, `palette_execute` routing through existing handlers, and tests for command/entity discoverability, quick switch, recency ordering, persistence, and unknown-id errors; updated architecture docs. | `cargo test --workspace`; dual-loop xhigh subagent review process (Loop A risk pass + Loop B architecture/tradeoff pass). |
| `PENDING` | `ULG-19` | Integrated block lifecycle with notification events and deep-link resolution for block context targeting. | Deliver M3-4 acceptance so block completion/failure/approval flows notify reliably and navigate back to exact block context. | Wired `finish_block` status transitions to `NotificationBus` (`TaskDone`/`TaskFailed`), added explicit approval-required publish API, implemented block/session-to-window/workspace/tab/pane navigation target resolver, exposed notification history/resolve helpers, and added tests for event emission, approval flow, and deep-link resolution; updated architecture docs. | `cargo test --workspace`; dual-loop xhigh subagent review process (Loop A risk pass + Loop B architecture/tradeoff pass). |

## M4 - ACP Host + Terminal App Control

| Commit | Issue | What | Why | How | Validation |
|---|---|---|---|---|---|
| `c3d6167` | `ULG-20` | Implemented ACP lifecycle server + JSON-RPC stdio transport for initialize/new/load/prompt/cancel/update flows. | Deliver M4-1 acceptance for stable ACP session lifecycle and update-stream handling. | Added initialized ACP server state machine, monotonic session/prompt ids, per-session update queues, structured server errors, JSON-RPC request/response contracts + method dispatch, parameter/error mapping, and expanded integration tests for protocol lifecycle and transport failures. | `cargo test -p ulgen-acp`; `cargo test --workspace`; dual-loop xhigh subagent review process (Loop A risk pass + Loop B architecture/tradeoff pass). |
