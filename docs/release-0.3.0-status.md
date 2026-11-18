# Release 0.3.0 — status of the work order (branch `vibe/release-0.3.0`)

One commit per point. No tag, no release, no merge — the PR is opened
separately. Every UI change was rendered headless and the PNG inspected
before committing (`feature_screenshots` + `CATWRIGHT_SHOT_DIR`, plus
`tools/screenshots/render.sh` for the README shots).

## Done (A, required — in order)

1. **File chips** in agent answers: path-like words / inline code that name
   an existing file of the session workspace render as chips (file icon,
   tooltip = absolute path, click opens the preview pane). Existence only,
   cached 3 s; contents never read; paths outside the workspace and
   function names stay plain text. Commit `8e0a30c`.
2. **Drag & drop** in the composer: `hovered_files`/`dropped_files`,
   "Drop to attach" overlay while dragging, dropped files go through the
   same attachment list as "+" (PNG → image, else resource_link, several
   per drop). Commit `9195daa`.
3. **README screenshots** `preview-diff.png` and `activity.png` via
   `tools/screenshots/render.sh` (stand-in agent plays a live prompt with a
   permission request; no test hooks in `src/`). New documented start
   options `--open <file>` (+`--preview-diff`) and `--activity`. Existing
   screenshots re-rendered, alt texts in the README. Commit `c407988`.
4. **README feature list** extended (diff comments, preview pane, detail
   levels, activity feed, file chips, drag&drop); "Known limits / tested
   with Vibe 3.1.0–3.1.2" untouched on purpose. Commit `9fb9661`.
5. **Version 0.3.0** in `app/Cargo.toml` + the lock entry in the same
   commit; `cargo build --locked` green. Commit `c76e4f3`.
6. **CHANGELOG**: `## Unreleased` → `## v0.3.0` with the v0.2.0-style
   download block; all features since v0.2.0 (the four from PR #2 plus the
   new ones), fixes carried over. Commit `9270991`.
7. **docs/test-plan-0.3.0.md**: manual checklist against real vibe-acp
   3.1.3 (start/login, sessions, prompt + approval, all four detail levels
   during a running round, diff comment incl. line-number check after folded
   hunks, preview code/diff, file chips, drag&drop PNG + text, activity feed
   incl. answering from the feed, FAILED, archive, routines incl. cron in
   the interval field, fork, rewind, German). Commit `356a0e9`.

## Done (B)

8. **Worktrees**: sidebar lists each project's worktrees (porcelain, cached,
   main worktree never listed); delete with confirmation, dirty warning,
   branch option default OFF; create dialog with optional start ref.
   Commit `9378189`.
9. **Palette (Ctrl K)** lists the current project's files (git ls-files,
   cached, fuzzy); Enter opens the preview. Commit `47192f5`.
10. **Per-round detail**: expandable at the folded round header; tool calls
    with status and client-side duration (arrival timestamps on updates).
    Commit `fe2cbfe`.
11. **Desktop notifications**, opt-in (Settings → General, default OFF),
    only when the window is unfocused, via `notify-send` when installed,
    project name only in the text. Commit `e37636f`.
12. **Routines "Delete all"** via `_loops/clear` with confirmation;
    `docs/acp.md` documents `_loops/clear` and cron in `_loops/create`.
    Commit `43dc5be`.

## Not done / notes

- The README activity screenshot shows a waiting (answerable) and a
  finished entry; a *failed* entry needs a failed prompt and a demo run
  sends exactly one prompt — the FAILED state is covered by the test plan
  and the `06-sidebar-failed` test shot.
- "Known limits / tested with Vibe 3.1.0–3.1.2" stays until the manual test
  with 3.1.3 passes (per the order).

## Open questions

- Does vibe-acp 3.1.3 accept a cron expression in `_loops/create`'s
  `interval` (test plan covers it; `docs/acp.md` now documents it)?
- Should notifications also fire for questions (`elicitation/create`), not
  only approvals?
- Worktree delete on a dirty worktree currently relies on git refusing
  `worktree remove` (error in the status line) plus the dialog warning;
  a `--force` option was deliberately not added.

Checks: `cargo fmt --check` clean, `cargo clippy --all-targets -- -D warnings`
clean, `cargo test` 444 unit + 4 integration green, German catalog complete.
