//! Typed views of Vibe's ACP extension payloads (account, trust, loops,
//! review, connectors, questions) plus the German display helpers shared by
//! the harness UI. Pure and tolerant: missing fields become `None`/empty,
//! unknown fields are ignored. Secret-bearing fields (`apiKey`) are never
//! read.

use crate::acp::protocol::PermissionKind;
use crate::i18n::{fill, tr, tr_in, trf, Lang};
use serde_json::{json, Value};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// account + identity (`_account/read`, `_identity/read`)
// ---------------------------------------------------------------------------

/// Plan and monthly budget, exactly as Vibe reports them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AccountInfo {
    pub plan: Option<String>,
    /// Share of the monthly Vibe allowance already used (0–100+).
    pub usage_percent: Option<f64>,
    /// ISO-8601 time the allowance resets.
    pub reset_at: Option<String>,
    /// Pay-as-you-go: usage past 100 % is billed instead of paused.
    pub payg: bool,
}

pub fn parse_account(v: &Value) -> AccountInfo {
    let plan = &v["plan"];
    AccountInfo {
        plan: plan["title"]
            .as_str()
            .or(plan["name"].as_str())
            .map(str::to_string),
        usage_percent: v["budget"]["usagePercentage"].as_f64(),
        reset_at: v["budget"]["resetAt"].as_str().map(str::to_string),
        payg: v["budget"]["paygEnabled"].as_bool().unwrap_or(false),
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct IdentityInfo {
    pub name: Option<String>,
    pub email: Option<String>,
    pub workspace: Option<String>,
}

pub fn parse_identity(v: &Value) -> IdentityInfo {
    let first = v["firstName"].as_str().unwrap_or_default();
    let last = v["lastName"].as_str().unwrap_or_default();
    let name = format!("{first} {last}").trim().to_string();
    IdentityInfo {
        name: (!name.is_empty()).then_some(name),
        email: v["email"].as_str().map(str::to_string),
        workspace: v["workspace"]["name"].as_str().map(str::to_string),
    }
}

// ---------------------------------------------------------------------------
// workspace trust (`_meta.workspace_trust`, `_trust/*`)
// ---------------------------------------------------------------------------

/// Trust state of a folder. Vibe loads AGENTS.md, `.vibe/` configuration,
/// skills and hooks of a project only once the folder is trusted.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrustInfo {
    pub trusted: bool,
    /// Files in the folder that wait for the decision, e.g. `AGENTS.md`.
    pub detected_files: Vec<String>,
    /// `availableDecisions`: `trust_cwd`, `trust_repo`, `decline`.
    pub decisions: Vec<String>,
}

impl TrustInfo {
    /// Vibe has something to trust here and the user has not decided yet.
    pub fn needs_decision(&self) -> bool {
        !self.trusted && self.decisions.iter().any(|d| d.starts_with("trust_"))
    }
}

/// The trust state from a `session/new` or `session/load` result's `_meta`.
pub fn trust_of_meta(meta: Option<&Value>) -> Option<TrustInfo> {
    let t = meta?.get("workspace_trust")?;
    Some(parse_trust(t["status"].as_str(), &t["details"]))
}

/// The trust state from a `_trust/decision` answer.
pub fn trust_of_answer(v: &Value) -> TrustInfo {
    parse_trust(v["trust_status"].as_str(), &v["details"])
}

fn parse_trust(status: Option<&str>, details: &Value) -> TrustInfo {
    let strings = |key: &str| -> Vec<String> {
        details[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut detected_files = strings("detectedFiles");
    for f in strings("repoDetectedFiles") {
        if !detected_files.contains(&f) {
            detected_files.push(f);
        }
    }
    TrustInfo {
        trusted: status == Some("trusted"),
        detected_files,
        decisions: strings("availableDecisions"),
    }
}

// ---------------------------------------------------------------------------
// scheduled loops (`_loops/*`)
// ---------------------------------------------------------------------------

/// One recurring prompt of a session. Vibe runs it only while the session
/// is live and idle; missed runs are not caught up.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopInfo {
    pub id: String,
    pub prompt: String,
    pub interval_seconds: Option<u64>,
    pub cron: Option<String>,
    /// Unix time (seconds) of the next run.
    pub next_fire_at: f64,
}

impl LoopInfo {
    /// The loop's name: the first line of its prompt (Vibe's convention).
    pub fn name(&self) -> &str {
        self.prompt.lines().next().unwrap_or_default().trim()
    }

    /// "every 5 min", the common cron shapes in words ("daily at 09:00",
    /// "Mondays at 09:00") or else the cron expression itself.
    pub fn schedule(&self) -> String {
        match (self.interval_seconds, &self.cron) {
            (Some(s), _) => trf("every {time}", &[("time", &human_duration(s))]),
            (None, Some(c)) => cron_words(c).unwrap_or_else(|| format!("cron {c}")),
            (None, None) => "–".to_string(),
        }
    }
}

/// The usual five-field cron expressions in words: every N minutes or hours,
/// hourly at a minute, daily, on weekdays or on one weekday at a time.
/// `None` for anything else, which is then shown as it is.
pub fn cron_words(cron: &str) -> Option<String> {
    let f: Vec<&str> = cron.split_whitespace().collect();
    let [min, hour, dom, month, dow] = f.as_slice() else {
        return None;
    };
    if *dom != "*" || *month != "*" {
        return None;
    }
    let num = |s: &str, max: u32| s.parse::<u32>().ok().filter(|n| *n <= max);
    let every = |s: &str| {
        s.strip_prefix("*/")
            .and_then(|n| n.parse::<u64>().ok())
            .filter(|n| *n > 0)
    };
    // every N minutes / hours
    if *hour == "*" && *dow == "*" {
        if let Some(n) = every(min) {
            return Some(trf("every {time}", &[("time", &human_duration(n * 60))]));
        }
        if let Some(m) = num(min, 59) {
            return Some(trf(
                "every hour at :{minute}",
                &[("minute", &format!("{m:02}"))],
            ));
        }
        return None;
    }
    if *dow == "*" {
        if let (Some(0), Some(n)) = (num(min, 59), every(hour)) {
            return Some(trf("every {time}", &[("time", &human_duration(n * 3600))]));
        }
    }
    let (m, h) = (num(min, 59)?, num(hour, 23)?);
    let time = format!("{h:02}:{m:02}");
    let days = [
        "Sundays",
        "Mondays",
        "Tuesdays",
        "Wednesdays",
        "Thursdays",
        "Fridays",
        "Saturdays",
    ];
    match *dow {
        "*" => Some(trf("daily at {time}", &[("time", &time)])),
        "1-5" => Some(trf("weekdays at {time}", &[("time", &time)])),
        d => {
            let day = days[num(d, 7)? as usize % 7];
            Some(trf(
                "{day} at {time}",
                &[("day", &tr_day(day)), ("time", &time)],
            ))
        }
    }
}

/// A weekday of `cron_words`, translated (the names are literals, so the
/// catalogs know them).
fn tr_day(day: &str) -> String {
    match day {
        "Sundays" => tr("Sundays"),
        "Mondays" => tr("Mondays"),
        "Tuesdays" => tr("Tuesdays"),
        "Wednesdays" => tr("Wednesdays"),
        "Thursdays" => tr("Thursdays"),
        "Fridays" => tr("Fridays"),
        _ => tr("Saturdays"),
    }
    .to_string()
}

fn parse_loop(v: &Value) -> Option<LoopInfo> {
    Some(LoopInfo {
        id: v["id"].as_str()?.to_string(),
        prompt: v["prompt"].as_str().unwrap_or_default().to_string(),
        interval_seconds: v["intervalSeconds"].as_u64(),
        cron: v["cron"].as_str().map(str::to_string),
        next_fire_at: v["nextFireAt"].as_f64().unwrap_or(0.0),
    })
}

pub fn parse_loops(v: &Value) -> Vec<LoopInfo> {
    v["loops"]
        .as_array()
        .map(|a| a.iter().filter_map(parse_loop).collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// review (`_review/state`)
// ---------------------------------------------------------------------------

/// One file the agent changed and the user has not reviewed yet.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewFile {
    pub path: String,
    /// `created`, `modified`, `deleted`, …
    pub status: String,
    /// Change regions still waiting for a decision.
    pub pending: usize,
}

/// The changes one turn caused: `scopes[]` of `_review/state`. The owner's
/// `turnId` is the user message id of the turn (see `ReviewScope::round`).
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewScope {
    /// The turn that caused these changes; `None` when Vibe sent no owner
    /// or no id (the changes then have no round to belong to).
    pub turn_id: Option<String>,
    /// Paths of this turn's changes, in Vibe's order.
    pub files: Vec<String>,
}

/// `_review/state` as a whole: the undecided files plus Vibe's per-turn
/// scopes. `scopes` is empty when Vibe did not send it (older builds).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReviewState {
    pub files: Vec<ReviewFile>,
    pub scopes: Vec<ReviewScope>,
}

/// Parse `_review/state`. `files` behaves exactly like `parse_review`; a
/// missing or `null` `scopes` list stays empty.
pub fn parse_review_state(v: &Value) -> ReviewState {
    ReviewState {
        files: parse_review(v),
        scopes: parse_review_scopes(v),
    }
}

/// The `scopes[]` of `_review/state`: one entry per turn that changed
/// files. Entries without a usable path list are dropped; a scope whose
/// owner carries no `turnId` is kept with `turn_id: None` so its files stay
/// visible instead of vanishing.
pub fn parse_review_scopes(v: &Value) -> Vec<ReviewScope> {
    v["scopes"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    let files: Vec<String> = s["files"]
                        .as_array()?
                        .iter()
                        .filter_map(|f| f.as_str().map(str::to_string))
                        .collect();
                    if files.is_empty() {
                        return None;
                    }
                    Some(ReviewScope {
                        turn_id: s["owner"]["turnId"].as_str().map(str::to_string),
                        files,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Files with undecided changes (decided ones leave the list).
pub fn parse_review(v: &Value) -> Vec<ReviewFile> {
    v["files"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|f| {
                    let regions = f["regions"].as_array()?;
                    let pending = regions
                        .iter()
                        .filter(|r| r["decision"].as_str() == Some("pending"))
                        .count();
                    (pending > 0).then(|| ReviewFile {
                        path: f["path"].as_str().unwrap_or_default().to_string(),
                        status: f["status"].as_str().unwrap_or_default().to_string(),
                        pending,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Badge for a review file status.
pub fn review_status_label(status: &str) -> &'static str {
    match status {
        "created" => tr("NEW"),
        "deleted" => tr("DELETED"),
        "renamed" => tr("MOVED"),
        _ => tr("CHANGED"),
    }
}

/// One file of a turn's diff (`_review/turnDiff`): the content before and
/// after the turn. `baseline` is `None` for a file the turn created.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnDiff {
    /// `created`, `modified`, `deleted`, … (Vibe's review status).
    pub status: String,
    pub baseline: Option<String>,
    /// `None` when the turn deleted the file.
    pub current: Option<String>,
}

/// Parse a `_review/turnDiff` answer; `None` when it carries neither side
/// (then there is no diff to show).
pub fn parse_turn_diff(v: &Value) -> Option<TurnDiff> {
    let baseline = v["baseline"].as_str().map(str::to_string);
    let current = v["current"].as_str().map(str::to_string);
    if baseline.is_none() && current.is_none() {
        return None;
    }
    Some(TurnDiff {
        status: v["status"].as_str().unwrap_or_default().to_string(),
        baseline,
        current,
    })
}

/// One-liner for a turn diff Vibe could not deliver.
pub fn turn_diff_note(error: Option<&str>) -> String {
    match error.map(str::trim).filter(|e| !e.is_empty()) {
        Some(e) => trf("Diff not available: {error}", &[("error", &e)]),
        None => tr("Diff not available.").to_string(),
    }
}

// ---------------------------------------------------------------------------
// connectors (`_connectors/*`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorInfo {
    pub name: String,
    pub enabled: bool,
    pub connected: bool,
    pub tools: usize,
    pub error: Option<String>,
}

pub fn parse_connectors(v: &Value) -> Vec<ConnectorInfo> {
    v["connectors"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    Some(ConnectorInfo {
                        name: c["name"].as_str()?.to_string(),
                        enabled: c["enabled"].as_bool().unwrap_or(false),
                        connected: c["reachability"].as_str() == Some("connected"),
                        tools: c["tools"].as_array().map(Vec::len).unwrap_or(0),
                        error: c["error"].as_str().map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// questions (`elicitation/create`, Vibe's ask_user_question tool)
// ---------------------------------------------------------------------------

/// One question of a form Vibe asks.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionField {
    /// Form key (`q0`, `q1`, …) the answer goes under.
    pub key: String,
    pub header: Option<String>,
    pub text: String,
    /// Offered answers (`const` values, sent back verbatim).
    pub options: Vec<String>,
    /// Offered answers left out (beyond `MAX_QUESTION_OPTIONS`, or longer
    /// than `MAX_QUESTION_TEXT`): the free-text answer still takes them.
    pub hidden_options: usize,
    pub multi: bool,
}

/// Most questions one form may ask. Vibe's `ask_user_question` sets no
/// upper bound and runs without approval, and every question is required,
/// so a larger form is declined rather than cut (an answer without some
/// questions would not be one).
pub const MAX_QUESTION_FIELDS: usize = 16;
/// Most offered answers shown per question (Vibe recommends 2–4).
pub const MAX_QUESTION_OPTIONS: usize = 32;
/// Longest question / header / note / answer text kept, in characters.
pub const MAX_QUESTION_TEXT: usize = 2000;

/// `text` cut to `max` characters with "…".
fn cut_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut t: String = text.chars().take(max).collect();
    t.push('…');
    t
}

/// What the user picked or typed for one question.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QuestionAnswer {
    pub selected: Vec<String>,
    /// Free-text answer ("Andere"); Vibe accepts values outside the options.
    pub other: String,
}

/// An open `elicitation/create` request. The turn waits until it is answered.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingQuestion {
    pub request_id: Value,
    pub session_id: Option<String>,
    pub note: Option<String>,
    pub fields: Vec<QuestionField>,
    pub answers: HashMap<String, QuestionAnswer>,
}

/// Parse an `elicitation/create` request; `None` for a shape that cannot be
/// rendered as a form (the caller then declines it).
pub fn parse_question(request_id: Value, params: &Value) -> Option<PendingQuestion> {
    let schema = &params["requestedSchema"];
    let props = schema["properties"].as_object()?;
    if props.len() > MAX_QUESTION_FIELDS {
        return None;
    }
    let mut fields = Vec::with_capacity(props.len());
    for (key, prop) in props {
        let multi = prop["type"].as_str() == Some("array");
        let list = if multi {
            prop["items"]["anyOf"].as_array()
        } else {
            prop["oneOf"].as_array()
        };
        let offered: Vec<&str> = list
            .map(|a| a.iter().filter_map(|o| o["const"].as_str()).collect())
            .unwrap_or_default();
        // an option is sent back verbatim, so a long one is left out, not cut
        let options: Vec<String> = offered
            .iter()
            .filter(|o| o.chars().count() <= MAX_QUESTION_TEXT)
            .take(MAX_QUESTION_OPTIONS)
            .map(|o| o.to_string())
            .collect();
        fields.push(QuestionField {
            key: key.clone(),
            header: prop["title"]
                .as_str()
                .map(|t| cut_chars(t, MAX_QUESTION_TEXT)),
            text: cut_chars(
                prop["description"].as_str().unwrap_or_default(),
                MAX_QUESTION_TEXT,
            ),
            hidden_options: offered.len() - options.len(),
            options,
            multi,
        });
    }
    if fields.is_empty() {
        return None;
    }
    // q0, q1, …, q10: numeric order, not lexicographic
    fields.sort_by_key(|f| {
        f.key
            .trim_start_matches(|c: char| !c.is_ascii_digit())
            .parse::<usize>()
            .unwrap_or(usize::MAX)
    });
    Some(PendingQuestion {
        request_id,
        session_id: params["sessionId"].as_str().map(str::to_string),
        note: schema["description"]
            .as_str()
            .map(|t| cut_chars(t, MAX_QUESTION_TEXT)),
        fields,
        answers: HashMap::new(),
    })
}

impl PendingQuestion {
    /// The answer form as Vibe expects it, `None` while a question is
    /// unanswered (Vibe marks every question required).
    pub fn content(&self) -> Option<Value> {
        let mut out = serde_json::Map::new();
        for f in &self.fields {
            let empty = QuestionAnswer::default();
            let a = self.answers.get(&f.key).unwrap_or(&empty);
            let other = a.other.trim();
            if f.multi {
                let mut values = a.selected.clone();
                if !other.is_empty() {
                    values.push(other.to_string());
                }
                if values.is_empty() {
                    return None;
                }
                out.insert(f.key.clone(), json!(values));
            } else if !other.is_empty() {
                out.insert(f.key.clone(), json!(other));
            } else {
                out.insert(f.key.clone(), json!(a.selected.first()?));
            }
        }
        Some(Value::Object(out))
    }
}

// ---------------------------------------------------------------------------
// display helpers (language-aware; the `_in` variants take the language
// explicitly, the plain ones use the current UI language)
// ---------------------------------------------------------------------------

/// Seconds as a short duration: "45 sec", "5 min", "2 h", "1 day" (German:
/// "45 Sek", "5 Min", "2 Std", "1 Tag").
pub fn human_duration(secs: u64) -> String {
    human_duration_in(crate::i18n::lang(), secs)
}

/// `human_duration` in `lang`.
pub fn human_duration_in(lang: Lang, secs: u64) -> String {
    let n = |text: &'static str, n: u64| fill(tr_in(lang, text), &[("n", &n)]);
    match secs {
        0..=59 => n("{n} sec", secs),
        60..=3599 => n("{n} min", secs / 60),
        3600..=86_399 => n("{n} h", secs / 3600),
        _ => {
            let d = secs / 86_400;
            if d == 1 {
                tr_in(lang, "1 day").to_string()
            } else {
                n("{n} days", d)
            }
        }
    }
}

/// Elapsed time of a running turn: "8 s", "1:05", "1:02:09".
pub fn elapsed_label(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs} s"),
        60..=3599 => format!("{}:{:02}", secs / 60, secs % 60),
        _ => format!("{}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60),
    }
}

/// Days from 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Unix seconds of an ISO-8601 timestamp such as
/// `2026-11-07T16:03:06.046000+00:00` or `2026-12-01T00:00:00Z`.
pub fn iso_to_epoch(iso: &str) -> Option<i64> {
    let (date, rest) = iso.trim().split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (year, month, day) = (d.next()??, d.next()??, d.next()??);
    // `get`, not `split_at`: the timestamp comes from the agent, and a
    // multi-byte character across byte 8 must not panic the UI thread
    let (hms, tail) = (rest.get(..8)?, rest.get(8..)?);
    let mut t = hms.split(':').map(|x| x.parse::<i64>().ok());
    let (h, min, s) = (t.next()??, t.next()??, t.next()??);
    // plausible ranges only, so the arithmetic below cannot overflow
    let in_range = (0..=9999).contains(&year)
        && (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && (0..=23).contains(&h)
        && (0..=59).contains(&min)
        && (0..=60).contains(&s);
    if !in_range {
        return None;
    }
    // skip fractional seconds, then read the zone offset
    let zone = tail.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = if let Some(z) = zone.strip_prefix(['+', '-']) {
        let mut z = z.split(':');
        let zh: i64 = z.next()?.parse().ok()?;
        let zm: i64 = z.next().unwrap_or("0").parse().ok()?;
        if !(0..=23).contains(&zh) || !(0..=59).contains(&zm) {
            return None;
        }
        let total = zh * 3600 + zm * 60;
        if zone.starts_with('-') {
            -total
        } else {
            total
        }
    } else {
        0
    };
    Some(days_from_civil(year, month, day) * 86_400 + h * 3600 + min * 60 + s - offset)
}

/// Short age of an ISO-8601 timestamp relative to `now` (Unix seconds):
/// "now", "5 min", "3 h", "yesterday", "4 d", "Oct 12" (German: "gerade",
/// "5 Min", "3 Std", "gestern", "4 T", "12.10."; fr/es/it/pt: "12/10").
pub fn relative_time(iso: &str, now: i64) -> Option<String> {
    relative_time_in(crate::i18n::lang(), iso, now)
}

/// `relative_time` in `lang`.
pub fn relative_time_in(lang: Lang, iso: &str, now: i64) -> Option<String> {
    let t = iso_to_epoch(iso)?;
    let age = (now - t).max(0);
    let n = |text: &'static str, n: i64| fill(tr_in(lang, text), &[("n", &n)]);
    Some(match age {
        0..=59 => tr_in(lang, "now").to_string(),
        60..=3599 => n("{n} min", age / 60),
        3600..=86_399 => n("{n} h", age / 3600),
        86_400..=172_799 => tr_in(lang, "yesterday").to_string(),
        172_800..=2_591_999 => n("{n} d", age / 86_400),
        _ => {
            let (_, m, d) = civil_from_days(t.div_euclid(86_400));
            match lang {
                Lang::De => format!("{d:02}.{m:02}."),
                Lang::En => format!("{} {d}", month_abbr(m)),
                Lang::Fr | Lang::Es | Lang::It | Lang::Pt => format!("{d:02}/{m:02}"),
            }
        }
    })
}

/// Date of an ISO-8601 timestamp in the current language: "Dec 1, 2026"
/// (German: "01.12.2026", fr/es/it/pt: "01/12/2026").
pub fn local_date(iso: &str) -> Option<String> {
    local_date_in(crate::i18n::lang(), iso)
}

/// `local_date` in `lang`.
pub fn local_date_in(lang: Lang, iso: &str) -> Option<String> {
    let t = iso_to_epoch(iso)?;
    let (y, m, d) = civil_from_days(t.div_euclid(86_400));
    Some(match lang {
        Lang::De => format!("{d:02}.{m:02}.{y}"),
        Lang::En => format!("{} {d}, {y}", month_abbr(m)),
        Lang::Fr | Lang::Es | Lang::It | Lang::Pt => format!("{d:02}/{m:02}/{y}"),
    })
}

/// English month abbreviation of month `m` (1–12).
fn month_abbr(m: i64) -> &'static str {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    MONTHS[((m - 1).clamp(0, 11)) as usize]
}

/// Inverse of `days_from_civil`.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Current Unix time in seconds.
pub fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Button label for a permission option; Vibe's own name when the id is
/// not one of its standard options. The path-scoped options name their
/// target in a fixed English phrase ("this file only", "these folders",
/// vibe/utils/tool_presentation.py); "these" in it means several targets.
pub fn permission_label(option_id: &str, name: &str) -> String {
    let several = name.contains("these");
    match (option_id, several) {
        ("allow_once", _) => tr("Allow once"),
        ("allow_always", _) => tr("For this session"),
        ("allow_always_permanent", _) => tr("Always allow"),
        ("reject_once", _) => tr("Deny"),
        ("allow_session_exact", false) => tr("This file, for this session"),
        ("allow_session_exact", true) => tr("These paths, for this session"),
        ("allow_session_directory_recursive", false) => tr("This folder, for this session"),
        ("allow_session_directory_recursive", true) => tr("These folders, for this session"),
        ("allow_permanent_exact", false) => tr("Always allow this file"),
        ("allow_permanent_exact", true) => tr("Always allow these paths"),
        ("allow_permanent_directory_recursive", false) => tr("Always allow this folder"),
        ("allow_permanent_directory_recursive", true) => tr("Always allow these folders"),
        _ => name,
    }
    .to_string()
}

/// The `kind` Vibe gives each of its permission option ids
/// (`build_permission_options` in vibe/acp/utils.py).
fn expected_kind(option_id: &str) -> Option<PermissionKind> {
    Some(match option_id {
        "allow_once" => PermissionKind::AllowOnce,
        "reject_once" => PermissionKind::RejectOnce,
        "allow_always"
        | "allow_always_permanent"
        | "allow_session_exact"
        | "allow_session_directory_recursive"
        | "allow_permanent_exact"
        | "allow_permanent_directory_recursive" => PermissionKind::AllowAlways,
        _ => return None,
    })
}

/// How a permission option is shown: the label, and the kind that decides
/// its colour and number key, or `None` for an option that is not one of
/// Vibe's known id/kind pairs.
#[derive(Debug, Clone, PartialEq)]
pub struct OptionView {
    pub label: String,
    pub kind: Option<PermissionKind>,
}

/// The ONE place a permission option's label, colour and number key come
/// from (card and activity feed alike). What is sent is the option id, so
/// everything shown must follow from the id: a known id whose `kind`
/// matches it gets the app's label and its kind; any other option (an
/// unknown id, or a known id with a different kind, e.g. id
/// `allow_always_permanent` dressed as `reject_once` "Deny") is shown as
/// what it sends, neutral and without a number key (audit H1).
pub fn option_view(option_id: &str, kind: &PermissionKind, name: &str) -> OptionView {
    match expected_kind(option_id) {
        Some(expected) if &expected == kind => OptionView {
            label: permission_label(option_id, name),
            kind: Some(expected),
        },
        _ => OptionView {
            label: trf(
                "{name} (sends “{id}”)",
                &[
                    ("name", &visible_controls(name)),
                    ("id", &visible_controls(option_id)),
                ],
            ),
            kind: None,
        },
    }
}

/// Most options a permission card shows (Vibe sends at most six; the
/// request keeps no more, see `PermissionRequest::bounded`).
pub const MAX_PERMISSION_OPTIONS: usize = crate::acp::protocol::MAX_PERMISSION_OPTIONS;

/// `text` with every character that could hide or reorder what an
/// approval shows replaced by a visible `⟨U+XXXX⟩`: control characters
/// (except line feed and tab), bidirectional overrides and isolates,
/// zero-width and other invisible characters, and the Unicode line /
/// paragraph separators. Line feeds stay: the approval card shows every
/// line and says how many there are.
pub fn visible_controls(text: &str) -> std::borrow::Cow<'_, str> {
    fn hidden(c: char) -> bool {
        (c.is_control() && c != '\n' && c != '\t')
            || matches!(c,
                '\u{061C}' | '\u{200E}' | '\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
                | '\u{200B}'..='\u{200D}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2028}' | '\u{2029}'
                | '\u{00AD}' | '\u{034F}' | '\u{180E}' | '\u{FEFF}'
                | '\u{115F}' | '\u{1160}' | '\u{3164}' | '\u{FFA0}')
    }
    if !text.chars().any(hidden) {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 16);
    for c in text.chars() {
        if hidden(c) {
            out.push_str(&format!("⟨U+{:04X}⟩", c as u32));
        } else {
            out.push(c);
        }
    }
    std::borrow::Cow::Owned(out)
}

/// Vibe's standard thinking levels, weakest first (`THINKING_LEVELS` in
/// vibe/config_values.py).
const THINKING_ORDER: [&str; 6] = ["off", "low", "medium", "high", "xhigh", "max"];

/// The level of `levels` that stands in for `want`: `want` itself when it is
/// offered, else the strongest offered standard level below it, else the
/// weakest offered one. `None` when nothing fits by rank (no levels, or
/// `want` is a model's own level that this model does not offer).
pub fn fit_thinking(want: &str, levels: &[String]) -> Option<String> {
    if levels.iter().any(|l| l == want) {
        return Some(want.to_string());
    }
    let rank = |l: &str| THINKING_ORDER.iter().position(|x| *x == l);
    let want_rank = rank(want)?;
    let ranked: Vec<(usize, &String)> = levels
        .iter()
        .filter_map(|l| rank(l).map(|r| (r, l)))
        .collect();
    ranked
        .iter()
        .filter(|(r, _)| *r < want_rank)
        .max_by_key(|(r, _)| *r)
        .or_else(|| ranked.iter().min_by_key(|(r, _)| *r))
        .map(|(_, l)| (*l).clone())
}

/// Label of a thinking level.
pub fn thinking_label(level: &str) -> String {
    match level {
        "off" => tr("Off"),
        "low" => tr("Low"),
        "medium" => tr("Medium"),
        "high" => tr("High"),
        "max" => tr("Max"),
        other => other,
    }
    .to_string()
}

/// One-liner for a built-in mode; Vibe's own text for custom ones.
pub fn mode_description(id: &str, fallback: Option<&str>) -> String {
    match id {
        "ask" => tr("Asks before every action"),
        "accept-edits" => tr("File changes automatically, commands with approval"),
        "smart-approve" => tr("Safe actions automatically, risky ones with approval"),
        "auto-approve" => tr("Runs everything without asking"),
        _ => fallback.unwrap_or(""),
    }
    .to_string()
}

/// Description of Vibe's standard slash commands; Vibe's own text for
/// everything else.
pub fn command_description(name: &str, fallback: &str) -> String {
    match name {
        "compact" => tr("Summarise the conversation and free context"),
        "help" => tr("Show the available commands"),
        "log" => tr("Show the path to the session log"),
        "mcp" => tr("Show or change the MCP sign-in status"),
        "reload" => tr("Reload configuration, instructions and skills"),
        "retry" => tr("Retry a failed or cancelled answer"),
        "data-retention" => tr("Information on data retention"),
        "proxy-setup" => tr("Proxy and certificate settings"),
        "teleport" => tr("Move the session to Vibe Code Web"),
        _ => fallback,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_reads_exact_values_and_never_the_key() {
        let a = parse_account(&json!({
            "status": "ready",
            "plan": {"kind": "chat", "name": "INDIVIDUAL", "title": "Pro"},
            "apiKey": {"preview": "abc", "scope": "vibe"},
            "budget": {"usagePercentage": 14.83, "resetAt": "2026-12-01T00:00:00Z", "paygEnabled": false}
        }));
        assert_eq!(a.plan.as_deref(), Some("Pro"));
        assert_eq!(a.usage_percent, Some(14.83));
        assert_eq!(a.reset_at.as_deref(), Some("2026-12-01T00:00:00Z"));
        assert!(!a.payg);
        assert_eq!(parse_account(&json!({})), AccountInfo::default());
    }

    #[test]
    fn identity_prefers_name_keeps_email_and_workspace() {
        let i = parse_identity(&json!({
            "email": "a@b.de", "firstName": null, "lastName": null,
            "workspace": {"id": "w", "name": "Default Workspace"}
        }));
        assert_eq!(i.name, None);
        assert_eq!(i.email.as_deref(), Some("a@b.de"));
        assert_eq!(i.workspace.as_deref(), Some("Default Workspace"));
        let i = parse_identity(&json!({"firstName": "A", "lastName": "B"}));
        assert_eq!(i.name.as_deref(), Some("A B"));
    }

    #[test]
    fn trust_needs_a_decision_only_when_vibe_offers_one() {
        // real shapes from vibe-acp 3.1.0
        let meta = json!({"workspace_trust": {"status": "untrusted", "details": {
            "detectedFiles": ["AGENTS.md"], "repoDetectedFiles": [],
            "availableDecisions": ["trust_cwd", "decline"]}}});
        let t = trust_of_meta(Some(&meta)).unwrap();
        assert!(t.needs_decision());
        assert_eq!(t.detected_files, vec!["AGENTS.md"]);
        let none = json!({"workspace_trust": {"status": "untrusted", "details": null}});
        assert!(!trust_of_meta(Some(&none)).unwrap().needs_decision());
        let done = trust_of_answer(&json!({"trust_status": "trusted", "details": null}));
        assert!(done.trusted);
        assert!(!done.needs_decision());
        assert_eq!(trust_of_meta(None), None);
    }

    #[test]
    fn cron_in_words() {
        let w = |c: &str| cron_words(c);
        assert_eq!(w("0 9 * * *").as_deref(), Some("daily at 09:00"));
        assert_eq!(w("30 8 * * 1-5").as_deref(), Some("weekdays at 08:30"));
        assert_eq!(w("0 9 * * 1").as_deref(), Some("Mondays at 09:00"));
        assert_eq!(w("0 9 * * 0").as_deref(), Some("Sundays at 09:00"));
        assert_eq!(w("0 9 * * 7").as_deref(), Some("Sundays at 09:00"));
        assert_eq!(w("*/15 * * * *").as_deref(), Some("every 15 min"));
        assert_eq!(w("0 */2 * * *").as_deref(), Some("every 2 h"));
        assert_eq!(w("5 * * * *").as_deref(), Some("every hour at :05"));
        // anything else stays a cron expression
        for c in [
            "0 9 1 * *",
            "0 9 * * 1,3",
            "61 9 * * *",
            "0 9 * *",
            "@daily",
        ] {
            assert_eq!(w(c), None, "{c}");
        }
        let l = LoopInfo {
            id: String::new(),
            prompt: String::new(),
            interval_seconds: None,
            cron: Some("0 9 * * 1,3".to_string()),
            next_fire_at: 0.0,
        };
        assert_eq!(l.schedule(), "cron 0 9 * * 1,3");
    }

    #[test]
    fn loops_parse_with_name_and_schedule() {
        let l = parse_loops(&json!({"loops": [{
            "id": "x", "prompt": "Build prüfen\nBitte den Build prüfen.",
            "intervalSeconds": 300, "nextFireAt": 17.5
        }]}));
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].name(), "Build prüfen");
        assert_eq!(l[0].schedule(), "every 5 min");
        assert_eq!(tr_in(Lang::De, "every {time}"), "alle {time}");
        assert_eq!(l[0].next_fire_at, 17.5);
        assert!(parse_loops(&json!({})).is_empty());
    }

    #[test]
    fn review_lists_only_undecided_files() {
        let files = parse_review(&json!({"files": [
            {"path": "/a", "status": "created", "regions": [{"decision": "pending"}]},
            {"path": "/b", "status": "modified", "regions": [{"decision": "keep"}]}
        ]}));
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "/a");
        assert_eq!(files[0].pending, 1);
        assert_eq!(review_status_label("created"), "NEW");
        assert_eq!(review_status_label("modified"), "CHANGED");
        assert_eq!(tr_in(Lang::De, "NEW"), "NEU");
        assert_eq!(tr_in(Lang::De, "CHANGED"), "GEÄNDERT");
    }

    #[test]
    fn review_scopes_carry_the_turn_and_its_files() {
        let state = parse_review_state(&json!({
            "files": [
                {"path": "/a", "status": "modified", "regions": [{"decision": "pending"}]}
            ],
            "scopes": [
                {"owner": {"turnId": "m1"}, "files": ["/a", "/b"]},
                {"owner": {"turnId": "m2"}, "files": ["/c"]}
            ]
        }));
        assert_eq!(state.files.len(), 1, "files parsing is unchanged");
        assert_eq!(state.files[0].path, "/a");
        assert_eq!(state.scopes.len(), 2);
        assert_eq!(state.scopes[0].turn_id.as_deref(), Some("m1"));
        assert_eq!(state.scopes[0].files, vec!["/a", "/b"]);
        assert_eq!(state.scopes[1].turn_id.as_deref(), Some("m2"));
        assert_eq!(state.scopes[1].files, vec!["/c"]);
    }

    #[test]
    fn review_without_scopes_keeps_the_old_behaviour() {
        let files = json!({"files": [
            {"path": "/a", "status": "created", "regions": [{"decision": "pending"}]}
        ]});
        for v in [
            files.clone(),
            json!({"files": files["files"], "scopes": null}),
        ] {
            let state = parse_review_state(&v);
            assert_eq!(state.files, parse_review(&v));
            assert_eq!(state.files.len(), 1);
            assert!(state.scopes.is_empty());
        }
        assert_eq!(parse_review_state(&json!({})), ReviewState::default());
        assert!(parse_review_scopes(&json!({"scopes": "kaputt"})).is_empty());
    }

    #[test]
    fn mixed_review_payloads_keep_every_usable_scope() {
        let scopes = parse_review_scopes(&json!({"scopes": [
            {"owner": {"turnId": "m1"}, "files": ["/a"]},
            // no owner at all: the files stay, without a round
            {"files": ["/b"]},
            // an owner without a turn id: same
            {"owner": {"kind": "user"}, "files": ["/c"]},
            // nothing to show: dropped, not shown as an empty round
            {"owner": {"turnId": "m2"}, "files": []},
            {"owner": {"turnId": "m3"}},
            {"owner": {"turnId": "m4"}, "files": ["/d", 7, null, "/e"]},
            "kaputt"
        ]}));
        assert_eq!(scopes.len(), 4);
        assert_eq!(scopes[0].turn_id.as_deref(), Some("m1"));
        assert_eq!(
            scopes[1],
            ReviewScope {
                turn_id: None,
                files: vec!["/b".into()]
            }
        );
        assert_eq!(
            scopes[2],
            ReviewScope {
                turn_id: None,
                files: vec!["/c".into()]
            }
        );
        assert_eq!(scopes[3].turn_id.as_deref(), Some("m4"));
        assert_eq!(scopes[3].files, vec!["/d", "/e"]);
    }

    #[test]
    fn turn_diff_parses_both_sides_and_says_when_there_is_none() {
        let d = parse_turn_diff(&json!({
            "status": "modified", "baseline": "alt\n", "current": "neu\n"
        }))
        .unwrap();
        assert_eq!(d.status, "modified");
        assert_eq!(d.baseline.as_deref(), Some("alt\n"));
        assert_eq!(d.current.as_deref(), Some("neu\n"));
        // a created file has no baseline, a deleted one no current
        let created =
            parse_turn_diff(&json!({"status": "created", "baseline": null, "current": "x"}));
        assert_eq!(created.unwrap().baseline, None);
        let deleted =
            parse_turn_diff(&json!({"status": "deleted", "baseline": "x", "current": null}));
        assert_eq!(deleted.unwrap().current, None);
        assert_eq!(parse_turn_diff(&json!({"status": "error"})), None);
        assert_eq!(parse_turn_diff(&json!({})), None);
        assert_eq!(turn_diff_note(None), "Diff not available.");
        assert_eq!(
            turn_diff_note(Some(" unknown file ")),
            "Diff not available: unknown file"
        );
        assert_eq!(
            tr_in(Lang::De, "Diff not available."),
            "Diff nicht verfügbar."
        );
    }

    #[test]
    fn connectors_parse() {
        let c = parse_connectors(&json!({"connectors": [
            {"name": "linear", "enabled": true, "reachability": "connected", "tools": [{"name": "t"}]},
            {"name": "github", "enabled": false, "reachability": "unknown", "tools": [], "error": "x"}
        ]}));
        assert_eq!(c.len(), 2);
        assert!(c[0].connected);
        assert_eq!(c[0].tools, 1);
        assert!(!c[1].enabled);
        assert_eq!(c[1].error.as_deref(), Some("x"));
    }

    /// The real `elicitation/create` shape of Vibe 3.1.0.
    fn question_params() -> Value {
        json!({
            "sessionId": "s", "toolCallId": "c", "message": "User input required", "mode": "form",
            "requestedSchema": {"required": ["q0", "q1"], "description": "Hinweis", "properties": {
                "q0": {"type": "string", "title": "Format", "description": "Welches Format?",
                       "oneOf": [{"const": "CSV", "title": "CSV"}, {"const": "JSON", "title": "JSON"}]},
                "q1": {"type": "array", "description": "Welche Spalten?",
                       "items": {"anyOf": [{"const": "x", "title": "x"}, {"const": "y", "title": "y"}]}}
            }}
        })
    }

    #[test]
    fn question_parses_single_and_multi() {
        let q = parse_question(json!(3), &question_params()).unwrap();
        assert_eq!(q.session_id.as_deref(), Some("s"));
        assert_eq!(q.note.as_deref(), Some("Hinweis"));
        assert_eq!(q.fields.len(), 2);
        assert_eq!(q.fields[0].key, "q0");
        assert_eq!(q.fields[0].options, vec!["CSV", "JSON"]);
        assert!(!q.fields[0].multi);
        assert_eq!(q.fields[1].header, None);
        assert!(q.fields[1].multi);
        assert!(
            parse_question(json!(1), &json!({"requestedSchema": {"properties": {}}})).is_none()
        );
        assert!(parse_question(json!(1), &json!({})).is_none());
    }

    #[test]
    fn question_content_needs_every_answer_and_allows_free_text() {
        let mut q = parse_question(json!(3), &question_params()).unwrap();
        assert_eq!(q.content(), None, "nothing answered yet");
        q.answers.entry("q0".into()).or_default().selected = vec!["CSV".into()];
        assert_eq!(q.content(), None, "q1 still open");
        q.answers.entry("q1".into()).or_default().selected = vec!["x".into()];
        q.answers.entry("q1".into()).or_default().other = " z ".into();
        assert_eq!(q.content().unwrap(), json!({"q0": "CSV", "q1": ["x", "z"]}));
        q.answers.entry("q0".into()).or_default().other = "XML".into();
        assert_eq!(q.content().unwrap()["q0"], json!("XML"));
    }

    #[test]
    fn durations_are_short_in_both_languages() {
        let de = |s| human_duration_in(Lang::De, s);
        assert_eq!(de(30), "30 Sek");
        assert_eq!(de(300), "5 Min");
        assert_eq!(de(7200), "2 Std");
        assert_eq!(de(86_400), "1 Tag");
        assert_eq!(de(172_800), "2 Tage");
        let en = |s| human_duration_in(Lang::En, s);
        assert_eq!(en(30), "30 sec");
        assert_eq!(en(300), "5 min");
        assert_eq!(en(7200), "2 h");
        assert_eq!(en(86_400), "1 day");
        assert_eq!(en(172_800), "2 days");
        // the plain variant follows the UI language (English in tests)
        assert_eq!(human_duration(300), "5 min");
        assert_eq!(elapsed_label(8), "8 s");
        assert_eq!(elapsed_label(65), "1:05");
        assert_eq!(elapsed_label(3729), "1:02:09");
    }

    #[test]
    fn non_ascii_or_absurd_timestamps_do_not_panic() {
        // byte 8 inside a multi-byte character (audit H3)
        assert_eq!(iso_to_epoch("2026-11-07T€€€"), None);
        assert_eq!(iso_to_epoch("2026-11-07T12:00:0€"), None);
        assert_eq!(relative_time("2026-11-07T€€€", 0), None);
        // values whose arithmetic would overflow
        assert_eq!(iso_to_epoch("9223372036854775807-01-01T00:00:00Z"), None);
        assert_eq!(iso_to_epoch("2026-11-07T00:00:00+9999999999999:00"), None);
        assert_eq!(iso_to_epoch("2026-13-07T00:00:00Z"), None);
    }

    #[test]
    fn hidden_characters_become_visible() {
        assert!(matches!(
            visible_controls("ls -la\n\tpwd"),
            std::borrow::Cow::Borrowed(_)
        ));
        assert_eq!(visible_controls("rm \u{202E}fdp.exe"), "rm ⟨U+202E⟩fdp.exe");
        assert_eq!(
            visible_controls("a\u{200B}b\rc\u{1b}[2J"),
            "a⟨U+200B⟩b⟨U+000D⟩c⟨U+001B⟩[2J"
        );
        assert_eq!(visible_controls("x\u{2028}y\u{FEFF}"), "x⟨U+2028⟩y⟨U+FEFF⟩");
        assert_eq!(visible_controls("Grüße 🙂"), "Grüße 🙂");
    }

    #[test]
    fn oversized_questions_are_declined_or_cut() {
        let field = |opts: Vec<String>| {
            json!({"type": "string", "title": "T", "description": "x".repeat(5000),
                   "oneOf": opts.iter().map(|o| json!({"const": o, "title": o})).collect::<Vec<_>>()})
        };
        // too many questions: declined (every question is required)
        let mut props = serde_json::Map::new();
        for i in 0..=MAX_QUESTION_FIELDS {
            props.insert(format!("q{i}"), field(vec!["a".into(), "b".into()]));
        }
        let params = json!({"requestedSchema": {"properties": props}});
        assert!(parse_question(json!(1), &params).is_none());
        // too many / too long options: shown up to the cap, the rest counted
        let mut opts: Vec<String> = (0..100).map(|i| format!("o{i}")).collect();
        opts.insert(0, "y".repeat(MAX_QUESTION_TEXT + 1));
        let params = json!({"requestedSchema": {"properties": {"q0": field(opts)}}});
        let q = parse_question(json!(1), &params).unwrap();
        let f = &q.fields[0];
        assert_eq!(f.options.len(), MAX_QUESTION_OPTIONS);
        assert_eq!(f.options[0], "o0", "a long option is left out, not cut");
        assert_eq!(f.hidden_options, 101 - MAX_QUESTION_OPTIONS);
        assert_eq!(f.text.chars().count(), MAX_QUESTION_TEXT + 1);
        assert!(f.text.ends_with('…'));
    }

    #[test]
    fn iso_timestamps_parse_with_and_without_zone() {
        assert_eq!(iso_to_epoch("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_to_epoch("1970-01-02T00:00:00+00:00"), Some(86_400));
        // the real Vibe format with microseconds and an offset
        assert_eq!(
            iso_to_epoch("2026-11-07T16:03:06.046000+00:00"),
            Some(1_794_067_386)
        );
        assert_eq!(iso_to_epoch("1970-01-01T01:00:00+01:00"), Some(0));
        assert_eq!(iso_to_epoch("1970-01-01T00:00:00-01:00"), Some(3600));
        assert_eq!(iso_to_epoch("nope"), None);
    }

    #[test]
    fn relative_times_are_short_labels_in_both_languages() {
        let now = iso_to_epoch("2026-11-08T12:00:00Z").unwrap();
        let rt = |s: &str| relative_time_in(Lang::De, s, now).unwrap();
        assert_eq!(rt("2026-11-08T11:59:30Z"), "gerade");
        assert_eq!(rt("2026-11-08T11:30:00Z"), "30 Min");
        assert_eq!(rt("2026-11-08T07:00:00Z"), "5 Std");
        assert_eq!(rt("2026-11-07T10:00:00Z"), "gestern");
        assert_eq!(rt("2026-11-04T10:00:00Z"), "4 T");
        assert_eq!(rt("2026-09-12T10:00:00Z"), "12.09.");
        assert_eq!(relative_time_in(Lang::De, "kaputt", now), None);
        assert_eq!(
            local_date_in(Lang::De, "2026-12-01T00:00:00Z").as_deref(),
            Some("01.12.2026")
        );
        let rt = |s: &str| relative_time_in(Lang::En, s, now).unwrap();
        assert_eq!(rt("2026-11-08T11:59:30Z"), "now");
        assert_eq!(rt("2026-11-08T11:30:00Z"), "30 min");
        assert_eq!(rt("2026-11-08T07:00:00Z"), "5 h");
        assert_eq!(rt("2026-11-07T10:00:00Z"), "yesterday");
        assert_eq!(rt("2026-11-04T10:00:00Z"), "4 d");
        assert_eq!(rt("2026-09-12T10:00:00Z"), "Sep 12");
        assert_eq!(
            local_date_in(Lang::En, "2026-12-01T00:00:00Z").as_deref(),
            Some("Dec 1, 2026")
        );
        let rt = |s: &str| relative_time_in(Lang::Fr, s, now).unwrap();
        assert_eq!(rt("2026-09-12T10:00:00Z"), "12/09");
        assert_eq!(
            local_date_in(Lang::Fr, "2026-12-01T00:00:00Z").as_deref(),
            Some("01/12/2026")
        );
        assert_eq!(relative_time("kaputt", now), None);
    }

    #[test]
    fn option_views_follow_the_id_that_is_sent() {
        let v = option_view("allow_once", &PermissionKind::AllowOnce, "Allow once");
        assert_eq!(
            (v.label.as_str(), v.kind),
            ("Allow once", Some(PermissionKind::AllowOnce))
        );
        let v = option_view(
            "allow_session_directory_recursive",
            &PermissionKind::AllowAlways,
            "Allow these folders for this session",
        );
        assert_eq!(v.label, "These folders, for this session");
        assert_eq!(v.kind, Some(PermissionKind::AllowAlways));
        // audit H1: a lasting grant dressed as "Deny" stays what it sends
        let v = option_view(
            "allow_always_permanent",
            &PermissionKind::RejectOnce,
            "Deny",
        );
        assert_eq!(v.kind, None, "no colour, no number key");
        assert_eq!(v.label, "Deny (sends “allow_always_permanent”)");
        // an unknown id: neutral, and the id is visible
        let v = option_view(
            "grant_everything",
            &PermissionKind::RejectOnce,
            "Deny\u{202E}",
        );
        assert_eq!(v.kind, None);
        assert_eq!(v.label, "Deny⟨U+202E⟩ (sends “grant_everything”)");
        let v = option_view("reject_once", &PermissionKind::Other, "Deny");
        assert_eq!(v.kind, None);
    }

    #[test]
    fn permission_labels_translate_known_ids_only() {
        assert_eq!(permission_label("allow_always", "x"), "For this session");
        assert_eq!(permission_label("reject_once", "x"), "Deny");
        assert_eq!(permission_label("custom", "Vibe Text"), "Vibe Text");
        assert_eq!(tr_in(Lang::De, "Allow once"), "Einmal erlauben");
        assert_eq!(tr_in(Lang::De, "Deny"), "Ablehnen");
    }

    #[test]
    fn labels_cover_levels_modes_and_commands() {
        assert_eq!(thinking_label("high"), "High");
        assert_eq!(tr_in(Lang::De, "High"), "Hoch");
        assert_eq!(thinking_label("exotic"), "exotic");
        let lmh: Vec<String> = ["low", "medium", "high"].map(String::from).to_vec();
        assert_eq!(fit_thinking("max", &lmh).as_deref(), Some("high"));
        assert_eq!(fit_thinking("medium", &lmh).as_deref(), Some("medium"));
        assert_eq!(fit_thinking("off", &lmh).as_deref(), Some("low"));
        assert_eq!(fit_thinking("exotic", &lmh), None);
        assert_eq!(fit_thinking("high", &[]), None);
        assert!(mode_description("auto-approve", None).contains("without asking"));
        // Vibe's `ask` may do everything after asking: it is not read-only
        assert_eq!(mode_description("ask", None), "Asks before every action");
        assert_eq!(
            tr_in(Lang::De, "Asks before every action"),
            "Fragt vor jeder Aktion"
        );
        assert_eq!(mode_description("own", Some("Meins")), "Meins");
        assert_eq!(mode_description("own", None), "");
        assert!(command_description("compact", "x").contains("context"));
        assert_eq!(command_description("own", "Text"), "Text");
    }
}
