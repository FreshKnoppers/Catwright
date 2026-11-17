# Handoff (2026-11-07, Nachträge 2026-11-08 bis 2026-11-17)

## Übergabe (Stand 2026-11-17, hier anfangen)

Alles unterhalb dieses Abschnitts ist älter. Branch
`claude/dreamy-carson-o2i16m` auf `main` (`e2217f3`), zwei Commits:

**1. Fixes aus dem Review vom 2026-11-16** (nachgestellt im privaten
Vorgänger-Repo, hier geprüft und portiert, jeweils mit Test):
- Slider mit `min > max` stürzte ab (`f64::clamp`): `render::slider_bounds`.
  `columns` mit vielen Kindern bricht um (`columns_per_row`).
- Selbstverschachtelnde ui-js-Tools liefen bis zum Stack-Überlauf: ui-js
  unter einem `text`-Knoten läuft nie, `text` höchstens 4 Ebenen tief
  (`RenderCtx::depth`).
- PNG-Vorschau: das 12 000-px-Limit liegt über der Texturgrenze vieler
  GPUs (8 192, `egui_glow` panikt): `fit_texture_side` verkleinert.
- Gescheiterter Startmodus schickte den Prompt in `accept-edits`: zurück in
  den eigenen Composer (`abort_setup` über `park_or_restore`). Test
  `set_mode_failure_still_sends_prompt_and_warns` dafür zu
  `set_mode_failure_holds_the_prompt_back_and_warns` umgestellt (User-
  Freigabe); Modell-/Denkstufen-Fehler senden weiter (neuer Test).
- Unlesbare Prompt-Antwort legte die zugestellte Nachricht zurück
  (`client::is_unreadable_response`).
- Freigabekarten zeigen die echten Argumente (`permission_arguments`),
  ui-Buttons die gesendete `action` beim Hover, Wayland-`app_id` gesetzt.
- Nur die erste Instanz schreibt `settings.json` (`.instance.lock`,
  `store::claim_settings`).
- Worktree im Hintergrund, 10-min-Limit, Aufräumen bei Fehlern
  (`make_worktree`, `watch::run_git`).

**2. Name Catwright** (User-Entscheidung 2026-11-17): Crate und Binary
`catwright`, `dist/catwright.desktop`, Icons, `app_id`, `clientInfo.name`,
Release-Archiv `catwright-<tag>-x86_64-linux`, Worktree-Branches
`catwright/<name>`, Skill `catwright-ui`, Hinweis `[Catwright] …`,
Konfiguration `~/.config/catwright`. Laut User gibt es keine Bestandsnutzer:
es gibt bewusst keinen Umzug alter Einstellungen und keine Erkennung früherer
Namen. Ausnahme: die echten Protokoll-Mitschnitte in
`app/tests/fixtures/acp/` enthalten noch den früheren Client-Namen, sie
dürfen laut AGENTS.md nicht bearbeitet werden.

**Tests:** fmt, clippy (keine Warnungen) sauber; 406 Unit + 4 Integration
grün, 2 ignored. Drei Tests scheitern nur als root (Container):
`write_private_failure_keeps_old_file_and_cleans_up_temp`,
`save_settings_fails_into_a_read_only_directory`,
`failed_autosave_is_visible_in_the_settings_status_line` — schon auf `main`.

**Screenshots neu machen (nächster Schritt, auf dem PC des Users):**
`tools/screenshots/render.sh` baut eine Demo-Welt in einem Temp-Ordner
(zwei Git-Projekte, vorbereitete Sitzungen, eigene Config- und Vibe-Ordner),
legt einen Stand-in für `vibe-acp` (spielt die Sitzungen per ACP ab, kein
Konto, kein Netz) vorn in den `PATH` und rendert mit `--screenshot` die vier
Szenen `start`, `session`, `subagents`, `dashboard` nach `docs/screenshots/`.
Mit `xvfb-run` unsichtbar, sonst öffnen sich die Fenster kurz. In der
Cloud-Sitzung scheiterte es nur an der fehlenden `libxkbcommon-x11`
(Paketinstallation war nicht erlaubt). Danach die Bilder ansehen (Ausschnitte,
Scrollposition am Ende des Verlaufs, Tortendiagramm), ggf. Inhalte in
`build.py` anpassen, Bilder committen. Die Skripte sind ungetestet gegen die
echte App gelaufen, nur trocken (Argumente) geprüft.

**Offen:** Die
Screenshots in `docs/screenshots/` zeigen noch den früheren Namen. Das
veröffentlichte GitHub-Release v0.2.0 trägt im Archivnamen noch den
früheren Namen. Marken-/Domain-Prüfung für „Catwright“ steht aus. GUI nicht
gestartet, Wayland-Icon nicht am Desktop geprüft.

## Übergabe (VIB Session 2026-11-13, älter)

Alles unterhalb dieses Abschnitts ist älter und in Teilen überholt. Bei
Widersprüchen gilt dieser Abschnitt, danach der Code.

**Zustand:** `cargo fmt --check` sauber, `clippy --all-targets` ohne
Warnungen, `cargo test` grün — 389 Unit + 4 Integration, 2 ignored
(letzter verifizierter Stand nach allen Wellen dieser Sitzung).

**Logik-Review (Vorbereitung):** Lead plus 6 Subagents haben die Logik
zwischen Protocol und UI reviewt (Reports und Lead-Synthese im
Session-Scratchpad, read-only). Ein Claude-Falsifikations-Pass hat die
13 Funde H1–H3/M1–M10 geprüft: 7 hielten stand, 4 wurden herabgestuft
(H1/H2/H3 von HOCH auf MITTEL, umbenannt zu M11/M12/M13; M3 auf
NIEDRIG; die Mechaniken blieben dabei verifiziert), 0 wurden widerlegt.
Endstand: 0 kritisch, 0 hoch, 13 mittel, plus ~20 niedrige Funde.

**Welle 1 (committet):**
- `a2ac3e5` Draft-Parkung pro Session (`park_or_restore`, `DraftSpot`):
  fehlgeschlagene Prompts, abgebrochene Queues, Verbindungsabbruch und
  Rewind geben Text der besitzenden Sitzung zurück; ein fehlgeschlagener
  Prompt räumt wartende Freigaben und die Queue ab; `session/list`-
  Paginierung hängt am Request-Cursor; `remove_project` antwortet offene
  Freigaben und schließt die Sitzungen des Ordners; Ctrl-K öffnet die
  Palette nicht mehr über fremden Modals. Tests u. a.:
  `failed_prompt_of_a_background_session_parks_under_its_session`,
  `nothing_is_discarded_on_a_restore`,
  `connection_loss_parks_queued_messages_per_session`,
  `connection_loss_returns_a_held_first_prompt_to_the_start_screen`,
  `failed_prompt_expires_waiting_approvals`,
  `failed_prompt_returns_the_queue_without_resending`,
  `session_list_answers_bind_to_their_request`,
  `remove_project_cancels_and_closes_the_sessions_of_the_folder`.
- `0bc9596` ui-js-Draht: getrennte Budgets je Richtung (Request
  398 528 Bytes, Antwort 606 208 Bytes inklusive State-Echo), State-Cap
  64 KiB bei `set_value`, zu große Requests werden vor dem Spawn mit der
  echten Ursache abgewiesen (M1), ein fehlgeschlagene Auswertung erhält
  einen begrenzten Retry nach 2 s (M2). Tests u. a.:
  `should_retry_decides_the_bounded_wakeup`,
  `a_failed_evaluation_retries_once_then_stays_quiet`,
  `a_transient_failure_recovers_on_the_retry`,
  `oversized_state_is_rejected_with_the_real_cause`,
  `worker_main_marks_oversized_requests_as_too_large`,
  `oversized_requests_are_rejected_before_the_spawn`,
  `worker_response_beyond_the_request_budget_arrives`,
  `worker_answer_past_the_response_budget_reports_the_size`;
  Integration: `worker_answer_with_payloads_beyond_the_request_budget_arrives`.
- `f99259f` PNG-Vorschau: eigene `image`-Limits (12 000 px, 128 MiB),
  begrenzter Texture-Cache (32 Einträge, LRU), Lese-Cap gilt während des
  Lesens. Tests u. a.: `preview_cap_is_enforced_during_the_read`,
  `preview_caps_files_that_understate_their_size`,
  `png_dimensions_allowed_covers_screenshots_rejects_giants`,
  `png_texture_refuses_images_beyond_the_dimension_limit`,
  `png_cache_evicts_the_least_recently_inserted_entry`.

**Welle 2–3, committet (`1caafb5`, `7111c6a`):**
- `1caafb5` Harness-Restfunde (N1, N3, N4, N6, N8, N9, set_chat_dir,
  Literale): Fork-Antwort ohne Quell-View öffnet die Fork mit ehrlicher
  Statusmeldung; der CLI-Startprompt überschreibt keinen getippten
  Draft und stirbt mit der Verbindung; Stopp beantwortet auch Fragen
  ohne Live-View; eingehend tote Sessions nehmen sent_prompts,
  hint_pending und geparkte Drafts mit (dying-Parameter: ein Reload
  behält die Geparkten); run_command hält denselben Busy-Guard wie
  send(); die Probe schließt sich sofort, wenn der Account-Read nicht
  gesendet werden konnte; set_chat_dir meldet Fehler statt still
  umzubiegen; die letzten zwei deutschen Status-Literale laufen über
  tr()/trf(). Tests: a_closed_session_drops_its_parked_draft_and_hint,
  late_prompt_failure_of_a_dropped_session_parks_nothing,
  cancel_answers_questions_without_a_live_view,
  connection_loss_keeps_the_parked_drafts_for_the_reload,
  probe_with_failed_account_reads_closes_the_probe_immediately.
- M8/i18n: die letzten deutschen Literale in `script.rs` sind auf
  `tr()`/`trf()` mit englischen msgids umgestellt. Der Worker-Kindprozess
  kennt keine UI-Sprache — der Draht trägt englische msgids; der Parent
  übersetzt bekannte msgids dort, wo sie nach `self.last` wandern
  (`translate_worker_error_in` in `script.rs`; bekannte msgids übersetzt,
  Eigenanteil „Script error“ um den Kopf, Fremdtext bleibt unangetastet).
  Test: `worker_errors_become_display_text_in_the_parent`;
  `worker_reports_script_errors` (Integration) prüft jetzt die englische
  msgid auf dem Draht.
- Doku-Drift: SKILL.md (Limits um State 64 KiB und Antwort-Budget
  592 KiB/606 208 Bytes ergänzt; „GUI is German“ → Labels folgen der
  UI-Sprache des Nutzers, Default Englisch), MECHANICS.md
  (`--settings` mit allen fünf Tabs inkl. konto/kuerzel; ui-js-Budget-
  zahlen aktualisiert), acp.md (Zeile `_trust/status` entfernt — Trust
  kommt über `_meta.workspace_trust` bzw. `_trust/decision`), design.md
  (Ziffern bei Freigabekarten: nur Einmal-/Ablehnen-Optionen,
  Position nach berechtigten Optionen, 1-s-Sperre), AGENTS.md
  (artifacts.rs, syntax.rs, memo.rs ins Datei-Inventar).

**Welle 4–5, committet (`db8f8db`, `f608032`):**
- `f608032` Per-Frame-Arbeit und Identitätsschlüssel (M10, M9, N2, N16,
  N17, N11b): das Transkript klont keine Turn-Diffs/Review-States mehr
  je Frame (Split-Borrows statt .cloned()), die Artefakt-Vorschau cacht
  pro Dateiversion hinter Arc (8 Einträge), plain-ui-Blöcke parsen über
  ein Memo; Klappzustände hängen an Message-/Tool-Call-IDs (rewindfest),
  Scroll-IDs sind inhaltsunabhängig, verschachtelte ui-Blöcke zählen den
  Blockzähler weiter; Checklisten-Werte reisen mit dem Form-Submit
  ihres Blocks (die SKILL.md-Zusage gilt jetzt); xdg-open wartet auf
  sein Kind (keine Zombies mehr); der Write-only-State round_tops ist
  entfernt. Tests: nested_ui_blocks_keep_their_own_field_values,
  checklist_states_travel_with_the_block_s_submit,
  fold_states_follow_their_content_across_a_rewind,
  preview_is_cached_per_file_version.
- `db8f8db` Randserie (N5, N12–N14, N18, M3-Härtung, Fokus): git_output
  bleibt auch mit Enkel-Prozess im Budget (Antwort über Kanal mit
  Restzeit), der Fallback-Walk folgt keinen Verzeichnis-Symlinks und
  stoppt bei einem Ordner-Budget; Freigabe-Karten nummerieren nur
  Einmal-Optionen (reject-always nur per Klick) und zählen gemalte
  Frames statt Wanduhr für die Ziffern-Sperre; boolean-Config-Optionen
  tolerieren currentValue:null; abgebrochene Turns frieren den Status
  unfertiger Subagenten dauerhaft ein; Backups unverlesbarer Settings
  kollidieren nie (numerische Suffixe), write_atomic fsynct das
  Zielverzeichnis, Autosave-Fehler stehen in der Statuszeile, Picker
  nehmen beim Öffnen den Tastaturfokus. Tests:
  git_output_returns_bounded_when_a_grandchild_holds_stdout,
  walk_files_terminates_on_dir_symlinks_and_keeps_them_as_entries,
  cancelled_subagent_stays_cancelled_over_later_turns,
  backup_name_appends_suffixes_on_colliding_stamps,
  failed_autosave_is_visible_in_the_settings_status_line.

**Lead-Nacharbeit (eigene Commits, aktuelle Baumspitze):** M6 als
Start-Watchdog — `START_TIMEOUT` (30 s) in `poll` behandelt einen
lebenden, aber stummen `vibe-acp` über `connection_lost` (Kind wird
gekillt, der Reconnect-Plan läuft, der Status meldet „initialize did
not answer in time“); die reine Entscheidung `starting_expired` ist
eigens getestet (a_mute_agent_counts_as_lost_after_the_start_timeout).
Das durch die Frame-Zählung überflüssige `PendingPermission.shown_at`
ist samt Konstruktionsstellen entfernt; der open_groups-Kommentar
beschreibt die Identitätsschlüssel. design.md zieht die Ziffern-Passung
nach (gemalte Frames statt 1 s, reject-always nur per Klick); AGENTS.md
und die übrigen Dokumente sind aus Welle 3 aktuell.

**Endaudit (Claude, gleiche Session):** Ein Abschluss-Audit über alle
10 Fix-Commits bestätigte 3 sauber und fand in den übrigen 7 echte
Fehler (alle vom Lead am Code verifiziert): Paginations-Deduplizierung
bei parallel antwortenden `session/list`-Ketten desselben cwd (Seiten
mergen jetzt nach `sessionId`); die FIFO-feste Vorschau-Regression aus
`f99259f` (der Typ-Check des Pfads fehlte vor dem Open — UI-Thread
konnte an einer namenlosen Pipe hängen); `cancel()` hat die Fragen von
Subagent-Kindern ANDERER laufender Sitzungen mit abgewürgt (der Filter
kennt jetzt `foreign_children`); parametrisierte Worker-Fehler kamen
nur mit unlocalisiertem Kopf an (der Parent rekonstruiert jetzt
Template + Argumente, inkl. neuem sichtbarem msgid für
„invalid JSON: {detail}“ — schließt zugleich die Scanner-Lücke);
`cancel_unfinished_subagents` fror Subagenten EARLIERER Turns ein
(Gefrier läuft ab `turn_start`); die Ziffern-Schutzfrist der
Freigabekarten war mit 3 Frames (~50 ms bei Animations-Framerate) ohne
Wirkung (60 Frames ≈ 1 s); verschachtelte ```ui-Blöcke verschieben die
field-/script-Keys der nachfolgenden äußeren Blöcke nicht mehr
(eigener Nummernraum ab 1_000_000, äußere Zählung wird
wiederhergestellt). Gate danach: fmt/clippy/test grün.

**Noch offen:** `reset_items` (session.rs) ist toter Code (dokumentierter
Zweck wird nie gelebt); der i18n-Scanner sieht nur tr-/trf-Literale,
nicht zusammengesetzte Texte (die Worker-Fehler der Wire sind inzwischen
aber selbst msgids); `tool_row`/`output_box` samplen weiterhin ~400
Zeilen je Frame (begrenzt, M10-Klasse, bewusst zurückgestellt);
Request-Timeouts jenseits des Starts sind bewusst nicht eingebaut (ein
hängendes session/prompt bei lebendem Kind bleibt sichtbar-hängend).

## Übergabe (Stand 2026-11-10, hier anfangen)

Alles unterhalb dieses Abschnitts ist älter und in Teilen überholt. Bei
Widersprüchen gilt dieser Abschnitt, danach der Code.

**Zustand:** Der Code/Chat-Umbau (Branch `claude/great-meitner-lwkox2`,
Abschnitt „Erledigt: Code und Chat zusammengelegt“ unten) ist hier
mitgemergt; dieser Abschnitt gilt für den kombinierten Stand.
`cargo fmt --check`, `clippy --all-targets` (keine Warnungen) und
`cargo test` sind grün: 320 Unit + 3 Integration, 2 ignored. Die Bugs 1 bis
7 aus dem Code-Review vom 2026-11-09 sind behoben (die ursprüngliche
Beschreibung steht im Abschnitt darunter).

1. **ui-js über 64 KiB** (`script.rs` `run_worker`): stdout wird jetzt in
   einem Lese-Thread geleert, während auf das Prozessende gewartet wird.
   Test: `worker_output_beyond_the_pipe_buffer_arrives` (90 kB).
2. **Fork-Race** (`acp/client.rs`, `harness/mod.rs`): `SessionCreated`
   trägt jetzt `origin` (`SessionOrigin::{New, Probe, Fork}`),
   `RequestKind::ForkSession` trägt `message_id`. Die Fork-Antwort wird
   nicht mehr über `fork_pending.take()` geraten, `probe_pending` ist ganz
   entfernt (die Probe-Erkennung lief über Ordner und Reihenfolge), und
   `connection_lost` leert `fork_pending`. Der Start-Race-Hack „Probe wird
   als echte Sitzung konsumiert“ ist weg: Die Probe wird immer geschlossen,
   die eigene `session/new`-Antwort des Users kommt als `New`. Tests:
   `fork_answer_is_not_stolen_by_a_racing_session_new`,
   `connection_lost_clears_the_pending_fork`,
   `probe_race_keeps_the_message_in_the_real_session`.
3. **Unerwartete Antwortform** (`acp/client.rs` `route_response`):
   Parse-Fehler werden als `RequestFailed` mit der Anfrage gemeldet (Code
   `-32602`), nicht mehr nur als `ProtocolError`. `TurnUsage`-Felder,
   `currentValue: null` und fehlendes `priority` sind jetzt über
   `#[serde(default)]` verträglich. Tests:
   `unparsable_result_becomes_request_failed_for_the_request`,
   `partially_worded_answers_still_parse`.
4. **PNG-Vorschau** (`harness/ui/artifacts.rs`): dekodiert mit dem
   `image`-Crate (war schon Dependency, nur `png`-Feature),
   `ctx.load_texture`, Cache pro Pfad und mtime. Test:
   `png_texture_decodes_and_caches_until_mtime_changes`.
5. **settings.json** (`store.rs`): `write_generic` schreibt atomar
   (Temp-Datei + `sync_all` + `rename`, gemeinsamer `write_atomic` mit
   `write_private`). `load_settings` liefert `Result`: fehlende Datei =
   Standardwerte (erster Start), Lesefehler = Meldung; die unlesbare Datei
   wird vor dem nächsten Save als `settings.json.unlesbar-<ns>` gesichert
   (`backup_unreadable_settings`), der Status zeigt beides an. Die
   `panel_closed`-Migration (liest `[bool; 2]` und `bool`, Code-Wert
   gewinnt, `panel_closed_compat`) kam mit dem Umbau-Zweig und ist drin.
6. **Tasten 1–4 bei Freigaben** (`harness/ui/cards.rs`): Zahlentasten
   antworten nur „Einmal erlauben“ und „Ablehnen“ (durchnummeriert nach
   berechtigten Optionen, nicht nach Vibes Listenposition); die
   dauerhaften „Immer“-Optionen sind nur per Klick erreichbar. Eine Karte,
   die gerade erschienen ist, ignoriert Ziffern für 1 s
   (`PERMISSION_KEY_LOCKOUT`, `PendingPermission.shown_at`), damit eine
   für den Composer getippte Ziffer keine ungesehene Karte beantwortet.
   Vorbild: die Vibe-CLI ist modal (Eingabezeile wird während der Freigabe
   ersetzt, `ApprovalApp` in `vibe/cli/textual_ui`), deshalb ist dort
   Taste 3 unproblematisch. Tests: `option_digits_skip_lasting_decisions`,
   `fresh_cards_ignore_number_keys_for_a_moment`.
7. **Einklappen pro Runde** (`acp/session.rs` `fold_rounds`): gelöst über
   `SessionView.turn_start` aus dem Umbau-Zweig — die Runden des aktuellen
   Turns (bis `turn_start`, `None` = die letzte) bleiben offen, solange er
   läuft oder fehlschlug; fertige Runden davor bleiben eingeklappt. Das
   ist genauer als der erste Fix hier („nur die letzte Runde“), der wurde
   fallengelassen. Die zwei Tests, die das alte Verhalten festgeschrieben
   hatten, prüfen jetzt das neue Verhalten (User hat zugestimmt); der
   Abbruch-Fall ist zusätzlich abgesichert.

**Bestehende Tests, die in dieser Sitzung angefasst wurden:** die beiden
Falt-Tests (Bug 7), `fork_copies_the_transcript_into_a_new_active_session`
und `probe_session_new_is_marked_and_still_routes` (Bug 2: Ereignis-Form),
`probe_session_only_teaches_choices_and_defaults` und
`failed_probe_new_session_keeps_the_queued_prompt` (Bug 2: `origin` statt
`probe_pending`), `probe_race_keeps_the_message_in_the_first_session` →
`probe_race_keeps_the_message_in_the_real_session` (der alte Test zementierte
das Konsumieren der Probe als echte Sitzung). Nicht abgeschwächt: Jeder Fix
hat neue Tests, die Verhaltensgrenzen bleiben geprüft.

Weiter offen aus dem Review: die „Weiteren Befunde“ unten. Der Code/Chat-
Umbau ist erledigt und hier drin (siehe Zustand oben); die zwei leeren
Branches `vibe/artifacts-view-a6b848` und `vibe/ui-js-worker-timeout-a6b848`
können auf dem Remote weg.

**Testfalle:** `store::tests::write_private_failure_keeps_old_file_and_cleans_up_temp`
schlägt fehl, wenn die Tests als root laufen (Container): root darf in das
0500-Verzeichnis schreiben. Kein Fehler im Code.

## Übergabe (Stand 2026-11-09, älter)

Alles unterhalb dieses Abschnitts ist älter und in Teilen überholt. Bei
Widersprüchen gilt dieser Abschnitt, danach der Code.

**Zustand:** `cargo fmt --check` und `clippy --all-targets` sauber. Tests:
285 Unit + 3 Integration grün, 2 ignored. Seit dem Abschnitt darunter sind
aus der Ideenliste gebaut: Schleifen-Wächter (`b7e6751`), fertige Runden
einklappen (`8c5179a`), Änderungen pro Runde mit `_review/turnDiff`
(`b08d394`). Der Code/Chat-Umbau ist weiter nicht begonnen und nicht
freigegeben.

**Testfalle:** `store::tests::write_private_failure_keeps_old_file_and_cleans_up_temp`
schlägt fehl, wenn die Tests als root laufen (Container): root darf in das
0500-Verzeichnis schreiben. Kein Fehler im Code.

### Offene Bugs aus dem Code-Review vom 2026-11-09

Nichts davon ist behoben. 1 bis 3 wurden mit temporären Tests nachgestellt
(wieder entfernt), 4 folgt aus der egui-Quelle, 5 bis 7 sind gelesen.

1. **ui-js-Ergebnis über 64 KiB: 2 s Einfrieren, dann „Zeitlimit“**
   (`script.rs` `run_worker`). Der Elternprozess wartet auf das Prozessende
   und liest stdout erst danach; der Worker blockiert am vollen Pipe-Puffer.
   Gemessen mit dem echten Binary: 30 kB in 26 ms, 90 kB Fehler nach 2,0 s.
   Dokumentiert sind 256 KiB. Fix: stdout lesen, während gewartet wird
   (Lese-Thread), der Integrationstest nutzt `wait_with_output` und deckt
   den Fall deshalb nicht ab.
2. **Fork-Antwort landet bei der falschen Sitzung** (`harness/mod.rs`,
   `SessionCreated`, `fork_pending.take()`). Das nächste beliebige
   `SessionCreated` gilt als Fork-Antwort, und `connection_lost` leert
   `fork_pending` nicht. Nachgestellt: (a) `vibe-acp` stirbt während eines
   Forks, nach dem Neustart wird die Probe-Sitzung zur aktiven „Abzweigung“,
   `probe_pending` bleibt gesetzt. (b) Ein anderes `session/new` antwortet
   vor dem Fork: der neue Chat bekommt den fremden Verlauf, `queued` bleibt
   für immer stehen, jedes Senden endet mit „Vibe arbeitet noch“.
   Fix: `route_response` (`acp/client.rs`) gibt die auslösende Anfrage in
   `SessionCreated` mit (Fork, Probe oder normal) statt sie zu verwerfen;
   das ersetzt auch die Probe-Erkennung über Ordner und Reihenfolge.
3. **Unerwartete Antwortform lässt die Sitzung hängen** (`acp/client.rs`
   `route_response`). Scheitert das Parsen einer Antwort, entsteht nur ein
   `ProtocolError`, die Anfrage ist dann vergessen. Nachgestellt:
   `session/prompt` mit unvollständigem `usage` (`TurnUsage` verlangt drei
   Felder), der Turn bleibt auf „läuft“; `session/new` mit einer
   select-Option, deren `currentValue` null ist, die erste Nachricht bleibt
   stecken. Auch ein `plan`-Eintrag ohne `priority` verwirft das ganze
   Update. Fix: Parse-Fehler als `RequestFailed` mit der Anfrage melden und
   die inneren Felder in `acp/protocol.rs` mit `#[serde(default)]` lockern.
4. **PNG-Vorschau in „Artefakte“** (`harness/ui/artifacts.rs`,
   `egui::Image::from_bytes`). egui 0.29 hat ohne `egui_extras` keinen
   Image-Loader, im Projekt ist keiner registriert, das ergibt
   `LoadError::NoImageLoaders`. Nicht laufend gesehen (kein Display). Fix
   ohne neues Crate: mit dem `image`-Crate dekodieren, `ctx.load_texture`,
   pro Pfad und mtime cachen.
5. **`settings.json` wird nicht atomar geschrieben** (`store.rs`
   `write_generic`), und `load_settings` fällt bei jedem Lesefehler still
   auf Standardwerte zurück; der nächste Save überschreibt die Datei, die
   Projektliste ist weg. Der atomare Writer `write_private` liegt daneben
   als toter Code. **Wichtig für den Code/Chat-Umbau:** wird `panel_closed`
   von `[bool; 2]` zu einem Wert, scheitert das Parsen jeder alten Datei
   und setzt alles zurück. Das Feld braucht eine Migration (beide Formen
   lesen).
6. **Tasten 1 bis 4 beantworten Freigaben sofort** (`harness/ui/cards.rs`
   `permission_cards`, `take_digit`), sobald der Entwurf leer ist. Erscheint
   die Karte beim Tippen einer Ziffer, ist die Freigabe erteilt; Taste 3 ist
   bei Vibe „Always allow“ (dauerhaft). Entscheidung des Users offen, etwa
   Ziffern nur für „einmal erlauben“ und „ablehnen“ oder eine kurze Sperre
   nach dem Erscheinen.
7. **Eingeklappte Runden klappen bei jedem laufenden Turn alle auf**
   (`acp/session.rs` `fold_rounds`): die Entscheidung hängt am Turn-Zustand
   der Sitzung statt an der einzelnen Runde, ebenso nach einem Abbruch. Der
   Test `several_rounds_fold_independently_and_keep_their_keys` schreibt das
   fest; eine Änderung fasst also einen bestehenden Test an (dem User sagen).

### Weitere Befunde — abgearbeitet 2026-11-10 (Nachmittag)

- **`#[allow(dead_code)]` auf `mod acp` ist weg** (0 Warnungen). Konkret:
  `nextCursor` von `session/list` wird jetzt gelesen (Harness holt Folgeseiten
  per `list_sessions_page`, `list_pages` merkt sich, ob angehängt oder
  ersetzt wird; Test `session_list_pages_are_fetched_and_appended`);
  `promptCapabilities.image` steuert, ob PNG-Anhänge als `image`-Block
  gehen oder als `resource_link` (Test
  `png_attachments_follow_the_agents_image_capability`);
  `AcpClient::shutdown()` wird beim App-Ende gerufen (`App::on_exit` →
  `harness.shutdown_agent()`), `vibe-acp` bekommt also kein SIGKILL mehr.
  `SessionClosed` entfernt jetzt die lokale Sicht, `Exited.code` steht in
  der Failed-Meldung. Der wirklich tote Code ist gelöscht
  (`EditedFile`/`edited_files`, `SessionCapabilities::{can_list, can_close}`,
  `ConfigOption::id`, `AcpClient::stderr_tail`); `from_streams` und die
  `Round`-Helfer sind `#[cfg(test)]`. Die Wire-Format-Structs in
  `acp/protocol.rs` behalten ihre Felder (Doku des Protokolls) mit
  punktuellem `#[allow(dead_code)]`.
- **Per Frame:** In „Artefakte“ wird der `stat` je Datei 1 s gecacht
  (`artifacts::file_state`) und der Vorschau-Read pro Pfad und mtime
  (`artifacts::cached_capped`). **Weiter pro Frame:** Markdown-Parsing und
  der LCS-Diff offener Diffs (`transcript::diff_body`) — größerer Umbau,
  noch offen.
- **`harness/watch.rs`:** `git` läuft mit `--no-optional-locks` und bekommt
  ein 5-s-Zeitlimit (danach Kill; stdout im Lese-Thread, gleiche Pipe-Form
  wie `script.rs`).
- **`hint_pending` ist pro Sitzung** (`HashSet<String>` statt globalem
  Flag); `file://`-URIs werden prozent-kodiert (`harness::file_uri`, Test
  `file_uri_percent_encodes_spaces_and_specials`).
- **Überbleibsel weg:** `app/mechanics.json` gelöscht (MECHANICS.md
  erwähnt nur noch die Herkunft), Fenstergrößen-Kommentar aus `main.rs`,
  `clientInfo.name` heißt jetzt `"catwright"` (docs/acp.md
  mitgeändert).
- **Ideenliste 7 (klein), gebaut:** „Erneut versuchen“ nach einem
  fertigen Turn (`/retry` ist ein echtes Vibe-Command, Fixture bestätigt
  es); Plan-Karte im Panel nur, wenn nach der letzten User-Nachricht ein
  Plan-Item kam; Auto Approve fragt nach Bestätigung
  (`Confirm::AutoApprove`); `vibe-acp` wird zusätzlich unter
  `~/.local/bin` gesucht; `dist/catwright.desktop` liegt bei
  (Installation weiter manuell).
- **Struktur:** `Harness` hat weiter ~80 Felder; ein Aufteilen von
  `harness/mod.rs` steht noch aus.
- **Branches:** `claude/dreamy-carson-o2i16m` gelöscht; nur noch `main`.

**Ideenliste 4 (Runden-Navigation) ist gebaut** (2026-11-10, Branch
`vibe/round-nav-94e08e`, auf dem Cleanup-Zweig aufgesetzt): Beim Zeichnen
wird die y-Position jedes Rundenanfangs festgehalten (auch einer
eingeklappten — ihr Summary-Row ist der Rundenanfang,
`UiState.round_tops`); zwei Pills links unten („Vorherige Runde“) und
rechts unten („Nächste Runde“) springen über `scroll_to_round` zur
nächsten Runde über/unter der aktuellen Scroll-Position (Test
`round_jump_target_picks_the_nearest_round`). Nicht live am Display
geprüft, nur headless.

**Restliste abgearbeitet 2026-11-10 (Nachmittag, Branch
`vibe/round-nav-94e08e`, auf dem Cleanup-Zweig):**

- **Per-Frame-Cache:** `src/memo.rs` (kleiner, begrenzter Memo-Speicher;
  volles Cache wird komplett geleert). `markdown::parse_cached`,
  `diff::folded_cached`, `session::diff_stat_cached`; Transkript,
  Freigabekarten, Turn-Diffs und Panel nutzen die gecachten Varianten.
  Tests: `memo_reuses_and_caps`, `folded_cached_matches_the_direct_computation`,
  `parse_cached_matches_parse`.
- **Sitzungen pinnen:** Sitzungs-Menü „Anpinnen/Lösen“, Pin-Markierung
  statt Altersanzeige, Angepinnte sortieren in ihrem Workspace nach
  vorn (Zeitordnung bleibt innerhalb der Gruppen). Persistiert über
  `settings.pinned_sessions`, Sync wie `sync_projects`. Neues Icon
  `Pin`. Test: `sidebar_entries_sort_pinned_first`.
- **Klickbare Pfade (Idee 5):** Tool-Zeilen mit Datei-Location bekommen
  einen Datei-Knopf rechts; der Klick öffnet die Datei in der
  Artefakte-Vorschau (`h.open_artifact`, relative Pfade gegen den
  Workspace der Sitzung aufgelöst; Fetch ausgenommen — seine Location
  ist eine URL). Ein kompletter Dateibaum blieb bewusst weg: Die
  Seitenleiste listet Workspaces und Sitzungen, ein Baum dort wäre
  ein eigenes UI-Konzept (Ansatz im Backlog).
- **Syntax-Tokenizer (Idee 7):** `src/syntax.rs`, ein Durchlauf —
  Keywords (Rust, Python, JS/TS, Bash, JSON, Go), Strings mit
  Escapes, Zeilen-Kommentare je Sprache, Zahlen; `render_code_block`
  zeichnet die Runs als LayoutJob. Unbekannte Sprachen bleiben
  einfarbig.
- **Worktree pro Workspace (Idee 6):** Palette „Worktree für Projekt“:
  `git worktree add -b catwright/<name>` unter dem
  App-Konfigurationsordner (nie im Repository — sonst tauchte er als
  untracked auf), die Kopie wird eigener Workspace. Tests laufen
  einen echten git-Zyklus (init/add/commit/worktree).
- **`harness/mod.rs` geteilt:** die ~3.600 Zeilen Tests liegen jetzt in
  `harness/tests.rs`; `mod.rs` ist von 6.400 auf 2.700 Zeilen
  geschrumpft. Implementierung unverändert.

**Damit ist die offene Liste leer.** Was bleibt, ist Verifikation am
lebenden Gerät (PNG-Vorschau, Runden-Pills, Pin-Optik, Tokenizer-Farben,
Worktree im Alltag) und die zwei ignorierten Tests mit echtem
`vibe-acp`.

**Review-Nacharbeit (gleicher Tag, zweite Session):** Der Code-Review
der Restliste fand zwei Fehler, beide gefixt: Der Tokenizer baute
Zeichen außerhalb von Strings/Kommentaren byte-weise als Latin-1
wieder zusammen (aus `ö` wurde `Ã¶`) — der Lauf arbeitet jetzt
zeichenweise (Test `multi_byte_text_stays_intact_outside_strings_and_comments`);
und der Worktree-Test committe ohne globale Git-Identität (auf Rechnern
mit nur per-Repository-Identität rot) — das Fixture setzt jetzt
`-c user.name/-c user.email`.

**Live-Verifikation (User, gleicher Tag):** Der erste Testlauf lief
gegen einen zwei Tage alten Release-Binary (8.11., Stand vor dem Umbau)
— beim Senden einer Nachricht mit Umlaut verschwand das Fenster lautlos
(keine Panic im Journal, kein Coredump, auch Enter-Moment bestätigt).
Auf dem frisch gebauten Release (Stand dieser Übergabe) nicht
reproduzierbar. Neu ist der erste Vollrender-UT
(`the_full_ui_renders_multi_byte_text_without_panicking`): Die ganze
Harness-UI läuft headless über `egui::Context::run`, inklusive Fonts —
Draft, Nutzerzeile mit Umlaut, Sidebar-Titel und Rust-Codeblock.

**Release-Bereitschaft (gleicher Abend):** Version `0.2.0`
(`Cargo.toml`, gemeldet über `clientInfo.version`). Fenster- und
Taskbar-Icon: `app/assets/icon.png`, aus `theme::LOGO` und `GRADIENT`
generiert, per `include_bytes` eingebettet, Test
`the_bundled_window_icon_is_square_and_opaque`. Installation ohne root:
`dist/install.sh` kopiert Binary nach `~/.local/bin`, Menu-Eintrag und
Icon — der Launcher startet damit nie wieder einen Alt-Build.
**Offen:** Endprodukt-Prüfung (externe Session), danach Tag `v0.2.0`
und optional ein GitHub-Release. Nicht freigegeben: der Tag selbst.

### Empfohlene Reihenfolge

2 und 3 zusammen (eine Änderung an `route_response` und `AcpEvent`), dann
1, 5 und 4 (jeweils klein), 6 nach Entscheidung des Users, 7 nach Rückfrage
wegen des Tests.

## Übergabe (Stand 2026-11-08, älter)

**Zustand:** Code und Chat sind zu einer Ansicht zusammengelegt (Abschnitt
„Erledigt: Code und Chat zusammengelegt“ unten), auf dem Branch
`claude/great-meitner-lwkox2` (auf `main` = `b08d394` aufgesetzt, noch nicht
nach `main` gemergt). Die Branches `vibe/artifacts-view-a6b848` und
`vibe/ui-js-worker-timeout-a6b848` sind vollständig in `main` enthalten und
können weg. fmt, `clippy --all-targets -- -D warnings` und `cargo test` sind
grün: **292 + 3 Tests, 2 ignored** (die 2 brauchen ein echtes `vibe-acp`).

**Achtung root:** In einem Container, der als root läuft (Cloud-Sitzung),
scheitert `store::tests::write_private_failure_keeps_old_file_and_cleans_up_temp`,
weil root trotz 0500 in den Ordner schreiben darf (`CAP_DAC_OVERRIDE`). Als
normaler User läuft er durch. Das ist kein Fehler im Code.

**Seit der Übergabe darunter dazugekommen** (Ideenliste 1–3, je mit Tests):

- **Schleifen-Wächter** (`b7e6751`): `session::repeated_tool_call` zählt
  seit dem letzten `Item::User` aufeinanderfolgende Tool-Calls mit gleichem
  `title` und `raw_input`, ab `LOOP_THRESHOLD` = 3 zeigt `cards::loop_guard`
  eine Karte mit „Turn abbrechen“ (`h.cancel()`).
- **Fertige Runden einklappen** (`8c5179a`): `session::{rounds, fold_rounds,
  round_key}`, gezeichnet in `transcript.rs`, aufgeklappt über `open_groups`.
- **Änderungen pro Runde** (`b08d394`): `extras::parse_review_state` liest
  `scopes[].owner.turnId`, `ExtKind::ReviewTurnDiff` holt `_review/turnDiff`
  beim Klick, Zustand in `state::TurnDiffView`. Nach einem Rewind bleiben die
  Dateien gestrichener Runden als „ohne Runde“ sichtbar (Test
  `after_a_rewind_the_cut_rounds_changes_have_no_round_left`).

**Beim Nachtragen aufgefallen, nicht geändert:**

- ~~Solange ein Turn lief, klappten alle älteren Runden wieder auf~~
  (behoben 2026-11-09): `SessionView::turn_start` merkt sich, wo der Turn
  begann; `fold_rounds` hält nur dessen Runden offen (auch vor Vibes Echo
  klappt nichts auf). Zwei Prüfungen, die das alte Verhalten festschrieben,
  sind angepasst (der User weiß es): in `several_rounds_fold_independently_and_keep_their_keys`
  und in `finished_round_folds_after_the_turn_ends` (dort sagte der Kommentar
  schon „the earlier finished round stays folded“). Neu:
  `only_the_latest_turns_rounds_unfold`.
- Ein gescheitertes `session/prompt` setzt keinen `last_stop_reason`, die
  Runde klappt also trotzdem ein (im Test so dokumentiert). Die Regel „bei
  Fehler nichts einklappen“ greift nur bei einem Fehler-`stopReason`.

**Zwei bestehende Tests wurden bewusst geändert (der User weiß es):**
`fork_copies_the_transcript_into_a_new_active_session` prüft jetzt `sessionId`
statt `forkSessionId` (der alte Test zementierte den Bug), und
`cli_tab_maps_to_nav_entry` erwartet 5 statt 3 Einstellungsseiten.

### Was in dieser Sitzung entstanden ist

- **UI komplett neu:** `harness/ui.rs` (Shell, Kürzel, Toasts, Dialoge) und
  `harness/ui/{sidebar,header,transcript,cards,composer,panel,palette,artifacts}.rs`,
  dazu `widgets.rs`, `icons.rs`, neu aufgebautes `theme.rs`, `app/settings.rs`.
  Schrift: erster Treffer aus `fonts::UI_FONT_CANDIDATES` (hier Adwaita Sans).
- **Neue Logik:** `diff.rs` (LCS-Zeilendiff), `harness/extras.rs` (Payloads der
  Vibe-Erweiterungen, deutsche Texte), `harness/watch.rs` (Git-Status,
  Dateiliste für `@`), `harness/state.rs` (`UiState`, Dialoge, Queue).
- **Über ACP-Erweiterungen, gegen echtes `vibe-acp` geprüft:** Zurückspulen,
  Fork ab Nachricht, Umbenennen, Löschen, Routinen, Review, Konnektoren, Konto
  und Budget, Trust-Banner, Rückfragen (`elicitation`). Details: `docs/acp.md`.
- **Rein clientseitig:** Warteschlange beim Tippen während eines Turns, Entwurf
  pro Sitzung, Palette (Strg K), `/`- und `@`-Listen, Freigabe mit Tasten 1–4,
  echte Diffs, Tabellen-Layout, `--prompt` als Startparameter.
- **Bugs behoben:** Fork war nie funktionsfähig; `[Catwright]`-Hinweis stand
  in Titel und Bubble (`without_ui_hint`, `display_title`); Shift+Enter war tot;
  Hover verschob Zeilen; Palette sprang beim Scrollen hoch.

### Entscheidungen und Wünsche des Users

- **Umgesetzt (2026-11-08 abends, vom User freigegeben):** Code/Chat zu einem
  Modus zusammengelegt, der Workspace wird pro Sitzung gewählt, wie beim
  DeepSeek Harness. Der User hat zugestimmt, dass dafür die Tests geändert
  werden, die an zwei Ansichten hängen (Liste unten).
- Schleifen-Wächter, Einklappen und Änderungen pro Runde sind gebaut (oben).
- Modusfarben (blau, grün, amber, rot, lila bei eigenen Profilen) sind sein Wunsch.
- Weiter gültig: UI deutsch, Rust only, keine neuen Crates, keine ungefragten
  Netzwerkaufrufe, exakte Werte statt Schätzungen, Kosten oder Änderungen an
  `~/.vibe` vorher fragen, Antworten auf Deutsch.

### Erledigt: Code und Chat zusammengelegt

Eine Ansicht, ein Seitenleisten-Baum: oben angepinnt der Workspace
„Allgemein“ (`chat_dir`, `~/.config/catwright/chat-workspace`), darunter
die Projekte. Kein Umschalter, kein Strg 1/2, kein „Startmodus“ mehr.

- **Harness:** `View`, `parked_active`, `parked_project`, `reload_on_ready`,
  `set_view`, `cwd_belongs_to_current_view`, `parked_wait_hint` und
  `views_waiting_permission` sind weg. Neu: `GENERAL_WORKSPACE`, `workspaces()`,
  `is_chat_dir`, `workspace_name`, `select_workspace`, `activate` (zeigt eine
  Sitzung und macht ihren Ordner zum Workspace neuer Sitzungen),
  `fallback_workspace`, `workspace_waiting`; rein: `last_used_workspace`,
  `workspace_waiting_permission`.
- **Regeln:** `current_project` ist der Workspace der nächsten neuen Sitzung.
  Er folgt der geöffneten Sitzung, der Workspace-Zeile und dem Picker auf dem
  Startbildschirm. Bis der User in einem Lauf selbst wählt, gilt der Workspace
  mit dem neuesten `updatedAt` (bei jeder `session/list`-Antwort neu berechnet);
  vor der ersten Liste das erste Projekt, ohne Projekte „Allgemein“.
  „Allgemein“ ist nie in `projects` (ein alter Eintrag fällt beim Start weg,
  „Projekt hinzufügen“ mit diesem Ordner wählt ihn nur aus) und nicht
  entfernbar. Die Probe-Sitzung bleibt im Chat-Ordner.
- **Bewusst anders als vorher:** Eine von Vibe angelegte Sitzung wird immer
  angezeigt (kein Parken mehr; sonst stünde die Nachricht in einer Sitzung, die
  die Seitenleiste bis zum Turn-Ende nicht zeigt). Nach einem Neustart von
  `vibe-acp` lädt nur die Sitzung auf dem Bildschirm neu, andere offene beim
  nächsten Klick. Ein wartender Request markiert die Workspace-Zeile gelb, auch
  eingeklappt (ersetzt das Badge am Umschalter). Der Sitzungskopf zeigt immer
  den Workspace, auch „Allgemein“.
- **App/Einstellungen:** `UiMode`, `set_mode`, `Settings.ui_mode`,
  `HarnessAction::SwitchTo*` sind weg. `panel_closed` ist ein `bool`; eine alte
  `settings.json` mit `[code, chat]` lädt weiter (Code-Wert gilt), sonst wären
  beim Laden die Projekte verloren (Test
  `settings_from_the_two_view_era_keep_loading`). `--mode code|chat` wird samt
  Wert übersprungen. `widgets::segmented` und `Icon::Code` waren nur für den
  Umschalter da und sind entfernt.
- **Geänderte Tests (der User weiß es):** `views_keep_their_own_active_session_and_project`
  → `opening_a_session_moves_to_its_workspace`;
  `new_session_of_the_other_view_is_parked_not_activated` →
  `new_session_starts_in_the_workspace_of_the_last_opened_session`;
  `same_view_set_view_keeps_artifacts_open` → `artifacts_view_closes_only_on_navigation`;
  `views_waiting_permission_attribution` → `workspace_waiting_permission_attribution`
  (gleiche Fälle, pro Workspace); `parked_view_wait_hint_shown_only_when_the_other_side_waits`
  → `workspace_row_marks_waiting_sessions`;
  `view_switch_during_reconnect_keeps_each_views_session` →
  `reconnect_reloads_the_session_on_screen_into_its_workspace`;
  `mode_arg_accepts_code_and_chat_only` → `old_mode_arg_is_skipped_with_its_value`.
  Angepasst ohne Abschwächung: `probe_race_keeps_the_message_in_the_first_session`
  und `failed_probe_new_session_keeps_the_queued_prompt` (`select_workspace`
  statt `set_view`), `failed_load_leaves_no_empty_view` und
  `remove_project_closes_its_sessions_and_active` (Teil zu `parked_active` durch
  das Gegenstück in einer Ansicht ersetzt), `settings_changed_detects_differences`
  (`auto_account` statt `ui_mode`). Neu: `last_used_workspace_is_the_one_with_the_newest_session`,
  `startup_preselects_the_last_used_workspace_until_the_user_picks`,
  `chat_workspace_is_pinned_and_never_a_project`,
  `settings_from_the_two_view_era_keep_loading`.
- **Geprüft:** Screenshots unter Xvfb mit einem Fake-`vibe-acp` (Skript im
  Scratchpad, nicht im Repo): Startbildschirm wählt „Allgemein“, wenn dort die
  neueste Sitzung liegt; Sitzung in „Allgemein“ per `--project <chat-workspace>
  --session <id>` mit Freigabekarte, Wartemarke an der Workspace-Zeile, Kopf
  „ALLGEMEIN ›“. Eine alte `settings.json` (`ui_mode`, Panel-Paar) behielt ihre
  Projekte. **Nicht** gegen echtes `vibe-acp` und nicht von Hand geklickt.
- **Offen/aufgefallen:** ~~Öffnet man während eines Neustarts von `vibe-acp`
  eine Sitzung, entsteht eine leere Ansicht~~ (behoben 2026-11-09: der Klick
  wird in `open_on_ready` gemerkt und nach dem Neustart geöffnet, „Neue
  Sitzung“ verwirft ihn; Test `session_clicked_during_a_restart_opens_once_vibe_is_ready`).
  Mit dem Fake zeigte der Kopf „Neue Sitzung“,
  weil der Fake kein `session_info_update` schickt; ob echtes `vibe-acp` den
  Titel beim Laden schickt, ist ungeprüft.
- Screenshots in der Cloud-Sitzung: `apt-get install -y libxkbcommon-x11-0`,
  dann `xvfb-run -a -s "-screen 0 1440x900x24 +extension GLX" env
  XDG_CONFIG_HOME=<leer> HOME=<leer> target/debug/catwright --screenshot out.png`.

### Ideenliste (1–3 gebaut, 4–7 offen)

1. ~~Schleifen-Wächter~~ erledigt (`b7e6751`). In den Agenten eingreifen geht
   nicht, ACP hat dafür nichts; die Karte bietet nur den Turn-Abbruch.
2. ~~Fertige Runden einklappen~~ erledigt (`8c5179a`), Abweichungen oben.
3. ~~Änderungen pro Runde~~ erledigt (`b08d394`). Live gegen `vibe-acp` noch
   nicht gesehen: ob alte, nachgeladene Sitzungen `scopes` liefern.
4. **Runden-Navigation:** y-Positionen der `Item::User` beim Zeichnen merken,
   Sprung über `vertical_scroll_offset`.
5. **Dateibaum und klickbare Pfade:** `watch::files` plus Vorschau aus
   `ui/artifacts.rs`. Inline-Code in `render.rs` läuft als LayoutJob, klickbare
   Pfade brauchen den Widget-Weg wie Links.
6. **Worktree pro Sitzung:** `git worktree add`, dann `session/new` mit dem
   Worktree als `cwd`. `session/list` liefert nur exakte `cwd`-Treffer, also ist
   jeder Worktree ein eigener Workspace.
7. **Klein:** Bestätigung vor Auto Approve (`composer::mode_chip`, neue
   `Confirm`-Variante); „Erneut versuchen“ per `run_command("/retry")` in
   `transcript::turn_footer`; Plan in `panel::plan_card` nur zeigen, wenn nach
   dem letzten `Item::User` ein `Item::Plan` kommt; Sitzungen pinnen (Settings);
   eigener kleiner Syntax-Tokenizer in `render_code_block`; `vibe-acp` beim
   Start aus dem App-Menü auch unter `~/.local/bin` suchen (`ensure_started`),
   eine `.desktop`-Datei gibt es noch nicht.

**Geht nicht oder lohnt nicht:** Steering in laufende Turns (nur Vibe-intern,
nicht in `vibe/acp/`), Goal-Modus (gibt es in Vibe nicht), Agent-Teams,
Plugin-Marktplatz, Terminal und Browser in der App, Bilder aus der
Zwischenablage (egui liefert keine, `wl-paste` fehlt hier).

### Nicht verifiziert, nicht als erledigt behaupten

- Nichts lief bisher gegen ein **echtes Modell**, nur gegen ein Skript-Modell.
- Klicks und Mausrad waren simuliert (über den echten egui-Eingabeweg), nicht
  von Hand. Der User testet parallel und meldet weitere Funde.
- `_connectors/toggle` nie live; `_session/retrying` nur per Unit-Test; das
  tatsächliche Auslösen einer Routine nie beobachtet.
- Der Toast „gilt ab der nächsten Sitzung“ nach einer Trust-Entscheidung ist
  eine Annahme; nicht geprüft, ob die laufende Sitzung schon neu lädt.
- `_review/approve` mit `lastTurns: 1` leerte im Test die gesamte Liste, die
  Semantik ist nicht ganz geklärt. `revert` für Datei und `all` funktionierte.
- `RequestUserAttention` und Fenstertitel unter KDE/Wayland nicht beobachtet.

### Bekannte Schwächen

Forks haben keine Message-IDs, bis sie neu geladen sind (Rewind-Knöpfe fehlen
dort); viele angepinnte Karten können den Verlauf verdrängen; Markdown wird
jeden Frame neu geparst; `is_error_status` erkennt Fehler an Stichwörtern; der
Startbildschirm hat einen Entwurf für alle Workspaces (der Text bleibt beim
Wechsel des Workspaces stehen).

### Stolperfallen

- **Nie `ui.put` auf bereits belegte Zeilen:** es setzt den Layout-Cursor
  zurück. Immer `widgets::place` nehmen (Regressionstest vorhanden).
- **Kein `scroll_to_rect` pro Frame**, nur einmalig nach einer Aktion.
- **Screenshots ohne Mauszeiger zeigen keine Hover-Fehler.**
- zsh: `echo ====X` ist ein Fehler (`=`-Expansion); Semikolons in erzeugten
  Skripten quoten.

### Testwerkzeuge (gesichert in `app/shots/dev-tools/`, gitignored)

Der Scratchpad verschwindet mit der Sitzung, deshalb liegt dort eine Kopie. In
den Skripten steht noch der alte Pfad in `S=`, den muss man anpassen.

- `fake-vibe-acp`: Stand-in, das geskriptete ACP-Daten abspielt (Szenarien
  `fresh`, `session`, `running`, `permission`), gehört als `vibe-acp` in den `PATH`.
- `shot.sh`: rendert im unsichtbaren `kwin_wayland --virtual`.
- `shotcopy.sh` mit `patch2.py` bis `patch4.py`: baut eine Wegwerf-Kopie des
  Crates mit `SHOT_STATE`, `SHOT_POPUP`, `SHOT_INPUT` (`85:move:680,300;95:wheel:-3000`).
- `e2e.sh`, `driver.rs`, `fake_llm.py`, `drive.py`: echter Ablauf gegen
  `vibe-acp` mit lokalem Skript-Modell und leerem `VIBE_HOME`, ohne Kontingent.

## UI-Redesign und Vibe-Erweiterungen (2026-11-08)

**Neu aufgebaut:** die gesamte Oberfläche (`harness/ui/`, `widgets.rs`,
`icons.rs`, `theme.rs`, Einstellungen). Design siehe `docs/design.md`, Protokoll
siehe `docs/acp.md`. Status: fmt, clippy (`--all-targets`, keine Warnungen),
`cargo test` grün (266 + 3 Tests, 2 ignored).

**Korrekturen an diesem Dokument:** Die Abschnitte weiter unten sagen an
mehreren Stellen, etwas sei mit ACP nicht möglich. Das stimmt für Vibe 3.1.0
nicht mehr; `vibe/acp/agent.py` hat Erweiterungen, die dokumentiert sind in
`docs/acp.md`:

- **Fork war kaputt:** `session/fork` schickte `forkSessionId`, Vibe verlangt
  `sessionId` (`-32602`). Behoben; der Test wurde auf das echte Feld geändert.
- **Routinen sind nicht blockiert** (`_loops/*`), jetzt in der rechten Leiste.
- **Umbenennen und Löschen** gehen über `_session/set_title` und
  `_session/delete`.
- Neu darüber hinaus: Zurückspulen mit Datei-Wiederherstellung, Fork ab einer
  Nachricht, Vertrauens-Banner, Konto und Kontingent, Review (annehmen/verwerfen),
  Konnektoren, Vibes Rückfragen als Karte (`ask_user_question` ist erst mit der
  `elicitation`-Capability verfügbar), Nachrichten einreihen während Vibe arbeitet.

**Wie es verifiziert wurde** (zum Wiederholen):

- Rendern ohne den Desktop zu stören: `kwin_wayland --virtual --no-lockscreen
  --socket <name> --width 1440 --height 900 --exit-with-session <skript>`, das
  Skript startet die App mit `--screenshot`, `XDG_CONFIG_HOME` zeigt auf einen
  leeren Ordner.
- Echter Agent ohne Kosten: `VIBE_HOME=<leer>` mit einer `config.toml`, die über
  `[[providers]] api_base = "http://127.0.0.1:PORT/v1"` auf einen kleinen lokalen
  OpenAI-kompatiblen Server zeigt, der Tool-Calls als SSE skriptet. Damit läuft
  echtes `vibe-acp` inkl. Checkpoints, Freigaben und Rückfragen, ohne Abo und
  ohne `~/.vibe` anzufassen. So wurde der ganze Ablauf durchgespielt.

- Hover, Klicks, Mausrad und Tasten: in einer Wegwerf-Kopie des Crates über
  `eframe::App::raw_input_hook` skriptete `egui::Event`s einspeisen. Sie laufen
  durch den normalen Eingabeweg, deshalb fallen damit auch Hover-Fehler auf, die
  ein Screenshot ohne Mauszeiger nie zeigt.

**Bewusst offen:** `_voice/*` (Diktat), Hunk-genaues Review (`_review/hunks`),
Einfügen von Bildern aus der Zwischenablage, Drag & Drop (Wayland-Support in
winit fehlt). Kontingent und Konto werden nur auf Anfrage geladen (oder mit
Einstellung „beim Start“), wegen der Regel „keine ungefragten Netzwerkaufrufe“.
Am echten Modell (statt Skript-Modell) noch nicht ausprobiert: wie gut es die
Rückfragen und Routinen tatsächlich nutzt.

Stand: `main`, alles committet. fmt, clippy (`-D warnings`) und 163 Tests
sind grün. Das Release-Binary ist gebaut. Regeln: `AGENTS.md`. Architektur:
`app/MECHANICS.md`. Protokoll: `docs/acp.md`.

Zeilennummern sind Stand dieses Commits (`~` = ungefähr).

## Abarbeitung (Nachtrag 2026-11-07 spät, Mistral Vibe Agenten)

**Pivot danach (gleicher Abend, User-Entscheidung): reiner Mistral-Harness.** Die HTTP-Provider-Welt ist vollständig entfernt
(`a611261`: backend.rs/sse.rs gelöscht, API-Chat-Modus samt Model-Picker-
Eintritt, providers.json/keys/chats-APIs, Chat-Provider-Untermenü,
--headless/--chat/--provider/--mode api, reqwest+zeroize aus den Deps;
−5121 Zeilen). Damit sind
auch die Fixes 2.1/2.8.5/2.8.6 (API-Chat) samt ihren Tests entfallen — sie
betrafen eine Welt, die es nicht mehr gibt; alte Chat-Dateien auf der Platte
werden weder gelesen noch gelöscht. Stand jetzt: **175 Tests, 2 ignored**,
clippy/fmt/Release-Build grün; Exception: zwei `#[allow(dead_code)]` mit
Begründungskommentar in store.rs (write_private und ui_dsl_spec — beide
getestet und bewusst für die 0600-/Skill-Sicherung gehalten).

**Modus-Anpassung (gleicher Abend, weiterer User-Wunsch):** Der Modus `ask`
heißt in der App überall „Plan“ (nur Label, Wire-ID bleibt `ask`); die zwei
Claude-Delegate-Profile (`claude-delegate`, `claude-delegate-rust`) sind aus
allen Auswahllisten gefiltert ( Präfix `claude-delegate`), ein laufender
Session-Chip zeigt ihn weiter, ein gespeicherter Startmodus mit toter ID wird
beim Setup ignoriert. `~/.vibe/agents/*.toml` blieb unberührt.

**ACP-Potenzial umgesetzt (2026-11-08, Vibe Agent):**
(1) **Session-Fork (`session/fork`):** Button „⑃ Sitzung forken“ in der
rechten Leiste (Umgebung). Wire gemäß ACP v1 (`forkSessionId` + `cwd` +
`mcpServers`, Antwort wie `session/new`); die neue Sicht zeigt die
lokale Kopie des Transkripts (der Fork-Response repliziert ihn nicht,
`SessionView::forked`), Quelle bleibt unberührt. Guards: kein Fork
während eines Turns, Fehler (z. B. -32601) landen in der Statuszeile.
**Noch live am PC verifizieren:** ob `vibe-acp` `session/fork` wirklich
implementiert (Capability-Flag „fork:{}“ ist im Mitschnitt, die Methode
selbst wurde noch nie beobachtet). Scheitert es, steht ein klarer
Fehler in der Statuszeile — nichts kaputt.
(2) **Kostenanzeige:** `usage_update.cost` wird im Kontext-Panel als
„Kosten (Turn): 0,1234 EUR“ gezeigt, wenn Vibe eins schickt.
(3) **Aktivitäts-Sortierung:** Seitenleiste sortiert nach `updatedAt`
(neueste zuerst, ohne Zeitstempel zuletzt).
(4) **Zusätzliche Ordner:** `additionalDirectories` aus `session/list`
erscheinen in der Umgebung. `cargo test` nun **199 passed, 2 ignored**.

**Parität-Nachtrag (2026-11-08, Vibe Agent, „alles was du kannst“):**
(1) **Suche:** „Suchen“-Zeile in der linken Seitenleiste (beide Modi) mit
Filterfeld — Chats nach Titel, Projekte nach Name oder Sitzungstitel
(`matches_query`/`filter_entries` in `harness/mod.rs`, mit Tests).
(2) **PNG-Anhänge als `image`-Blöcke:** `prompt_blocks` schickt PNGs bis
5 MiB als Base64-Block (eigener RFC-4648-Encoder, kein neues Crate,
`read_png`/`base64_encode` mit Testvektoren); größere/fehlende PNGs und
alle anderen Dateien bleiben `resource_link`. (3) **Routinen:**
blockiert, ACP bietet keine geplanten Loops (siehe Abschnitt 4).
`cargo test` nun **196 passed, 2 ignored**.

**Artefakte-Ansicht umgesetzt (2026-11-08, Vibe Agent, nach User-Entscheid
gegen das rechts-andockte Panel und für die Referenz-Anordnung):**
„Artefakte“ ist jetzt ein Eintrag in der linken Seitenleiste (beide Modi,
wie `reference/picture-3.png`), die Ansicht öffnet im Hauptbereich:
Liste aller vom Agenten bearbeiteten Dateien (abgeschlossene Diff-Tool-Calls,
neuester Touch pro Pfad, nach Projekt gruppiert) links, Typ-Vorschau
rechts — Markdown wie eine Chat-Nachricht, PNG als Bild, CSV als
Tabellen-Raster, HTML als Quelltext mit „im Browser öffnen“ (xdg-open,
nie WebView), sonst Monospace mit Kopieren. Vorschau von Platte, Cap
2 MiB; fehlende/zu große/unlesbare Dateien werden ehrlich gemeldet.
Neues Modul `app/src/artifacts.rs` (Collect/Kind/CSV-Parser/Load mit
Tests), UI in `harness/ui.rs` (`artifacts_entry`, `artifacts_view`,
`artifact_preview`), `render_table_grid`/`render_code_block` sind jetzt
`pub(crate)`. Bewusst: nur PNG (JPEG bräuchte einen `image`-Feature-Flag
= Dependency-Änderung), Markdown-Vorschau ist read-only (ui-js-Blöcke
zeigen dort ihren Quelltext). `cargo test` nun **196 passed, 2 ignored**.

**2.7 nachgebessert (2026-11-08, Vibe Agent, User-Freigabe „mach was
möglich ist“):** Das Zeitlimit ist jetzt echt. Jede ui-js-Auswertung
läuft in einem frischen Worker-Kindprozess (`catwright
--ui-js-worker`, eine JSON-Anfrage auf stdin, eine JSON-Antwort auf
stdout, `script.rs` `run_worker`). Die App wartet maximal `EVAL_TIMEOUT`
(2 s) und killt den Worker danach — der UI-Thread friert nicht mehr
unbegrenzt ein, ein OOM killt den Kindprozess statt der App. Ein
boa-Upgrade bleibt weiter unmöglich (0.22 ist die neueste Version,
weiterhin kein Interrupt-Hook). Bewusst ohne `RLIMIT_AS` (würde ein
neues Crate brauchen); die Bündelung beim Reglerziehen bleibt. Kosten:
~30 ms Prozessstart pro Auswertung (Release, gemessen). Tests: Unit
in `script.rs` (Protokoll, Timeout-Kill, Crash/Spawn-Fehler) plus
Integrationstest `app/tests/ui_js_worker.rs` gegen das echte Binary.
`cargo test` nun **189 passed, 2 ignored**.

Abschnitt 0 komplett abgearbeitet mit Subagenten; jeder Fix mit Tests im
gegebenen Stil. `cargo test` nun **222 passed, 2 ignored** (vorher 165),
`clippy --all-targets -D warnings` und `fmt --check` grün. Commits:

| Punkt | Status | Commit |
|---|---|---|
| 1a Seitenleiste sofort | erledigt | `07b5538` |
| 1b Kontextgröße nach Modellwechsel | erledigt | `0ae8f9d` |
| 2.1 API-Chat falscher Chat | erledigt | `a979dc6` |
| 2.2 Freigaben unbekannter Sessions | erledigt | `2e35cce` |
| 2.3 Probe-Rennen, 2.4 verlorener Text, 2.8.1, 2.8.2 | erledigt | `81d3e0b` |
| 2.5 UI-Zustand pro Chat, 2.8.10-Rest | erledigt | `42e4212` |
| 2.6 atomare 0600-Schreibvorgänge | erledigt | `c71a7eb` |
| 2.7 JS-Sandbox | teilweise, ehrlich | `88bc891` |
| 2.8.3, 2.8.4, 2.8.8 | erledigt | `047f680` |
| 2.8.5, 2.8.6, 2.8.10 (app.rs/main.rs) | erledigt | `a979dc6` |
| 2.8.7 Neuverbindung | erledigt | `2e35cce` |
| 2.8.9 Tooltip Modellwahl | erledigt | `0ae8f9d` |
| 3.1 Markdown-Tabellen | erledigt | `f597de0` |
| 3.2 Slider-Dezimalen, 3.3 computed-Tausenderpunkt | erledigt | `f597de0` |

Feststellungen und bewusste Abweichungen:

- **1b:** Per Mitschnitt (isoliertes `VIBE_HOME`, kein Modellaufruf)
  verifiziert: `vibe-acp` schickt **nach `set_config_option` kein**
  `usage_update` (nur bei `session/new` und während Turns). Quelle daher:
  `usage_update` > `[[models]].max_context_length` aus Vibes config.toml
  (zeilenweise geparst, 64-KiB-Cap, kein toml-Crate) > dokumentierter
  Builtin-Standard nur `mistral-large-4-0-3` = 800 000 > „–“ bis zum
  nächsten `usage_update`. `backend.rs`-Tabelle bleibt HTTP-Provider-Fallback.
  Der Wechsel schreibt `active_model` in Vibes config.toml (Tooltip 2.8.9).
- **2.2:** Orphan-Freigaben sind nach einer Neuverbindung bewusst tot
  (Request-ID stirbt mit dem Prozess): Karten werden geleert, Status nennt
  den Verlust, Vibe fragt ggf. neu. Beim Prüfen von 2.8.7 zeigte sich ein
  echter Bug: Ansichtswechsel während des Reconnects zerrte die Code-Session
  in den Chat-View — `Initialized` stellt Sessions jetzt in die View ihres
  Ordners zurück.
- **2.4:** Bei fehlgeschlagenem Senden wird nur der **Text** zurückgestellt,
  Anhänge nicht (konsistent mit `connection_lost`).
- **1a:** PromptFinished lädt nach **jedem** Turn die Liste neu (+1 RPC pro
  Turn, deckt spätere Umbenennungen ab). Titelableitung hängt am
  `user_message_chunk`-Echo.
- **2.7:** User-Entscheidung war „Bündeln + Zeitlimit“. Die Bündelung sitzt
  (max. eine ui-js-Auswertung pro 100 ms beim Reglerziehen, Start/Ende
  sofort). Das Zeitlimit ist mit der gepinnten **boa_engine 0.22.0 nicht
  möglich** — kein Interrupt-/Abort-Hook (im Crate-Quelltext verifiziert), ein
  solches Limit wäre vorgegaukelt. Loop-/Rekursions-Caps bleiben die einzige
  harte Grenze; ehrlich dokumentiert in `script.rs` und `MECHANICS.md`.
  Optionen für später: boa-Upgrade (Interrupt-Hook) oder Worker-Prozess.
- **2.8.8:** Mit Vibe 3.1.0 erneut verifiziert: Session ohne Nachricht
  hinterlässt nichts außer Log-Resten. Notiz in `MECHANICS.md`.
- **GUI nicht gestartet:** Alles per Testsuite verifiziert; die Optik der
  Markdown-Tabellen läuft über denselben Painter wie die (erprobten)
  ui-Tabellen. Ein manueller Blick auf die neue Modell-„–“-Anzeige, die
  Orphan-Freigabekarte, das Switcher-Badge und die Drag-Drosselung lohnt
  sich beim nächsten Start.

## 0. Empfohlene Reihenfolge

| # | Was | Warum zuerst | Abschnitt |
|---|-----|--------------|-----------|
| 1 | Neue Chats sofort in der Seitenleiste | vom User gemeldet | 1a |
| 2 | Kontextgröße nach Modellwechsel | vom User gemeldet | 1b |
| 3 | API-Chat: Antwort landet im falschen Chat | Datenverlust/-vermischung auf Platte | 2.1 |
| 4 | Freigabe-Anfragen für unbekannte Sessions | Vibe hängt für immer | 2.2 |
| 5 | Probe-Race beim Start, verlorene Nachricht bei Fehlern | Nachricht „verschwindet“ | 2.3, 2.4 |
| 6 | Markdown-Tabellen | Vibe nutzt sie ständig | 3.1 |
| 7 | UI-Zustand pro Chat/Session trennen | Werte springen zwischen Chats | 2.5 |
| 8 | Atomare 0600-Schreibvorgänge | Absturz → kaputte Dateien | 2.6 |
| 9 | Rest aus 2 und 3 | | |

Jeder Fix bekommt einen Test, im Stil der vorhandenen Harness-Tests mit
`setup()` und einem `UnixStream`-Paar als Fake-Agent (`harness/mod.rs`, Tests
ab ~Z. 1060).

## 1. Vom User gemeldet, Ursache gefunden

### 1a. Neue Chats/Sessions erscheinen nicht sofort in der Seitenleiste

- Die Liste kommt nur aus `Harness::listed[dir]`, das mit `session/list`
  befüllt wird (`harness/ui.rs` `chat_sidebar_body` ~Z. 1575; für Code
  analog in der Projektliste).
- Neu geladen wird sie nur in `SessionCreated` (`harness/mod.rs` ~Z. 350).
  Das ist direkt nach `session/new`, also **vor** der ersten Nachricht.
- Vibe listet keine Sessions ohne Nachrichten. Darum fehlt die neue Session,
  bis die App neu startet oder ein anderes Ereignis die Liste neu lädt. Das
  betrifft den Chat- und den Code-Modus.
- Auch der Titel aus `session_info_update` landet nur in `SessionView.title`
  (`acp/session.rs` ~Z. 262). Die Seitenleiste zeigt aber die Titel aus
  `listed`, also bleibt dort „Neuer Chat“ stehen.
- Fix:
  - in `AcpEvent::PromptFinished` `list_sessions(Some(&view.cwd))` aufrufen,
    mindestens nach dem ersten Turn einer Session;
  - die aktive Session oben einblenden, solange sie nicht in `listed` steht
    (Titel aus `SessionView.title`, sonst aus der ersten Nachricht);
  - einen Titel aus `SessionView.title` dem aus `listed` vorziehen.
- Test: SessionCreated → kein Eintrag in `listed`; PromptFinished →
  `session/list` wird gesendet.

### 1b. Beim Modellwechsel passt die Kontextgröße nicht mehr

- Die Gesamtgröße im Kontext-Panel stammt nur aus `usage_update.size`
  (`SessionView::usage`, `acp/session.rs` ~Z. 271). Diese Meldung kommt nur
  während eines Turns.
- Nach `set_config_option model` (`ConfigOptionsChanged`, `harness/mod.rs`
  ~Z. 404) bleibt die alte Größe stehen, bis der nächste Turn antwortet.
  Beispiel: ML4 hat 800k, DeepSeek V4.1 Flash hat 1M.
- **User-Entscheidung: keine Schätzungen, der Wert muss stimmen.**
  - Quelle: Vibes eigene Angabe. Das ist `max_context_length` des Modells in
    `~/.vibe/config.toml`, Abgleich über `alias`. DeepSeek steht dort mit
    1048576; ML4 hat keinen Eintrag und nutzt Vibes eingebauten Standard
    (800k, mit Vibe abgeglichen).
  - Ist kein exakter Wert bekannt: „–“ zeigen, bis das nächste
    `usage_update` kommt.
  - Die Tabelle in `backend.rs` nur verwenden, wo sie Vibes echten Wert
    wiedergibt.
- Es gibt kein `toml`-Crate. Minimal zeilenweise parsen (nur lesen, Größe
  begrenzen, `[[models]]`-Abschnitte mit `alias` / `max_context_length`).
  Neue Crates nur nach Freigabe.
- Vorher per Mitschnitt prüfen, ob `vibe-acp` nach `set_config_option` selbst
  ein `usage_update` schickt. Wenn ja, reicht es, dieses anzuwenden.

## 2. Bei der Bugsuche gefunden

Alles im Code nachvollzogen. „Geprüft“ heißt, per Test oder Lauf bestätigt;
„vermutet“ ist gekennzeichnet.

### 2.1 API-Chat: laufende Antwort landet im falschen Chat (hoch)

- `open_chat`, `new_chat` und das Löschen (`app.rs` ~Z. 256, 238, 726)
  prüfen nicht, ob gerade eine Antwort läuft (`busy`).
- `poll_backend` (~Z. 338) hängt jedes Delta an die letzte Nachricht von
  `self.current` an, also an den gerade **offenen** Chat, nicht an den Chat, zu
  dem der Lauf gehört.
- **Folgen beim Wechsel während des Streamings:**
  - Der Rest der Antwort wird an die letzte Antwort des anderen Chats
    angehängt, und `persist_current` speichert das auf Platte.
  - Der ursprüngliche Chat behält eine leere oder abgebrochene Antwort.
  - Bei „+ Neuer Chat“ geht der Rest der Antwort verloren.
- `do_send` berechnet `chat_id` und verwirft es sofort (`let _ = chat_id;`
  ~Z. 301, dasselbe Muster bei ~Z. 486). Vermutlich war genau diese
  Zuordnung geplant.
- Fix: die Chat-ID zusammen mit `rx`/`run` speichern und Deltas nur in diesen
  Chat schreiben. Ist er nicht offen, laden, anhängen und speichern. Oder das
  Wechseln während einer Antwort sperren (Hinweis anzeigen). Mit Test.

### 2.2 Freigabe-Anfragen für unbekannte Sessions werden verschluckt (hoch)

- `AcpEvent::PermissionRequested` (`harness/mod.rs` ~Z. 369) legt die
  Anfrage nur ab, wenn die Session in `sessions` steht. Sonst passiert
  **nichts**: keine Antwort an Vibe. Der Turn hängt dann für immer.
- Wann das passieren kann:
  - Freigaben von Subagenten mit `child-…`-Session-ID. Das ist vermutet:
    Der Mitschnitt enthält keine Freigabe eines Kinds, `docs/acp.md`
    Abschnitt Subagents.
  - Rennen nach einer Neuverbindung oder beim Laden.
- Fix: Anfragen für unbekannte Sessions in der aktiven Session als Karte
  zeigen (mit Hinweis, woher sie kommen) oder mit „cancelled“ beantworten,
  dazu ein Status. Nie still verwerfen.
- Verwandt: Eine Freigabe in der **anderen** Ansicht (geparkte Session) ist
  unsichtbar, bis man umschaltet. Ein Hinweis am Code/Chat-Umschalter
  fehlt, z. B. „Code wartet auf Freigabe“.

### 2.3 Start-Rennen mit der Probe-Session (mittel)

- Ablauf: Der User schickt im Chat-Modus in der ersten Sekunde nach dem Start
  eine Nachricht.
  1. Die Antwort auf das `session/new` der Probe kommt zuerst. Wegen
     `user_waits` wird sie als echte Session genommen, und die Nachricht geht
     dorthin.
  2. Dabei wird `probe_pending = None` gesetzt (`harness/mod.rs` ~Z. 321).
  3. Danach kommt die Antwort auf das eigene `session/new` des Users. Sie
     wird über `SessionCreated` ~Z. 329 **aktiv**: eine leere Session.
- Folge: Es sieht so aus, als sei die Nachricht verschwunden.
- Fix: Im „consumed“-Zweig die Markierung behalten, damit die später
  ankommende Session als Probe geschlossen wird. Mit Test: zwei
  `SessionCreated` für denselben Ordner, die Nachricht bleibt in der
  sichtbaren Session.

### 2.4 Getippte Nachricht geht bei Fehlern verloren (mittel)

- `send()` leert den Entwurf sofort.
- Schlägt `session/new` fehl (`RequestFailed` → `NewSession`, ~Z. 427,
  z. B. nicht angemeldet), wird `queued = None` gesetzt und der Text ist weg.
- Dasselbe passiert, wenn das `session/new` der **Probe** fehlschlägt: Es
  löscht dann einen wartenden Prompt des Users. `RequestKind::NewSession`
  trägt `cwd`, die Probe ist aber nicht unterscheidbar.
- Auch bei einem fehlgeschlagenen `session/prompt` verschwindet der Text.
  Vibe sendet das Echo (`user_message_chunk`) nur bei Erfolg.
- Fix: den Text zurück ins Eingabefeld, wie es `connection_lost` schon macht.
  Die Probe-Anfrage eindeutig markieren, z. B. über die Request-ID.

### 2.5 UI-Zustand ist nicht pro Chat/Session getrennt (mittel)

- `field_key = "{msg_idx}#{block}#{name}"` (`render.rs` ~Z. 77) und
  `script_key` (~Z. 242) enthalten keine Chat- oder Session-ID.
- `form_values`, `scripts`, `open_groups` und `diff_open`
  (`harness/ui.rs`, Schlüssel = Item-Index) werden beim Wechsel nie geleert.
- Folge: Ein Regler oder ein aufgeklappter Tool-Block in Chat A erscheint in
  Chat B an derselben Nachrichtenposition. Ein Formular kann die Werte des
  anderen Chats abschicken.
- Fix: Schlüssel mit der Chat-/Session-ID beginnen lassen. Beim Löschen eines
  Chats die zugehörigen Einträge entfernen. Das löst nebenbei, dass die
  `scripts`-Map ohne Grenze wächst: Für jeden ```ui-js-Block bleibt ein
  boa-Kontext bis zum Programmende.

### 2.6 Schreiben der privaten Dateien ist nicht atomar (mittel)

- `write_private` (`store.rs` ~Z. 22) kürzt die Datei und schreibt an Ort und
  Stelle. Ein Absturz oder Stromausfall dabei hinterlässt einen kaputten Chat
  oder ein kaputtes `providers.json`.
- `.mode(0o600)` wirkt nur beim **Anlegen**. Eine vorhandene Datei mit
  offeneren Rechten bleibt offen.
- Fix für beides: Temp-Datei mit 0600 im selben Ordner schreiben, `sync_all`,
  dann `rename`.

### 2.7 JS-Sandbox: kein Speicherlimit (mittel, bekannt)

- Geprüft per Test:
  - 22 Verdopplungen `x = x + x` laufen ohne Fehler (4M Zeichen). Gut 30
    davon würden mehrere GB belegen. Das Schleifenlimit greift nicht, weil
    es nur ~30 Durchläufe sind.
  - Verschachtelte Schleifen **werden** vom Limit gestoppt.
- Die Doku in `script.rs` sagt selbst „kein hartes Speicherlimit“.
- Risiko: Ein fehlerhaftes oder eingeschleustes Skript kann die App per OOM
  abstürzen lassen. Prompt-Injection ist z. B. über Webinhalte möglich, die
  Vibe gelesen hat.
- Außerdem laufen Skripte im UI-Thread: Beim Ziehen eines Reglers wird jedes
  Frame neu gerendert.
- Ideen, mit dem User abstimmen:
  - Render in einem Worker-Thread mit Zeitlimit;
  - Re-Render beim Ziehen bündeln;
  - echtes Speicherlimit nur über einen Kindprozess mit `RLIMIT_AS`, das ist
    aufwendiger.

### 2.8 Kleinere Punkte

1. **`SessionCreated` setzt immer `active`** (~Z. 329), auch wenn der User
   inzwischen die Ansicht gewechselt hat. Fix: nur aktivieren, wenn `cwd` zur
   aktuellen Ansicht gehört, sonst `parked_active`.
2. **Fehlgeschlagenes `session/load`** (gelöschte Session, Kind-ID) hinterlässt
   eine leere, aktive Ansicht, nur mit einer Statuszeile. Fix: Ansicht
   entfernen, zurück zum Startbildschirm.
3. **`handle_ui_events` in der Harness** (~Z. 695) schickt pro Ereignis einen
   eigenen `session/prompt`. Zwei Ereignisse im selben Frame starten zwei
   parallele Prompts auf derselben Session. Fix: zu einem Prompt zusammenfassen.
4. **`remove_project`** (~Z. 894) lässt offene Sessions und `active` dieses
   Projekts stehen.
5. **API-Chat löschen (×)** geht ohne Rückfrage und lässt sich nicht
   rückgängig machen. Der User wollte Bestätigungen beim Löschen, siehe
   Projekt-Löschen.
6. **API-Chat Stopp:** Die Warteschlange bleibt erhalten. Die wartenden
   Nachrichten gehen nach der nächsten erfolgreichen Antwort raus, das ist
   überraschend.
7. **Neuverbindung:**
   - offene Freigabe-Karten verschwinden, weil die Anfrage tot ist. Das ist
     richtig, wird im Status aber nicht erwähnt;
   - Ansichtswechsel vor dem Neuladen: `active` zeigt auf eine noch nicht
     geladene Session. Vermutet harmlos, weil `get()` verwendet wird.
8. **Probe-Session:** Sie verlässt sich darauf, dass Vibe für Sessions ohne
   Nachricht nichts speichert (mit Vibe 3.1.0 geprüft). Nach Vibe-Updates
   erneut prüfen (`~/.vibe/logs/session/unified/`).
9. **Modellwahl ändert Vibes globales `active_model`.** So verhält sich Vibe,
   aber auch die CLI nutzt danach dieses Modell. Für den User sichtbar machen
   (Tooltip).
10. Kleinkram:
    - toter Code `let _ = chat_id;` (`app.rs` ~Z. 301, ~486);
    - `--mode api` fehlt in der CLI (`main.rs`);
    - veralteter Kommentar in `thinking_meter` („highest offered one“), der
      Standard kommt jetzt aus der Probe;
    - `harness/ui.rs` ~Z. 265 `expect(...)` besser als `let … else return`;
    - `api_providers` wird jedes Frame neu gebaut (`app.rs` `update`).

Geprüft und **in Ordnung**:
- Ein entfernter Provider löscht auch seine Key-Datei.
- Leere oder fehlerhafte Antworten gehen nicht zurück an den Provider
  (`backend.rs` ~Z. 232).
- Das Passwortfeld für Keys ist maskiert.
- Unbekannte Agent-Requests (`fs/*`, `terminal/*`) beantwortet der Client mit
  `Method not found`.

## 3. Live-Test catwright-ui-Skill (2026-11-07, vom User freigegeben)

Aufbau: eigenes `VIBE_HOME` im Scratchpad (Kopie der Config plus Skill). Das
echte `~/.vibe` blieb unberührt, der Skill ist dort **nicht** installiert.
Frage: „Kredit 20.000 €, 5 %, Laufzeit 2–8 Jahre abwägen, Monatsraten?“ (ML4).

- **Mit dem App-Hinweis** (`UI_HINT`, geht mit der ersten Nachricht einer
  neuen Session mit): Vibe antwortet mit einem ```ui-Block. Darin eine Card
  mit `slider` und drei `computed` (Formel mit `^` und negativem Exponenten),
  dazu eine Tabelle. Die App rendert das korrekt, der Regler rechnet live, die
  Werte stimmen (5 Jahre = 377,42 €).
- **Ohne Hinweis:** nur Text mit einer Markdown-Pipe-Tabelle. Der Hinweis ist
  also nötig. Die App sendet ihn nur, wenn der Skill installiert ist.

Dabei gefunden:
1. **Markdown-Tabellen (`| a | b |`) werden nicht unterstützt.** `markdown.rs`
   kennt keine Tabellen, sie erscheinen als rohe Pipes. Fix: Tabellenblock
   parsen (Kopfzeile, `|---|`-Trenner, Zeilen, Inline-Markdown in Zellen) und
   wie `UiNode::Table` rendern.
2. Ein Slider mit `step: 1` zeigt „5.00“ statt „5“. Fix: Nachkommastellen aus
   `step` ableiten.
3. `computed` formatiert ohne Tausenderpunkt („22645,48 €“). Fix: deutsches
   Format mit Tausendertrenner.
4. Der User muss den Skill einmal unter Einstellungen → Vibe installieren,
   sonst gibt es keine Widgets.

## 4. Was im Vergleich zu Claude Desktop noch fehlt (Ideen, nicht beauftragt)

- **Routinen / geplante Aufgaben: blockiert (2026-11-08 geprüft)** — ACP
  bietet dafür nichts: `docs/acp.md` kennt nur `available_commands_update`
  (Slash-Commands) und keine geplanten Loops; Vibes `scheduled-loops.json`
  ist per ACP nicht erreichbar. Erst wieder prüfen, wenn Vibe es über ACP
  anbietet. Kein eigener Scheduler.
- **Artefakte-Ansicht: erledigt (2026-11-08, siehe oben)** — als Eintrag in der
  linken Seitenleiste mit Vorschau im Hauptbereich (Referenz-Anordnung),
  nicht als rechtes Panel. Offen geblieben: JPEG/weitere Bildformate
  (bräuchte einen `image`-Feature-Flag), Vorschau-Karten/Thumbnails wie in
  `reference/picture-4.png`, und die Outputs/Diffs-Liste rechts könnte
  künftig auf die Artefakte-Ansicht verweisen.
- **Fork / Worktree pro Session:** Fork ist umgesetzt (2026-11-08, siehe
  oben; Live-Verifikation gegen `vibe-acp` steht noch aus). Worktrees pro
  Session und Gruppierung nach Worktree: bewusst verschoben.
- **Bilder anhängen: teilweise erledigt (2026-11-08)** — PNG-Anhänge gehen
  jetzt als echte `image`-Blöcke raus (Base64 ohne neues Crate, Cap 5 MiB,
  `prompt_blocks`/`read_png`; größere/fehlende Dateien bleiben
  `resource_link`). Offen: Zwischenablage/Drag & Drop (GUI, am PC testen),
  JPEG (bräuchte `image`-Feature-Flag), Prüfen welche Vibe-Modelle Bilder
  annehmen (`promptCapabilities.image` steht in `initialize`).
- **@-Erwähnung von Dateien: besteht bereits** — der Composer-Anhang
  (Pfad-Eingabe) sendet `resource_link`-Blöcke (`prompt_blocks`). Ein
  Auto-Vervollständigen mit `@` wäre Komfort, kein Protokoll-Feature.
- **MCP-Server- und Skill-Verwaltung** in den Einstellungen. Vibe hält sie in
  `~/.vibe/config.toml` und `~/.vibe/skills/`. Änderungen außerhalb des
  Projekts nur nach Rückfrage.
- **Sessions umbenennen/löschen:** ACP hat dafür nichts. Löschen hieße, in
  Vibes Dateien zu schreiben, also vorher mit dem User klären.
- **Suche über Vibe-Chats: erledigt (2026-11-08)** — „Suchen“-Zeile in der
  linken Seitenleiste (beide Modi, wie `reference/picture-3.png`) mit
  Filterfeld: Chats nach Titel, Projekte nach Name oder Sitzungstitel
  (`matches_query`/`filter_entries`, groß-/kleinschreibungsunabhängig).
- **Benachrichtigung, wenn ein langer Turn fertig ist:** ohne neues Crate nur
  über `notify-send` (Prozessaufruf), Freigabe nötig.
- **Pixel-Katze an weiteren Stellen**, siehe den Backlog in `docs/design.md`.

## 5. Entscheidungen des Users (gelten weiter)

- Vibe nur über `vibe-acp`, also nur das Abo-Kontingent. HTTP-Provider sind
  optional.
- Rust only; einzige Ausnahme ist `boa_engine`. Keine WebView.
- Exakte Werte statt Schätzungen (Kontextgrößen). Lieber „–“ als eine
  Schätzung.
- Automatischer `/models`-Abruf im API-Chat bleibt (einmal pro Provider und
  Start, nur mit Key oder lokal).
- Kosten oder Änderungen außerhalb des Projekts (`~/.vibe`) vorher fragen.
- Vibes eigene Modusnamen verwenden. UI-Texte Deutsch, Code Englisch.

## 6. Arbeitsweise / Werkzeuge

- Bauen und testen: `cd app && CARGO_NET_OFFLINE=true cargo test`, dazu
  `cargo fmt --check` und `cargo clippy --all-targets -- -D warnings`.
- Screenshots mit sauberem Zustand:
  `XDG_CONFIG_HOME=<leerer Ordner> ./target/release/catwright --mode chat --frames 600 --screenshot out.png`.
  **Nicht** `--headless` verwenden: Das schickt einen Prompt an einen
  HTTP-Provider.
- Markdown oder Widgets ohne Modellaufruf testen: einen API-Chat als
  `chats/<id>.jsonl` in den isolierten Config-Ordner legen (Zeile 0
  `{"meta": …}`, dann `{"message": …}`) und mit `--chat <id>` öffnen.
- Vibe ohne Eingriff in `~/.vibe` testen: `VIBE_HOME=<Scratchpad-Ordner>` mit
  einer 0600-Kopie von `config.toml`. Die Anmeldung kommt aus dem Keyring.
- Neuverbindung testen: in der laufenden App den `vibe-acp`-Kindprozess per PID
  beenden. Nach ~1 s muss ein neuer laufen.
- Vibe-Delegation: Skill `vibe-delegate`, Profil `claude-delegate-rust`,
  `CARGO_TARGET_DIR=$HOME/.cache/catwright-vibe-target`. Läufe neigen am Ende
  zu Schleifen: Dateiaktivität beobachten und eingreifen.
