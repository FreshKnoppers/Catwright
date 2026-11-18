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
