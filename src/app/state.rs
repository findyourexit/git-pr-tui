use std::collections::HashMap;
use std::time::Instant;

use crate::app::workspace::{Tab, TabKind, Workspace};
use crate::data::cache::CacheKey;
use crate::data::github::{DashboardBucket, PrListFilter, PrListSort, RateLimitSnapshot};
use crate::data::models::{PrChecks, PrDetail, PrFiles, PrId, PrSummary, Repo};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Dashboard,
    PrList { repo: Repo },
    PrDetail { id: PrId, tab: DetailTab },
    Diff { id: PrId, file_index: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Conversation,
    Files,
    Checks,
    Commits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesPane {
    Tree,
    Diff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffMode {
    #[default]
    Unified,
    SideBySide,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Selection {
    Dashboard { focused: u8, rows: [usize; 3] },
    PrListRow(usize),
    DetailFile(usize),
    DiffLine { side: DiffSide, line: u32 },
    ReviewThread { thread_index: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerContext {
    ReplyToThread {
        pr_id: PrId,
        thread_id: String,
    },
    PrComment {
        pr_id: PrId,
    },
    LineComment {
        pr_id: PrId,
        head_sha: String,
        comment: crate::data::github::NewLineComment,
    },
}

#[derive(Debug, Clone)]
pub struct ComposerState {
    pub body: String,
    pub context: ComposerContext,
    pub head_moved: bool,
    /// Whether the live Markdown preview pane is shown (default true).
    pub preview_visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForcePushAlert {
    pub pr_id: PrId,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub kind: ToastKind,
    pub message: String,
    pub created_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct PendingWrite {
    pub kind: crate::data::cache::WriteKind,
    pub request: crate::app::effect::WriteRequest,
    pub version_at_submit: u64,
    pub status: PendingStatus,
}

pub const PR_LIST_FILTER_OPTIONS: &[(&str, PrListFilter)] = &[
    ("is:open", PrListFilter::Open),
    ("is:closed", PrListFilter::Closed),
    ("is:merged", PrListFilter::Merged),
    ("author:@me", PrListFilter::AuthoredByMe),
    ("draft:false", PrListFilter::NotDraft),
];

pub const PR_LIST_SORT_OPTIONS: &[(&str, PrListSort)] = &[
    ("sort:updated-desc", PrListSort::UpdatedDesc),
    ("sort:updated-asc", PrListSort::UpdatedAsc),
    ("sort:created-desc", PrListSort::CreatedDesc),
    ("sort:comments-desc", PrListSort::CommentsDesc),
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterModalState {
    pub selected: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SortModalState {
    pub selected: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewModalOption {
    Approve,
    RequestChanges,
    Comment,
}

pub const REVIEW_MODAL_OPTIONS: &[ReviewModalOption] = &[
    ReviewModalOption::Approve,
    ReviewModalOption::RequestChanges,
    ReviewModalOption::Comment,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewModalState {
    pub pr_id: PrId,
    pub selected: usize,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeModalKind {
    MethodPicker {
        methods: Vec<crate::data::models::MergeMethod>,
        selected: usize,
    },
    Blocked {
        reasons: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeModalState {
    pub pr_id: PrId,
    pub head_sha: String,
    pub kind: MergeModalKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReopenAction {
    Close,
    Reopen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseReopenConfirmState {
    pub pr_id: PrId,
    pub action: CloseReopenAction,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchState {
    pub query: String,
}

/// Per-tab UI state. Fetched data stays global in `AppState` (keyed by
/// `Repo`/`PrId`); this is purely presentation/cursor state (spec §3.1).
#[derive(Debug, Clone)]
pub struct TabState {
    pub selection: Selection,
    /// PR tab: active sub-tab.
    pub detail_tab: DetailTab,
    /// PR tab: `Some(i)` while viewing file `i`'s diff full-screen
    /// (Phase-1 single-focus; Phase 2 replaces this with a split).
    pub diff_file: Option<usize>,
    /// PR tab: per-tab diff view state (was global on `AppState`).
    pub diff_cursor: usize,
    /// PR tab: Conversation sub-tab vertical scroll offset (lines).
    pub conversation_cursor: usize,
    /// PR tab: when true, the renderer pins the focused review thread in view
    /// (set on `n`/`N`, cleared on manual scroll).
    pub conversation_scroll_to_focused: bool,
    pub diff_selection: Option<DiffSelection>,
    pub diff_whitespace_hidden: bool,
    pub diff_mode: DiffMode,
    /// Which pane is focused in the Files split (Phase 2).
    pub files_focus: FilesPane,
    /// Repo tab: list affordances (was global modal/search state).
    pub search: Option<SearchState>,
    pub filter_modal: Option<FilterModalState>,
    pub sort_modal: Option<SortModalState>,
}

impl TabState {
    #[must_use]
    pub fn for_kind(kind: &crate::app::workspace::TabKind) -> Self {
        use crate::app::workspace::TabKind;
        let selection = match kind {
            TabKind::Dashboard => Selection::Dashboard {
                focused: 0,
                rows: [0; 3],
            },
            TabKind::Repo(_) => Selection::PrListRow(0),
            TabKind::Pr(_) => Selection::DetailFile(0),
        };
        Self {
            selection,
            detail_tab: DetailTab::Conversation,
            diff_file: None,
            diff_cursor: 0,
            conversation_cursor: 0,
            conversation_scroll_to_focused: false,
            diff_selection: None,
            diff_whitespace_hidden: false,
            diff_mode: DiffMode::default(),
            files_focus: FilesPane::Tree,
            search: None,
            filter_modal: None,
            sort_modal: None,
        }
    }
}

/// Inclusive line-range selection inside the diff viewer.
///
/// `anchor` is frozen when the user presses `V`; `cursor` follows `j`/`k`.
/// Both are 0-based indices into the rendered patch line stream (the same
/// space used by `diff_cursors`). The inclusive range is
/// `[min(anchor, cursor), max(anchor, cursor)]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffSelection {
    pub anchor: usize,
    pub cursor: usize,
}

impl DiffSelection {
    #[must_use]
    pub fn new(line: usize) -> Self {
        Self {
            anchor: line,
            cursor: line,
        }
    }

    #[must_use]
    pub fn range(&self) -> (usize, usize) {
        if self.anchor <= self.cursor {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingStatus {
    Submitting,
    Validation(String),
    /// 5xx / network failure: caller should refetch to discover true server state.
    UnknownOutcome,
    /// 422 stale-sha; payload is the server-side head sha at time of rejection.
    Stale(String),
    /// 409 conflict; payload is the server message.
    Conflict(String),
}

#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct AppState {
    pub should_quit: bool,
    pub workspace: Workspace,
    pub pending_write: Option<PendingWrite>,
    pub toast: Option<Toast>,
    pub rate_limit: Option<RateLimitSnapshot>,
    pub rate_limit_warned_low: bool,
    pub rate_limit_warned_critical: bool,
    pub force_push_alert: Option<ForcePushAlert>,
    pub layout_mode: crate::ui::layout::LayoutMode,
    /// Current terminal width in columns. `0` means "unknown → treat as wide".
    pub viewport_width: u16,
    pub help_overlay: bool,
    pub palette: Option<crate::ui::palette::PaletteState>,
    pub action_menu: Option<crate::ui::action_menu::ActionMenuState>,
    pub log_view: Option<crate::ui::log_view::LogViewState>,
    pub last_refresh: HashMap<CacheKey, Instant>,
    pub composer: Option<ComposerState>,
    pub user_idle_since: Instant,
    pub dashboard: Option<Vec<(DashboardBucket, Vec<PrSummary>)>>,
    pub dashboard_cursors: HashMap<DashboardBucket, String>,
    /// Buckets with an in-flight "load more" fetch, so navigation can't fire
    /// duplicate paginated requests with the same cursor.
    pub dashboard_loading: std::collections::HashSet<DashboardBucket>,
    pub pr_lists: HashMap<Repo, Vec<PrSummary>>,
    pub pr_list_cursors: HashMap<Repo, String>,
    pub pr_list_loading: std::collections::HashSet<Repo>,
    /// Repos with an in-flight paginated "load more" fetch, so fast scrolling
    /// can't append the same next page twice.
    pub pr_list_paginating: std::collections::HashSet<Repo>,
    pub review_modal: Option<ReviewModalState>,
    pub merge_modal: Option<MergeModalState>,
    pub close_reopen_confirm: Option<CloseReopenConfirmState>,
    pub pr_details: HashMap<PrId, PrDetail>,
    pub pr_files: HashMap<PrId, PrFiles>,
    pub pr_diffs: HashMap<PrId, String>,
    pub pr_checks: HashMap<PrId, PrChecks>,
    pub pr_files_load_requested: std::collections::HashSet<PrId>,
    pub pr_file_diff_inflight: std::collections::HashSet<(PrId, String)>,
    pub repo_status: Option<RepoStatus>,
    pub checkout_inflight: Option<PrId>,
    pub keymap: crate::app::keymap::Keymap,
    /// Per-resource background-refresh intervals (seconds), from `[refresh]`.
    pub refresh: crate::config::RefreshConfig,
    /// Effective animation intensity for this session.
    pub animations: crate::app::anim::AnimationIntensity,
    /// Animation cues produced by the current event; cleared at the start of
    /// each `update` call so each event yields only its own cues.
    pub animation_cues: Vec<crate::app::anim::AnimationCue>,
    /// Whether the next first-data-load should fire a `DataLoaded` transition.
    /// Re-armed on every view-entry change (see `update`) and consumed when the
    /// cue fires, so a view whose data streams in as several async loads
    /// (detail, then checks, then files) animates exactly once per entry.
    pub data_anim_armed: bool,
}

impl AppState {
    #[must_use]
    pub fn active_tab(&self) -> &Tab {
        self.workspace.active_tab()
    }

    pub fn active_tab_mut(&mut self) -> &mut Tab {
        self.workspace.active_tab_mut()
    }

    /// Derive the legacy `View` from the active tab (rendering/handlers vocabulary).
    #[must_use]
    pub fn current_view(&self) -> View {
        let tab = self.active_tab();
        match &tab.kind {
            TabKind::Dashboard => View::Dashboard,
            TabKind::Repo(repo) => View::PrList { repo: repo.clone() },
            TabKind::Pr(id) => match tab.state.diff_file {
                Some(file_index) => View::Diff {
                    id: id.clone(),
                    file_index,
                },
                None => View::PrDetail {
                    id: id.clone(),
                    tab: tab.state.detail_tab,
                },
            },
        }
    }

    #[must_use]
    pub fn selection(&self) -> &Selection {
        &self.active_tab().state.selection
    }

    pub fn selection_mut(&mut self) -> &mut Selection {
        &mut self.active_tab_mut().state.selection
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoStatus {
    pub dirty: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            should_quit: false,
            workspace: Workspace::new(),
            pending_write: None,
            toast: None,
            rate_limit: None,
            rate_limit_warned_low: false,
            rate_limit_warned_critical: false,
            force_push_alert: None,
            layout_mode: crate::ui::layout::LayoutMode::Full,
            viewport_width: 0,
            help_overlay: false,
            palette: None,
            action_menu: None,
            log_view: None,
            last_refresh: HashMap::new(),
            composer: None,
            user_idle_since: Instant::now(),
            dashboard: None,
            dashboard_cursors: HashMap::new(),
            dashboard_loading: std::collections::HashSet::new(),
            pr_lists: HashMap::new(),
            pr_list_cursors: HashMap::new(),
            pr_list_loading: std::collections::HashSet::new(),
            pr_list_paginating: std::collections::HashSet::new(),
            review_modal: None,
            merge_modal: None,
            close_reopen_confirm: None,
            pr_details: HashMap::new(),
            pr_files: HashMap::new(),
            pr_diffs: HashMap::new(),
            pr_checks: HashMap::new(),
            pr_files_load_requested: std::collections::HashSet::new(),
            pr_file_diff_inflight: std::collections::HashSet::new(),
            repo_status: None,
            checkout_inflight: None,
            keymap: crate::app::keymap::Keymap::vim_defaults(),
            refresh: crate::config::RefreshConfig::default(),
            animations: crate::app::anim::AnimationIntensity::Full,
            animation_cues: Vec::new(),
            data_anim_armed: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_dashboard_view() {
        let s = AppState::default();
        assert!(matches!(s.current_view(), View::Dashboard));
        assert!(!s.should_quit);
    }

    #[test]
    fn default_palette_and_log_view_are_none() {
        let s = AppState::default();
        assert!(s.palette.is_none());
        assert!(s.log_view.is_none());
    }
}
