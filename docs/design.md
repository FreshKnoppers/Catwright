# Catwright design: playful, pixel-ish, warm

Design references were screenshots of other products (Mistral docs, the
Vibe web app, Claude Desktop's Code mode); they live only in the local,
git-ignored `reference/` folder. The look is warm orange/yellow with pixel
blocks and mono labels; layout and harness features follow a classic
agent desktop (sidebar, transcript, right panel).

## Colour tokens (sampled from the references)

| token | hex | use |
|---|---|---|
| `BG` | `#1a1a1e` | main area |
| `SIDEBAR` | `#17171b` | sidebar / side panels |
| `CARD` | `#101010` | cards, code blocks, tool groups |
| `RAISED` | `#29292d` | selected row, chips, user bubble |
| `INPUT` | `#2a2a2e` | composer |
| `BORDER` | `#404040` | 1 px card / panel borders |
| `BORDER_SOFT` | `#2e2e34` | quiet lines between panels, around cards |
| `HOVER` | `#232328` | row under the pointer |
| `TEXT` | `#ececec` | body text |
| `MUTED` | `#b4b4ba` | secondary text that must stay comfortably readable |
| `DIM` | `#9a9aa0` | tertiary text, mono labels (≥ 4.5:1 up to `RAISED`) |
| `ORANGE` | `#fa500f` | primary accent: CTA buttons, active marks, logo |
| `ON_ACCENT` | `#101010` | text and icons on `ORANGE`/`AMBER`/`YELLOW` fills (white on `ORANGE` is only 3.4:1) |
| `SELECTION` | `ORANGE` at 35 % | selected text, selectable labels |
| `ORANGE_DIM` | `#e06a16` | pixel meters, numbers (context "1M", prices), 5.2:1 on `BG` |
| `YELLOW` | `#ffd800` | highlight tag (like "TRY IN PLAYGROUND"), focus |
| `AMBER` | `#ffaf00` | gradient step |
| `RED` | `#e10500` | error edges, deny outlines |
| `RED_TEXT` | `#ff6e62` | red text on dark backgrounds (contrast) |
| `GREEN` | `#3fb950` | success, added lines, allow |
| `DIFF_ADD_BG` / `DIFF_DEL_BG` | `#12281a` / `#301514` | diff line backgrounds |
| `BLUE` / `PURPLE` | `#5b9cf8` / `#b07cf8` | cool accents (session modes) |

Session modes run from calm to risky (`theme::mode_color`): Plan `BLUE`,
Accept Edits `GREEN`, Smart Approve `AMBER`, Auto Approve `RED_TEXT`, custom
Vibe profiles `PURPLE`.

Pixel gradient (for pixel art / meters, left to right):
`#e10500 #fa500f #ff8205 #ffaf00 #ffd800`.

## Shape and type

- Small radii: 2–4 px (cards 4, buttons 3, chips 2). No big pill shapes
  except the composer (6 px).
- Section labels, breadcrumbs, meter labels, model ids, token counts:
  monospace, UPPERCASE, small (11–12 px), `DIM`, with a short leading
  square bullet `■` in `ORANGE` for the active item (like the docs top nav).
- Body text stays proportional (egui default + fallback fonts).
- Thin 1 px `BORDER` lines, dashed separators are fine (draw short segments).

## Pixel elements (drawn with egui painter rects, no image assets)

- **Pixel meter**: row of square blocks (e.g. 10 × 10 px, 3 px gap). Filled
  = `ORANGE_DIM`, empty = `RAISED`. Use for the context window (Kontext
  panel: 20 blocks = 100 %), thinking level (low/medium/high = 1/2/3 of 3
  blocks) and speed-like indicators. A meter never shows more than is
  there: the context split goes into colours only while every part fills a
  whole block, otherwise the meter shows the total and the legend the split.
- **Pixel logo** (`theme::LOGO`): our OWN 12×12 pixel mark, an `ORANGE`
  cat sitting in profile on a `RED` branch, its tail hanging down over the
  branch; the eye is an empty cell (the tile shows through). Drawn at 2 pt
  cells in the sidebar. The app icons show it on a dark `SIDEBAR` tile with
  a `BORDER_SOFT` edge (rendered by `tools/icons/render.py`, which reads the
  bitmap from theme.rs; a test checks the bundled icon against it). Do NOT
  copy Mistral's "M" logo or their cat mascot; we only borrow the style.
- **Project icons**: 3×3 pixel squares whose colours are derived from a hash
  of the project path (palette = the pixel gradient + a few cool colours),
  like the coloured project dots in picture-7.
- **Busy indicator**: three pixel blocks pulsing through the gradient
  instead of a round spinner.
- **Status squares**: tool-call status as a small square: pending `DIM`
  outline, in_progress `YELLOW`, completed `GREEN`, failed `RED`.

## Type scale

The UI uses an installed grotesque (Inter, Adwaita Sans, Noto Sans, Liberation
Sans, in that order, `fonts::UI_FONT_CANDIDATES`), never bundled. egui's default
light font is too thin at small sizes. `Small` is 12 pt (egui's default 9 pt is
unreadable), body 14, conversation 14.5, mono labels 10.5–12.5. Nothing below
10.5 pt.

## Components (`widgets.rs`, `icons.rs`)

- **Icons**: 7×7 bitmaps drawn with painter rects (`icons::Icon`), 2 pt cells in
  rows, 1 pt for chevrons. No emoji, they do not exist on every system. An icon
  always comes with a label or a tooltip.
- **Rows** (`nav_row`, `paint_row_bg`): 27–30 pt high, full width, HOVER on
  hover, selected = RAISED with a 2 pt ORANGE edge. Titles end in an ellipsis,
  never at a fixed character count.
- **Actions appear on hover** in rows and at messages and cost no layout space
  (`widgets::place`; `Ui::put` would move everything after the row).
  A menu that is open keeps its row in the hover state.
- **Accent budget**: ORANGE marks the one thing to do or the place you are,
  not every state. One filled Primary per screen while a decision waits:
  while an approval or a question card is open, other Primary buttons (e.g.
  "Send to Vibe") drop to Secondary.
- **Buttons**: `Primary` (ORANGE with `ON_ACCENT` text) once per card,
  `Secondary` (RAISED), `Danger` (RED outline, `RED_TEXT`).
- **Segmented switch** (`widgets::segmented`, e.g. Code | Diff): a state, not
  an action, so no Primary fill: soft frame, the chosen segment RAISED with
  TEXT and a 2 pt ORANGE underline, the others DIM.
- **Attention cards**: a card with a 2 pt coloured top edge asks for a decision.
- **Toasts**: the status line as a toast below the header. Info fades after six
  seconds, errors stay until dismissed.

## Layout

- **Left sidebar** (resizable, Strg B folds it to an icon rail, automatic below
  880 pt): brand, Neue Sitzung, Suchen (Strg K), Artefakte, the workspace tree
  (the chat workspace "Allgemein" pinned on top, then the projects) with status
  and age per session (running = moving blocks, waiting = yellow, unseen result =
  orange; a workspace row with a waiting session carries a yellow mark, also
  while folded), account footer with connection state and budget meter. There is
  no Code/Chat switch: chats are sessions of "Allgemein".
- **Session header**: workspace crumb ("Allgemein" or the project), session title as menu (rename, fork, copy
  transcript as Markdown, delete), git branch and dirty count ("4 uncommitted":
  git's view of the working directory, deliberately not called "changes", which
  is the right panel's word for Vibe's edits in this session), fork, folder,
  right-panel toggle.
- **Conversation** (column max. 820 pt): user bubble right, agent text left in
  a quiet flow, tool groups as one line, edited files always visible as cards
  with a real line diff.
- **Pinned above the composer** (they never scroll away): trust banner, Vibe's
  questions, permission cards with command or diff preview (number keys answer
  the first card while the composer is empty — only its one-shot allow/deny
  options, numbered by their position among those; every lasting option
  including reject-always stays behind a click, and a fresh card ignores
  digits for its first painted frames), message queue.
- **Composer**: grows up to 220 pt, mode and "+" left, model and thinking right;
  `/` and `@` open keyboard-driven lists. Typing anywhere goes into it. Each
  session keeps its own draft. Because it is nearly always focused, focus is
  a quiet `BORDER` step, not an accent ring; the ORANGE belongs to the send
  button once there is text.
- **Tables**: columns of numbers align right (header included), so digits of
  the same place line up; everything else aligns left.
- **Diffs**: the transcript wraps long lines in its column. The preview pane
  shows line numbers (the old side's on removed lines), never wraps and
  scrolls sideways, so code keeps its shape. Added and removed lines form
  continuous bands.
- **Inline code** in answers: monospace in a calm warm tint (`#e3d4c2`) on a
  quiet box (`#26262b`). The box marks where the code starts and ends, so the
  colour can stay calm and never outshouts the sentence; file chips keep the
  warmer `#ffd9a0` and a border because they are clickable.
- **Right panel** (resizable, Strg J, hidden below 1120 pt): flat collapsible
  sections on the panel background, separated by hairlines (no boxes inside
  the box): Kontext, Plan, Session changes (Vibe's edits in this session;
  accept / revert on hover), Subagents, Routinen, Konnektoren, Umgebung. A
  folded section still shows its summary ("none", "2 connected",
  "master · Trusted"). "Compact context" appears from 50 % on. While the
  panel shows the plan, the conversation shows one plan line.
- **Start screen**: cat, "Was baust du heute in [project ▾]" (in "Allgemein":
  "Worüber möchtest du chatten? [Allgemein ▾]"; picker and heading share one
  size and middle), composer (its placeholder names `@` and `/`, no hint
  row). Below it what the sidebar does not tell: **Needs you** (waiting
  sessions with the command of their approval), **New results** of this
  run (first line of the answer), **Continue** (the latest other session),
  **In this project** (branch, uncommitted files, trust). Only while nothing
  waits and nothing is new, the recent sessions follow. The picker chooses
  the workspace of the new session.
- **Charts** (generative UI): round axis ticks (1, 2 or 5 × 10ⁿ), point marks
  only while points stand at least 8 pt apart, one title style for cards and
  charts (TEXT, bold); computed numbers use `ORANGE_DIM`. Bar charts keep
  their labels in one column so all bars start at the same x, bars in
  `ORANGE_DIM`, values without float noise. A pie sits on the middle of a
  taller legend. The agent picks no colours: series and slices take
  `theme::CHART_SERIES` by position. Sliders have a narrow square handle
  with the stretch left of it filled; a ui button keeps its own width,
  also inside columns.
- **Command palette** (Strg K): sessions, projects, actions, slash commands.
- **Transcript detail** (Settings → General): the four levels as cards in a
  row (two rows when narrow), each with a check square, its name, what it
  shows while Vibe works, and a little picture of how much is open (grey a
  folded line, orange a tool group, green a result, amber a thought). The
  chosen card has an ORANGE edge.

Playful is welcome (pixel mascot, gradient blocks), but readability first:
contrast of body text against `BG` stays high, and the pixel accents never carry
information on their own (always with a label).

## Backlog

- Done: the animated pixel cat (own few-frame bitmap) runs in the subagent
  status bar, the busy row of a turn and on the start screen. In the
  subagent bar a running cat chases a rolling ball of yarn (`theme::YARN`,
  YELLOW with ORANGE_DIM strands, a YELLOW thread on the ground); each
  subagent's cat keeps its own phase (`theme::cat_stagger`). Finished
  states show only the sitting cat.
