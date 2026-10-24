# Repository instructions for coding agents

## Mandatory first read

**Before analyzing source code, proposing repository-specific fixes, or editing any file, read the full document `docs/DOCK_SYSTEM_IMPLEMENTATION_PLAN.md` from beginning to end.** Do this at the start of every new task/session involving this repository. It is the canonical project brief, target architecture, ordered implementation roadmap, and acceptance checklist.

Do not start by scanning implementation files. Read the plan first, then identify the relevant stage and acceptance criteria, and only then inspect the minimum source files needed for that stage.

## Working rules

1. Follow the plan's stages in order. Do not skip stages, combine them, or start later-stage features before the current stage meets its acceptance criteria unless the user explicitly changes the plan.
2. Treat the plan's architecture and boundaries as requirements, not suggestions. Keep `dock-core` and `ui` unchanged by default. Before changing either one, explain why the change is needed, identify the exact file and minimal diff, and wait for the user's explicit approval. Approval for one specific change does not authorize other changes to those crates.
3. Before editing, verify the real current API and code. The plan marks some API details as needing confirmation; do not guess signatures or assume a planned API exists.
4. Make focused, reviewable changes. Preserve working behavior and add/adjust regression tests for the stage being implemented.
5. Finish a stage only after applicable build, test, and lint checks where the environment permits, except for checks explicitly excluded by the known-environment rule below. Never say checks passed unless they were actually run.
6. At the start of a task, state internally or in the work summary: the stage being addressed, its acceptance criteria, and the files expected to change. If the task does not fit the roadmap, explain the mismatch before changing architecture.
7. Do not use one-off floating-specific patches to compensate for a broken shared interaction model. Compare floating and docked behavior against the plan's common geometry, event/session, and rendering contracts.
8. Work one individual checklist line/subtask at a time, not one whole roadmap stage at a time. Complete that unit, then stop and report before starting the next line. Each report must state the exact checklist item, what changed, files touched, and the commit SHA/link. Mention checks only when they were actually run and useful; do not repeat the known-unavailable-checks boilerplate described below. Keep each line's changes in its own focused commit when practical; do not bundle several checklist lines into one large change.
9. Before continuing past a completed checklist line, provide the report in the conversation. Do not wait until the entire stage is finished to report progress.

## Canonical document

- `docs/DOCK_SYSTEM_IMPLEMENTATION_PLAN.md` — read this first; it is the source of truth for the planned architecture and implementation order.


## Known environment limitation — user-directed

The user has explicitly requested that agents not spend time on the following routine checks in the current environment because they are ineffective or unavailable:

- Do not attempt local `cargo build`, `cargo test`, or `cargo clippy` runs by default.
- Do not check GitHub Actions/commit status, probe DNS/network connectivity, or retry access diagnostics for each patch.
- Do not perform repetitive post-edit bookkeeping checks solely to produce a verification paragraph, such as refetching the latest `main` SHA or separately checking symbol/test/import presence after every small change.

This exception overrides rules 5 and 8 for the checks listed above. Do not claim they passed, and do not repeat explanations about DNS, missing CI runs, or unavailable tests in every report. Focus on the requested plan line, read the relevant source before editing, keep the patch narrow, and report the actual changes and commit link. Resume any excluded checks only if the user explicitly asks or states that the environment has changed.
