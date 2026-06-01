#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserIntent {
    Quit,
    Refresh,
    Down,
    Up,
    PageDown,
    PageUp,
    Top,
    Bottom,
    /// No default key binding since Phase 2 (`h`/`l`/arrows now map to
    /// `FocusLeft`/`FocusRight`). Kept so existing `[keys]` config overrides
    /// referencing `left`/`right` still parse. Do not remove.
    Left,
    Right,
    /// Move pane focus within a split (e.g. Files tree ↔ diff); no-op elsewhere.
    FocusLeft,
    FocusRight,
    OpenSelected,
    SwitchTab(TabDirection),
    StartReview,
    StartComment,
    StartReplyToThread,
    StartLineComment,
    CheckoutBranch,
    Merge,
    ToggleClose,
    Submit,
    Cancel,
    EnterCommandPalette,
    EnterHelp,
    EnterRepoSwitcher,
    EnterLogView,
    ToggleSideBySide,
    ToggleWhitespace,
    SearchForward,
    NextHunk,
    PrevHunk,
    NextFile,
    PrevFile,
    LoadMore,
    OpenFilterModal,
    OpenSortModal,
    OpenSearch,
    OpenInBrowser,
    OpenActionMenu,
    NextTab,
    PrevTab,
    GotoTab(u8),
    CloseTab,
    GotoDashboard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabDirection {
    Next,
    Prev,
}
