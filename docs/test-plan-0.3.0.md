# Manual test plan for v0.3.0

Checklist for the manual test on the maintainer's PC with a real
`vibe-acp` **3.1.3** (v0.3.0 was developed against 3.1.2; the "tested with"
notes in README/CHANGELOG stay at 3.1.0–3.1.2 until this passes).

Each point: what to do, what should happen. Tick off or note the failure.

## Start and login

- [ ] Start `catwright` with `vibe` signed in (run `vibe` once in a terminal
      first). **Should:** the app connects, shows "Mistral Vibe 3.1.3" in the
      status bar, lists projects/sessions. If not signed in: a clear "please
      sign in" status, no crash.
- [ ] First start asks for the language (English/Deutsch).

## Sessions

- [ ] Open a project, start a new session, send a prompt that makes Vibe
      call tools (e.g. "read README.md and summarise it").
      **Should:** user message, tool calls with status, agent answer render;
      an approval card appears if the mode asks for one.
- [ ] Load an existing session (click it in the sidebar).
      **Should:** history replays, title/mode/model shown, no duplicates.
- [ ] Rename, fork, delete a session from the sidebar menu.
      **Should:** rename keeps the session open; fork opens a new session;
      delete asks for confirmation and removes it.

## Approvals

- [ ] Answer a permission card (Allow once / Deny / …).
      **Should:** the turn continues or stops accordingly; the card disappears.
- [ ] Answer a permission from the **activity feed** (sidebar → Activity,
      entry shows Allow once/Deny). **Should:** same as answering the card.

## Detail levels during a running round

- [ ] Start a longer prompt (several tool calls), then switch
      Settings → transcript detail between Compact / Standard / Detailed /
      Verbose **while the round runs**. **Should:** Compact shows the running
      round as one "Working · N tool calls · step" line plus agent text and
      changed files; Standard/Detailed/Verbose open more tool groups/outputs;
      the setting persists across restarts.

## Diff comments

- [ ] In a tool call with a diff, hover a changed line and click "+", write
      a comment, add a second comment on another line, then "Send to Vibe".
      **Should:** comments hang under their line (yellow edge), the editor has
      a frame and one primary button; "Send to Vibe" puts one composer draft
      naming **the correct line numbers** (check after a folded/collapsed
      hunk — the numbers must still be right).

## Preview pane

- [ ] Click a diff chip / changed file / file chip. **Should:** preview pane
      opens on the right, relative path, line numbers, Code/Diff toggle.
      Code mode shows the file; Diff mode shows the red/green diff against
      the last commit (no raw git text). `--open <file> [--preview-diff]`
      opens it on start.
- [ ] File chips: an answer mentioning `somefile.py` shows a chip with a file
      icon; click opens the preview, hover shows the full path. A mention of a
      non-existing file stays plain text. **Should:** as described.

## Attachments / drag & drop

- [ ] Click "+" and attach a PNG and a text file; also **drag & drop** a PNG
      and a text file onto the composer (hint "Drop to attach" while
      dragging). **Should:** chips appear under the input; on send the PNG goes
      as image (when the agent accepts images), the text file as resource
      link; remove via the chip's ×.

## Activity feed and failed turns

- [ ] Let a turn finish and one turn fail (e.g. stop the agent or provoke an
      error). Open sidebar → Activity. **Should:** "Waiting for you",
      "Finished" and "Failed" sections with entries (workspace in mono
      caps, age, Open on the right); failed session shows red FAILED in the
      sidebar until it is opened or a new prompt is sent.
- [ ] "Mark all read" clears the badge; a session can be marked unread and
      archived (archived group in the sidebar, un-archive from there).
      **Should:** as described; archiving persists across restarts.

## Routines

- [ ] Create a routine (sidebar → routines, e.g. daily summary) with a
      plain interval (`1d`, `30m`). **Should:** accepted and listed with its
      next run. A cron expression (`0 9 * * 1-5`) is not accepted by
      vibe-acp 3.1.3 (`_loops/create` takes `<number><unit>` only).
      **Should:** Vibe's error shows in the status line, no crash. Delete a
      routine, then "Delete all". **Should:** gone from the list.

## Fork and rewind

- [ ] Fork an active session at a message. **Should:** new session with the
      history up to that message.
- [ ] Rewind to an earlier message. **Should:** confirmation, then the
      transcript is cut there and Vibe's state is rewound.

## Language

- [ ] Settings → General → Deutsch. **Should:** the whole UI switches to
      German (restart not needed); every new 0.3.0 screen (activity feed,
      preview pane, comments, chips, drop hint) is translated too. Switch back
      to English. **Should:** complete.

## Notes

- Anything that looks wrong: note the exact steps. UI changes were verified
  headless (`--screenshot --headless`), but a real agent run can still
  surprise — that is what this pass is for.
