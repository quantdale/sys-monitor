# Deprecated sys-monitor — Agent Instructions

This repository is a **deprecated legacy snapshot**. The maintained successor for current system-monitor development is `quantdale/monitorers`.

## Prime directive

Do not treat this repository as the active product. Unless the task explicitly targets this legacy codebase, prefer inspection/documentation only and direct active development to the maintained successor. Never copy current successor changes back here merely to make the repositories look synchronized.

## Required reading

1. `README.md` for the deprecation boundary and historical run/build commands.
2. `.agent/STATE.md` for current maintenance status.
3. `.agent/PLANNER_HANDOFF.md` and `.agent/EXECUTION_PROMPT.md` if present.
4. Current source/configuration only when a requested legacy task actually requires it.

Harness adapters under `.claude/`, `.opencode/`, `.kimi-code/`, and `.agents/` are subordinate to this file.

## Maintenance rules

- Preserve historical code and records unless a concrete maintenance task requires a change.
- Do not add new product features here by default.
- Do not claim the legacy project builds or runs unless the relevant command was executed at the current checkout.
- If a defect affects both this snapshot and the maintained successor, treat them as separate repositories with separate evidence; do not assume parity.
- Keep the deprecation notice visible in living documentation.

Evidence precedence is `current executable evidence` > `current repository contents` > `active task state` > `living docs` > `historical records` > `assumptions`.

## Documentation policy

Living docs may be corrected for factual accuracy, deprecation status, setup, and maintenance guidance. Historical commit messages, evidence, and old design context remain historical; do not rewrite them to imply later validation.

## Git safety

Inspect status and diff before committing. Do not discard unrelated work, rewrite history, force-push, or use destructive cleanup. Keep any legacy maintenance change narrowly scoped and explain why it belongs here rather than in `quantdale/monitorers`.
