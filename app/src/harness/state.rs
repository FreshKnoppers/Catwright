//! Plain data of the harness views: transient UI state (open dialogs, the
//! palette, panel visibility) and prompts waiting in the queue. No logic that
//! talks to Vibe lives here.

use crate::acp::protocol::PromptBlock;
use crate::dialogs::FilePicker;
use crate::harness::extras::TurnDiff;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

/// A message typed while Vibe was still working; sent when the turn ends.
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedPrompt {
    pub text: String,
    pub blocks: Vec<PromptBlock>,
}

/// The rename dialog.
#[derive(Debug, Clone)]
pub struct RenameDialog {
    pub session_id: String,
    pub cwd: PathBuf,
    pub title: String,
}

/// One file of a turn's diff as the transcript shows it: still loading,
/// loaded, or not available (Vibe's error, kept honestly).
#[derive(Debug, Clone, PartialEq)]
pub enum TurnDiffView {
    Loading,
    Loaded(TurnDiff),
    Failed(String),
}

/// The turn diffs of one session, keyed "<turnId>::<path>". Loaded once per
/// turn and file on click (`_review/turnDiff`); `Harness::review_refresh`
/// drops them.
#[derive(Debug, Clone, Default)]
pub struct TurnDiffs {
    entries: HashMap<String, TurnDiffView>,
}

impl TurnDiffs {
    fn key(turn_id: &str, path: &str) -> String {
        format!("{turn_id}::{path}")
    }

    pub fn get(&self, turn_id: &str, path: &str) -> Option<&TurnDiffView> {
        self.entries.get(&Self::key(turn_id, path))
    }

    pub fn loading(&self, turn_id: &str, path: &str) -> bool {
        self.get(turn_id, path) == Some(&TurnDiffView::Loading)
    }

    pub fn insert(&mut self, turn_id: &str, path: &str, view: TurnDiffView) {
        self.entries.insert(Self::key(turn_id, path), view);
    }

    /// A shared empty instance: the transcript renderer borrows it when a
    /// session has no loaded diffs yet, instead of cloning the map (its
    /// entries can hold whole file baselines, M10).
    pub fn empty() -> &'static TurnDiffs {
        static EMPTY: std::sync::OnceLock<TurnDiffs> = std::sync::OnceLock::new();
        EMPTY.get_or_init(TurnDiffs::default)
    }

    /// Drop every loaded diff (a new `_review/state`: baselines may have
    /// moved).
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// The rewind confirmation, filled from `_rewind/preview`.
#[derive(Debug, Clone)]
pub struct RewindDialog {
    pub session_id: String,
    pub message_id: String,
    pub paths: Vec<String>,
    pub restore_files: bool,
}

/// A destructive action waiting for confirmation.
#[derive(Debug, Clone)]
pub enum Confirm {
    DeleteSession {
        session_id: String,
        cwd: PathBuf,
        title: String,
    },
    RevertFile {
        session_id: String,
        path: String,
    },
    RevertAll {
        session_id: String,
        files: usize,
    },
    /// Switching to `auto-approve` (Vibe then executes everything without
    /// asking) wants a deliberate confirmation.
    AutoApprove {
        session_id: Option<String>,
    },
    /// `_loops/clear`: delete every routine of the session.
    ClearRoutines {
        session_id: String,
        count: usize,
    },
    /// Previewing a path that resolves outside the session's workspace
    /// (agent-named tool/diff paths can point anywhere on disk): the read
    /// itself is capped, but the user decides whether to look at all.
    PreviewOutside {
        session_id: Option<String>,
        path: PathBuf,
        diff: bool,
        /// The pick came from the Artefakte view (select it there instead of
        /// opening the side preview pane).
        artifacts: bool,
    },
}

/// The worktree-create dialog: optional start ref (branch, tag or commit).
pub struct WorktreeDialog {
    pub start_ref: String,
}

/// The worktree-delete confirmation: the main worktree never gets here.
pub struct WorktreeDelete {
    pub project: PathBuf,
    pub path: PathBuf,
    pub branch: String,
    /// Uncommitted changes in the worktree — warned about in the dialog.
    pub dirty: bool,
    /// Also delete the branch: the user's own choice, default OFF.
    pub delete_branch: bool,
}

/// The command palette while it is open.
#[derive(Debug, Clone, Default)]
pub struct PaletteState {
    pub query: String,
    pub selected: usize,
    /// The keyboard moved the selection: bring it into view once. Doing
    /// this every frame would fight the mouse wheel.
    pub scroll_to_selected: bool,
}

/// Kind of an activity-feed entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    /// A permission or question waits for the user.
    Waiting,
    /// A turn finished.
    Finished,
    /// A turn failed.
    Failed,
}

/// One entry of the activity feed across all sessions.
#[derive(Debug, Clone)]
pub struct ActivityEvent {
    pub kind: ActivityKind,
    pub sid: String,
    pub cwd: PathBuf,
    pub title: String,
    pub text: String,
    /// Permission request id of a waiting entry, so the feed can answer it
    /// directly while the request is still open.
    pub request_id: Option<serde_json::Value>,
    /// Epoch seconds.
    pub at: i64,
}

/// One review comment on a diff line (line number on the new side of the
/// diff).
#[derive(Debug, Clone, PartialEq)]
pub struct DiffComment {
    pub line: usize,
    pub text: String,
}

/// A comment being written for one diff (`key` = the diff chip's key).
#[derive(Debug, Clone)]
pub struct CommentEdit {
    pub key: String,
    pub line: usize,
    pub text: String,
}

/// The "Neue Routine" form of a session.
#[derive(Debug, Clone)]
pub struct LoopForm {
    pub open: bool,
    pub interval: String,
    pub prompt: String,
}

impl Default for LoopForm {
    fn default() -> Self {
        Self {
            open: false,
            interval: "10m".to_string(),
            prompt: String::new(),
        }
    }
}

/// State of the composer's voice dictation. Vibe records the microphone and
/// transcribes through Mistral; Catwright only sends the three `_voice/*`
/// requests and appends the text it gets back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dictation {
    #[default]
    Idle,
    /// `_voice/transcribeStart` is in flight.
    Starting,
    /// Vibe records; `_voice/transcriptionDelta` pieces append to the draft.
    Recording,
    /// `_voice/transcribeStop` is in flight: Vibe flushes, so text may still
    /// arrive briefly after its answer.
    Stopping,
}

/// The answer a narration belongs to: its session and its place in the
/// transcript (the message id Vibe gave it, or its position when there is
/// none), so only that answer's button shows the stop state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NarrationTarget {
    pub session_id: String,
    pub message_id: Option<String>,
    pub index: usize,
}

impl NarrationTarget {
    /// Is this the answer at `index` (with `message_id`) of `session_id`?
    /// The message id wins when both sides have one: a rewind or a trim can
    /// move an answer's position, never its id.
    pub fn is(&self, session_id: &str, message_id: Option<&str>, index: usize) -> bool {
        self.session_id == session_id
            && match (self.message_id.as_deref(), message_id) {
                (Some(mine), Some(theirs)) => mine == theirs,
                _ => self.index == index,
            }
    }
}

/// State of reading answers aloud. Vibe summarises the answer, speaks the
/// summary through Mistral and plays it; Catwright only sends the two
/// `_voice/narrate*` requests and shows the button.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Narration {
    #[default]
    Idle,
    /// `_voice/narrate` went out for this answer. It lasts until Vibe reports
    /// the narration done, an error, a failed start, or a cancel.
    Narrating(NarrationTarget),
}

impl Narration {
    /// Is the answer at `index` (with `message_id`) of `session_id` the one
    /// being read aloud?
    pub fn is_answer(&self, session_id: &str, message_id: Option<&str>, index: usize) -> bool {
        match self {
            Narration::Idle => false,
            Narration::Narrating(target) => target.is(session_id, message_id, index),
        }
    }
}

/// What the composer's popup list currently offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ComposerPopup {
    pub selected: usize,
    /// Esc closed the list for the current token.
    pub dismissed: bool,
}

/// The transient toast of the status line.
#[derive(Debug, Clone, Default)]
pub struct Toast {
    pub text: String,
    pub since: Option<Instant>,
}

/// What the preview pane shows: the file on disk, or its diff against HEAD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewMode {
    #[default]
    Code,
    Diff,
}

/// The preview pane beside the transcript. While it is open it replaces the
/// right work panel.
#[derive(Debug, Clone)]
pub struct PreviewState {
    pub path: String,
    /// Panel width in points; the user can resize it.
    pub width: f32,
    pub mode: PreviewMode,
    /// The path lies outside the session's workspace; the header then shows
    /// a warning badge.
    pub outside_workspace: bool,
}

/// Transient state of the harness UI.
pub struct UiState {
    /// Persisted through the settings.
    pub sidebar_collapsed: bool,
    /// The right panel is closed. Persisted through the settings.
    pub panel_closed: bool,
    /// How open tool groups and thoughts are while a turn runs. Persisted
    /// through the settings.
    pub transcript_detail: String,
    /// Collapsible keys the user closed explicitly; an auto-open default
    /// (the detail level) never overrides them.
    pub closed_groups: HashSet<String>,
    /// The file preview pane beside the transcript (`None`: closed).
    pub preview: Option<PreviewState>,
    /// Review comments on diffs, keyed by the diff chip's key.
    pub diff_comments: HashMap<String, Vec<DiffComment>>,
    /// The comment currently being written.
    pub comment_edit: Option<CommentEdit>,
    /// The activity feed is shown in the main area.
    pub activity_open: bool,
    /// How many activity entries were already seen (the sidebar badge counts
    /// the rest).
    pub activity_seen: usize,
    /// Projects whose archived-session group is expanded in the sidebar.
    pub archived_expanded: HashSet<PathBuf>,
    /// Projects whose session lists are folded in the sidebar.
    pub collapsed_projects: HashSet<PathBuf>,
    /// Projects showing all sessions instead of the newest few.
    pub expanded_projects: HashSet<PathBuf>,
    /// Card key -> open, for the collapsible cards of the right panel.
    pub panel_sections: HashMap<&'static str, bool>,
    pub palette: Option<PaletteState>,
    pub rename: Option<RenameDialog>,
    pub rewind: Option<RewindDialog>,
    pub confirm: Option<Confirm>,
    /// The worktree-create dialog (optional start ref), if open.
    pub worktree_dialog: Option<WorktreeDialog>,
    /// The worktree-delete confirmation, if open.
    pub worktree_delete: Option<WorktreeDelete>,
    pub file_picker: Option<FilePicker>,
    pub loop_form: LoopForm,
    pub composer_popup: ComposerPopup,
    /// The composer should take the keyboard focus on the next frame.
    pub focus_composer: bool,
    /// Next frame jumps the transcript to its end.
    pub scroll_to_end: bool,
    /// One-shot jump of a session's transcript to a round top: (sid, y).
    pub scroll_to_round: Option<(String, f32)>,
    pub toast: Toast,
    /// Last window title sent to the compositor.
    pub window_title: String,
    /// The window was asked for attention for this state already.
    pub attention_sent: bool,
    /// Some session was running in the last frame.
    pub was_running: bool,
    /// The settings modal is open (set by the app shell).
    pub settings_open: bool,
    /// The composer's list took this frame's Esc.
    pub composer_used_escape: bool,
    /// Session the composer's draft belongs to (`None`: the start screen).
    pub draft_owner: Option<String>,
    /// Drafts of the other sessions: (text, attachments).
    pub parked_drafts: HashMap<Option<String>, (String, Vec<PathBuf>)>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            sidebar_collapsed: false,
            panel_closed: false,
            transcript_detail: "standard".to_string(),
            closed_groups: HashSet::new(),
            preview: None,
            diff_comments: HashMap::new(),
            comment_edit: None,
            activity_open: false,
            activity_seen: 0,
            archived_expanded: HashSet::new(),
            collapsed_projects: HashSet::new(),
            expanded_projects: HashSet::new(),
            panel_sections: HashMap::new(),
            palette: None,
            rename: None,
            rewind: None,
            confirm: None,
            worktree_dialog: None,
            worktree_delete: None,
            file_picker: None,
            loop_form: LoopForm::default(),
            composer_popup: ComposerPopup::default(),
            focus_composer: true,
            scroll_to_end: false,
            scroll_to_round: None,
            toast: Toast::default(),
            window_title: String::new(),
            attention_sent: false,
            was_running: false,
            settings_open: false,
            composer_used_escape: false,
            draft_owner: None,
            parked_drafts: HashMap::new(),
        }
    }
}
