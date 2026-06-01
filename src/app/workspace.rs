//! The tabbed workspace: the source of truth for navigation. Dashboard is
//! pinned at index 0 and cannot be closed. Repos and PRs open as
//! de-duplicated, closeable tabs (browser model — spec §3).

use crate::app::state::TabState;
use crate::data::models::{PrId, Repo};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabKind {
    Dashboard,
    Repo(Repo),
    Pr(PrId),
}

#[derive(Debug)]
pub struct Tab {
    pub kind: TabKind,
    pub state: TabState,
}

impl Tab {
    fn new(kind: TabKind) -> Self {
        let state = TabState::for_kind(&kind);
        Self { kind, state }
    }
}

#[derive(Debug)]
pub struct Workspace {
    pub tabs: Vec<Tab>,
    pub active: usize,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tabs: vec![Tab::new(TabKind::Dashboard)],
            active: 0,
        }
    }

    #[must_use]
    pub fn active_tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub fn active_tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    /// Position of an existing tab with this exact kind.
    fn position(&self, kind: &TabKind) -> Option<usize> {
        self.tabs.iter().position(|t| &t.kind == kind)
    }

    /// Open `kind`: focus the existing tab if present, else push + focus.
    /// Returns `true` if a *new* tab was created.
    pub fn open(&mut self, kind: TabKind) -> bool {
        if let Some(i) = self.position(&kind) {
            self.active = i;
            false
        } else {
            self.tabs.push(Tab::new(kind));
            self.active = self.tabs.len() - 1;
            true
        }
    }

    /// Close the active tab (no-op on pinned Dashboard at index 0).
    /// Returns `true` if a tab was closed.
    pub fn close_active(&mut self) -> bool {
        if self.active == 0 {
            return false;
        }
        self.tabs.remove(self.active);
        self.active = self.active.saturating_sub(1); // left neighbour
        true
    }

    pub fn goto(&mut self, idx: usize) {
        if self.tabs.is_empty() {
            return;
        }
        self.active = idx.min(self.tabs.len() - 1);
    }

    pub fn next(&mut self) {
        self.active = (self.active + 1) % self.tabs.len();
    }

    pub fn prev(&mut self) {
        self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::models::{PrId, Repo};

    fn repo(n: &str) -> Repo {
        Repo {
            owner: "acme".into(),
            name: n.into(),
        }
    }
    fn pr(n: u64) -> PrId {
        PrId {
            repo: repo("widgets"),
            number: n,
        }
    }

    #[test]
    fn new_workspace_has_pinned_dashboard_active() {
        let ws = Workspace::new();
        assert_eq!(ws.tabs.len(), 1);
        assert!(matches!(ws.active_tab().kind, TabKind::Dashboard));
        assert_eq!(ws.active, 0);
    }

    #[test]
    fn open_repo_pushes_and_focuses() {
        let mut ws = Workspace::new();
        ws.open(TabKind::Repo(repo("widgets")));
        assert_eq!(ws.tabs.len(), 2);
        assert_eq!(ws.active, 1);
    }

    #[test]
    fn open_existing_focuses_without_duplicating() {
        let mut ws = Workspace::new();
        ws.open(TabKind::Pr(pr(421)));
        ws.open(TabKind::Repo(repo("widgets")));
        ws.open(TabKind::Pr(pr(421))); // re-open
        assert_eq!(ws.tabs.len(), 3, "no duplicate PR tab");
        assert!(matches!(ws.active_tab().kind, TabKind::Pr(ref id) if id.number == 421));
    }

    #[test]
    fn close_focuses_left_neighbour() {
        let mut ws = Workspace::new();
        ws.open(TabKind::Repo(repo("widgets"))); // idx 1
        ws.open(TabKind::Pr(pr(1))); // idx 2, active
        ws.close_active();
        assert_eq!(ws.tabs.len(), 2);
        assert_eq!(ws.active, 1, "lands on left neighbour");
    }

    #[test]
    fn close_dashboard_is_noop() {
        let mut ws = Workspace::new();
        assert!(!ws.close_active(), "pinned dashboard cannot close");
        assert_eq!(ws.tabs.len(), 1);
    }

    #[test]
    fn goto_clamps_and_dashboard_jump() {
        let mut ws = Workspace::new();
        ws.open(TabKind::Repo(repo("a")));
        ws.open(TabKind::Repo(repo("b")));
        ws.goto(0);
        assert_eq!(ws.active, 0);
        ws.goto(99); // out of range → clamp to last
        assert_eq!(ws.active, ws.tabs.len() - 1);
    }

    #[test]
    fn next_prev_wrap() {
        let mut ws = Workspace::new();
        ws.open(TabKind::Repo(repo("a")));
        ws.next(); // wraps 1 -> 0
        assert_eq!(ws.active, 0);
        ws.prev(); // wraps 0 -> 1
        assert_eq!(ws.active, 1);
    }
}
