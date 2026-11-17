# Recherche: Orca (orca.dev) und DeepSeek Harness vs. Catwright

Stand: 2026-11-17, Branch `vibe/research-orca-dsh-a372c4`. Nur Analyse, kein
Code. Quellen: Landingpages und Docs von [orca.dev](https://orca.dev) /
[orca.dev/docs](https://orca.dev/docs) (Worktrees, Automations, Orchestration,
Browser/Design Mode, Mobile, SSH, Settings) und
[deepseek.com/en/harness](https://www.deepseek.com/en/harness/) plus
[github.com/deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness)
(docs/architecture.md, docs/user/guide/schedule.md, packages/schedule),
gespiegelt gegen `docs/acp.md`, `app/MECHANICS.md`, `docs/HANDOFF.md` und den
Code in `app/src/`.

## Kurzfazit

Beide Produkte sind Harness-Oberflächen für CLI-Agenten; Catwright ist dagegen
ein reiner ACP-Client für `vibe-acp` (spawn lokal, JSON-RPC over stdio,
`app/src/acp/client.rs`). Viele Orca-/DSH-Features existieren in Catwright
bereits in anderer Form, weil Vibe sie selbst über ACP-Extensions liefert
(`_loops/*`, `_account/read`, `_review/*`, `_rewind/*`, `_connectors/*` —
siehe `docs/acp.md` und `app/src/acp/client.rs:224-238`). Was technisch nicht
geht, ist durch die Architektur und die Hard Rules in `AGENTS.md` bestimmt:
keine neuen Crates, keine WebView, kein Netzwerk-Crate, keine Telemetrie, keine
Netzwerk-Calls ohne User-Trigger.

## Aktueller Umfang von Catwright (Referenz)

- Sidebar: Projekte + Sessions (Status, Pin, Umbenennen/Löschen), Chat-
  Workspace „Allgemein", Suche (Strg K), Artefakte, Konto-Footer,
  Verbindungsstatus (`harness/ui/sidebar.rs`).
- Header: Session-Menü (Umbenennen, Fork, Transkript kopieren, Löschen),
  Git-Branch + geänderte Dateien (`harness/ui/header.rs`, `harness/watch.rs`).
- Transkript: Thoughts, Tool-Gruppen, Line-Diffs, Plan, Rewind pro Runde,
  faltbare Runden (`harness/ui/transcript.rs`).
- Karten: Trust, Fragen (`elicitation/form`), Permissions, Queue
  (`harness/ui/cards.rs`).
- Rechtes Panel: Kontext (inkl. Compact), Plan, Changes mit Review
  (Accept/Revert), Subagents, **Routines (vollständige UI: anlegen/löschen,
  nächster Lauf — `harness/ui/panel.rs:595-690`)**, Connectors, Environment.
- Composer: Attachments (PNG als `image`, sonst `resource_link`), Slash- und
  `@`-Listen mit Tastatursteuerung, Mode/Model/Thinking (`harness/ui/composer.rs`).
- Palette (Strg K): Sessions, Projekte, App-Aktionen, Slash-Commands
  (`harness/ui/palette.rs`).
- Artefakte: alle berührten Dateien pro Projekt, Vorschau je Typ (MD gerendert,
  PNG Bild, CSV Tabelle, HTML als Text + „im Browser öffnen“, nie WebView).
- Generative UI: ` ```ui `-Blöcke als native Widgets, ` ```ui-js ` in
  boa-Sandbox mit Budgets (Skill `catwright-ui`).
- Worktree: `create_worktree` (Hintergrund-`git worktree add`, Branch
  `catwright/<name>`, eigener Workspace; `harness/mod.rs:1098`).
- Settings: 5 Tabs (Allgemein, Vibe, Konto, Kuerzel, Datenschutz), Autosave,
  `auto_account`, gepinnte Sessions (`app/settings.rs`, `store.rs:193`).
- Persistenz: nur `~/.config/catwright`, atomar 0600, keine Secrets, kein Log.
- CLI: `--project`, `--session`, `--prompt`, `CATWRIGHT_PROMPT`, `--settings`,
  `--screenshot`, `--lang` (`main.rs`).

## Orca-Features — Bewertung

| Feature | Umsetzbar? | Anmerkung |
|---|---|---|
| Worktree pro Task, parallele Agenten | Fundament da | Ausbau: Worktrees listen/löschen (lokal via git; Branch-Löschung nur mit User-Freigabe) |
| Agent-Dashboard / Unread über alle Projekte | Ja | UI über bestehende Sidebar/Palette |
| Jump anywhere (Cmd-J) | Teilweise da | Palette existiert; Datei-/Artefakt-Sprünge fehlen |
| Automations (cron, durable, host-weit) | Nur Vibe-seitig | `_loops/*` bereits mit UI; Loops laufen nur bei geöffneter idle Session, nicht durable. Eigener Scheduler = User-Entscheidung 2026-11-08 (bisher nein) |
| Usage & Accounts | Größtenteils da | `_account/read` (`usagePercentage`, `resetAt`), Sidebar-Footer + Tab Konto + `auto_account`; Account-Wechsel geht nicht (kein Zugriff auf Login) |
| Notifications & Agents feed (Desktop) | Ja, mit Freigabe | `notify-send` ohne neues Crate (steht in HANDOFF §4) |
| Mobile Companion / Push / Remote Server | Nein | Pairing/Push = Netzwerk ohne User-Trigger; widerspricht Privacy-Regeln |
| Embedded Browser, Design Mode, Computer Use, Emulator | Nein | Keine WebView, keine neuen Crates |
| Dictation / Voice | Nein | Audio-Crate nötig; `_voice/*` ungenutzt |
| ACP-Terminal / Split-Panes | Nein / Designfrage | `terminal:false` wird advertised |
| Drag files in Prompt, @-Dateibaum | Teilweise | `@`-Liste existiert; Dateibaum-Popup + Datei-Drag&Drop (egui `dropped_files`) fehlen; Zwischenablage-Bilder gehen nicht |
| Rich Previews (PDF/HTML/DOCX in-app) | Nur MD/PNG/CSV | Rest ohne WebView/Crates nicht sinnvoll |
| Orchestration (Coordinator/Worker/DAG) | Eher nein | Vibe koordiniert selbst via Subagents; Steering in laufende Turns geht über ACP nicht |
| SSH-Hosts, Cloud-VMs | Nein | SSH-Crate + Netzwerk |
| Plugin-Marktplatz | Nein | Bereits entschieden; Gegenstück ist der `catwright-ui`-Skill |

## DeepSeek-Harness-Features — Bewertung

| Feature | Umsetzbar? | Anmerkung |
|---|---|---|
| „Everything is a plugin" (Cordis) | Nein als Architektur | TS/Node-Runtime; konzeptionelles Äquivalent: Vibe-Skills + `_connectors/*` (verdrahtet, aber nicht in Settings exponiert — kleine UI-Arbeit) |
| Scheduled tasks (cron + IANA-TZ, durable, Delivery-Records) | Siehe Loops | DSH-Vorteil ist host-weit durable; über ACP nur session-lokale `_loops/*`. Cron/TZ-Eingabe in der Routines-UI wäre prüfenswert (`cron?` im Result laut `docs/acp.md`) |
| Execution Traces / Developer Tools | Größtenteils ja | Turn-Detail mit Dauern/Tool-Übersicht aus dem Transkript; Zeitstempel pro Tool-Call ggf. clientseitig ergänzen |
| Deliverables / Artefakt-Previews | Teilweise | Artefakte existieren; DOCX/XLSX/PDF brauchen Crates → nein |
| Agent teams | Größtenteils da | Subagents mit Statusbalken existieren; Team-Gruppierung wäre Transcript-UI |
| Voice input, Creator mode | Nein | Audio-Crate bzw. Sicherheitsmodell; `catwright-ui` deckt UI-Erweiterung ab |
| Auto approval review | Da | Modi `smart-approve`/`auto-approve` + `_review/*` |

## Priorisierte Empfehlungen (nicht beauftragt)

1. Routines-Card: Cron/TZ-Eingabe und Anzeige nächster Läufe erweitern
   (Methoden existieren in `acp/client.rs`); Caveat „nur bei geöffneter idle
   Session" ist bereits in der UI vermerkt.
2. Worktree-Verwaltung in der Sidebar: bestehende Worktrees via
   `git worktree list` erkennen, löschen (mit Bestätigung; Branch-Löschung
   = User-Freigabe), Start-from-Ref beim Anlegen.
3. Palette um Dateien/Artefakte erweitern; im Composer Dateibaum-Popup zum
   `@`-Menü und Drag&Drop von Dateien (`watch::files`, `mention_at`,
   egui `dropped_files` existieren).
4. Desktop-Benachrichtigung bei Turn-Ende/Freigabe via `notify-send`
   (braucht User-Freigabe, kein neues Crate) + Unread-Markierungen über
   Projekte.
5. Connectors in den Settings exponieren (`_connectors/list|toggle` ist
   verdrahtet, `harness/extras.rs:314`).
6. Turn-Trace-Detailansicht (DSH-Developer-Tools) aus dem Transkript bauen.

## Ausgeschlossen (Grund)

In-App-Browser/Design Mode/Computer Use/Emulator (keine WebView, keine Crates),
Voice/Dictation (Audio-Crate), Mobile Companion/Push/Remote Server/SSH
(Netzwerk ohne User-Trigger), Plugin-Marktplatz/Cordis, ACP-Terminal,
Account-Wechsel, Bilder aus der Zwischenablage, eigener persistenter Scheduler
(User-Entscheidung 2026-11-08 steht dagegen und würde Vibe-Loops doppeln).
