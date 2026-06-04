pub mod actions;
pub mod anim;
pub mod effect;
pub mod event;
pub mod executor;
pub mod intent;
pub mod keymap;
pub mod render;
pub mod state;
pub mod update;
pub mod workspace;

use std::io;
use std::time::{Duration, Instant};

use crossterm::ExecutableCommand;
use crossterm::event::{Event as CtEvent, EventStream};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc::unbounded_channel;
use tokio::time::interval;

use self::anim::{
    AnimationCue, AnimationIntensity, cue_key, effect_for, frame_budget, should_reset_frame_clock,
};
use self::effect::Effect;
use self::event::AppEvent;
use self::executor::execute_effect;
use self::state::{AppState, View};
use self::update::update;
use crate::data::SharedGitHubClient;
use crate::ui::theme::Theme;

/// Compute the landing view from the cwd: `PrList { repo }` when `cwd`
/// is a git repo whose `origin` points at github.com, else `Dashboard`.
#[must_use]
pub fn initial_view(cwd: &std::path::Path) -> View {
    match crate::data::git::detect_repo(cwd) {
        Ok(repo) => View::PrList { repo },
        Err(_) => View::Dashboard,
    }
}

/// Resolve the landing view from CLI arguments layered over the cwd-derived
/// `cwd_view`. Precedence: `--dashboard` > explicit `repo`(+`--pr`) > cwd repo
/// (+`--pr`) > the cwd-derived view.
#[must_use]
pub fn cli_landing(
    cwd_view: View,
    dashboard: bool,
    repo: Option<crate::data::models::Repo>,
    pr: Option<u64>,
) -> View {
    if dashboard {
        return View::Dashboard;
    }
    // A `--pr` with no explicit repo falls back to the cwd repo, if any.
    let repo = repo.or_else(|| match &cwd_view {
        View::PrList { repo } => Some(repo.clone()),
        _ => None,
    });
    match (repo, pr) {
        (Some(repo), Some(number)) => View::PrDetail {
            id: crate::data::models::PrId { repo, number },
            tab: self::state::DetailTab::Conversation,
        },
        (Some(repo), None) => View::PrList { repo },
        (None, _) => cwd_view,
    }
}

/// Seed the workspace from a `View` (CLI/cwd landing).
///
/// Dashboard is always pinned at index 0. Additional tabs are opened as
/// needed so that `state.current_view()` returns the requested view.
pub fn seed_workspace_from_view(state: &mut AppState, view: View) {
    use self::workspace::TabKind;
    match view {
        View::Dashboard => {}
        View::PrList { repo } => {
            state.workspace.open(TabKind::Repo(repo));
        }
        View::PrDetail { id, tab } => {
            state.workspace.open(TabKind::Repo(id.repo.clone()));
            state.workspace.open(TabKind::Pr(id));
            state.active_tab_mut().state.detail_tab = tab;
        }
        View::Diff { id, file_index } => {
            state.workspace.open(TabKind::Repo(id.repo.clone()));
            state.workspace.open(TabKind::Pr(id));
            state.active_tab_mut().state.diff_file = Some(file_index);
        }
    }
}

pub struct App {
    pub state: AppState,
    pub client: SharedGitHubClient,
    /// Active color palette. Defaults to dark; overridden by config/`--no-color`.
    pub theme: Theme,
    /// tachyonfx effect manager; key is a static cue-kind string for de-dup.
    effect_manager: tachyonfx::EffectManager<&'static str>,
    /// Timestamp of the last rendered frame, used to compute dt for effects.
    last_frame: Instant,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("state", &self.state)
            .field("client", &"<dyn GitHubClient>")
            .field("theme", &self.theme)
            .field("effect_manager", &"<EffectManager>")
            .field("last_frame", &self.last_frame)
            .finish()
    }
}

impl App {
    #[must_use]
    pub fn new(client: SharedGitHubClient) -> Self {
        Self {
            state: AppState::default(),
            client,
            theme: Theme::dark(),
            effect_manager: tachyonfx::EffectManager::default(),
            last_frame: Instant::now(),
        }
    }

    /// Construct an `App` whose initial view is derived from `cwd`:
    /// `PrList { repo }` if `cwd` is a GitHub git repo, else `Dashboard`.
    #[must_use]
    pub fn with_cwd(client: SharedGitHubClient, cwd: &std::path::Path) -> Self {
        Self::with_cwd_and_config(client, cwd, None)
    }

    /// Like [`Self::with_cwd`] but applies key overrides from the config
    /// at `config_path` (or the default XDG path when `None`).
    #[must_use]
    pub fn with_cwd_and_config(
        client: SharedGitHubClient,
        cwd: &std::path::Path,
        config_path: Option<&std::path::Path>,
    ) -> Self {
        let view = initial_view(cwd);
        let config = crate::config::ConfigFile::load(config_path);
        let mut keymap = self::keymap::Keymap::vim_defaults();
        keymap.apply_overrides(&config.keys);
        // Record the cwd repo's dirtiness up front so the `C` checkout flow
        // knows whether it can safely fetch + switch branches.
        let repo_status = match crate::data::git::detect_repo(cwd) {
            Ok(_) => crate::data::git::is_worktree_dirty(cwd)
                .ok()
                .map(|dirty| self::state::RepoStatus { dirty }),
            Err(_) => None,
        };
        let theme = Theme::from_name(&config.ui.theme);
        let mut state = AppState {
            keymap,
            repo_status,
            refresh: config.refresh,
            ..AppState::default()
        };
        seed_workspace_from_view(&mut state, view);
        Self {
            state,
            client,
            theme,
            effect_manager: tachyonfx::EffectManager::default(),
            last_frame: Instant::now(),
        }
    }

    /// Run the TUI event loop until the reducer signals quit.
    ///
    /// Restores the terminal (`disable_raw_mode` + `LeaveAlternateScreen`)
    /// on every exit path. Panic safety is handled separately by
    /// `gprr::install_panic_hook`.
    ///
    /// # Errors
    /// Returns the underlying `io::Error` from terminal setup, drawing,
    /// event polling, or teardown.
    pub async fn run(mut self) -> io::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        stdout.execute(EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let theme = self.theme;
        let (data_tx, mut data_rx) = unbounded_channel();
        let mut events = EventStream::new();
        let mut tick = interval(Duration::from_millis(500));

        // Seed viewport_width so narrow-terminal logic works from the first frame.
        if let Ok((cols, _rows)) = crossterm::terminal::size() {
            self.state.viewport_width = cols;
        }

        // Seed an initial Tick so handle_tick fires the first FetchDashboard/
        // FetchPrList before any user input arrives. Without this the dashboard
        // sits empty until the first 500ms interval elapses.
        let initial_effects = update(&mut self.state, &AppEvent::Tick);
        dispatch_all(initial_effects, &self.state, &self.client, &data_tx);

        let result: io::Result<()> = async {
            loop {
                // Compute dt and advance the animation clock.
                let now = Instant::now();
                let dt = now.duration_since(self.last_frame);
                self.last_frame = now;

                // Borrow-split so the mutable `effect_manager` and the
                // immutable `state` can both be captured in the draw closure.
                let fx = &mut self.effect_manager;
                let state = &self.state;
                let anim_on = state.animations != AnimationIntensity::Off;
                terminal.draw(|f| {
                    render::render_app(f, state, &theme);
                    if anim_on && fx.is_running() {
                        let area = f.area();
                        fx.process_effects(
                            tachyonfx::Duration::from(dt),
                            f.buffer_mut(),
                            area,
                        );
                    }
                })?;

                if self.state.should_quit {
                    break;
                }

                // Compute frame budget BEFORE select (immutable borrow ends).
                let fx_has_active = self.effect_manager.is_running();
                let budget = frame_budget(fx_has_active);

                let app_event: Option<AppEvent> = tokio::select! {
                    maybe_term = events.next() => match maybe_term {
                        Some(Ok(CtEvent::Key(k))) => {
                            // Key press: clear active effects so the UI snaps
                            // to final state immediately.
                            if self.effect_manager.is_running() {
                                // Re-init the manager to drop all active effects.
                                self.effect_manager = tachyonfx::EffectManager::default();
                            }
                            Some(AppEvent::Key(k))
                        }
                        Some(Ok(CtEvent::Resize(w, h))) => Some(AppEvent::Resize { width: w, height: h }),
                        Some(Ok(_)) => None,
                        Some(Err(e)) => return Err(e),
                        None => break,
                    },
                    _ = tick.tick() => Some(AppEvent::Tick),
                    maybe_data = data_rx.recv() => maybe_data.map(AppEvent::Data),
                    // Fast frame-wake: active only while effects are running.
                    // Yields None so update/handle_tick are NOT called.
                    () = async {
                        match budget {
                            Some(d) => tokio::time::sleep(d).await,
                            None => std::future::pending::<()>().await,
                        }
                    } => None,
                };

                if let Some(event) = app_event {
                    let effects = update(&mut self.state, &event);
                    dispatch_all(effects, &self.state, &self.client, &data_tx);

                    // Drain animation cues produced by this update.
                    let cues: Vec<AnimationCue> =
                        std::mem::take(&mut self.state.animation_cues);
                    // Snapshot whether any effect was already running *before*
                    // we add the new ones (see `should_reset_frame_clock`).
                    let was_running = self.effect_manager.is_running();
                    let area = terminal.size().unwrap_or_default().into();
                    for cue in &cues {
                        let fx = effect_for(cue, area, &theme);
                        self.effect_manager.add_unique_effect(cue_key(cue), fx);
                    }
                    // On the idle→active transition, restart the frame clock so
                    // the (possibly multi-hundred-ms) blocking wait that
                    // preceded this event is not charged as the first `dt` —
                    // which would otherwise run the whole short effect to
                    // completion in a single, imperceptible frame.
                    if should_reset_frame_clock(was_running, !cues.is_empty()) {
                        self.last_frame = Instant::now();
                    }
                }
            }
            Ok(())
        }
        .await;

        disable_raw_mode()?;
        terminal.backend_mut().execute(LeaveAlternateScreen)?;
        result
    }
}

/// Dispatch a batch of effects, expanding any `Effect::Refetch` into the
/// concrete `Fetch*` effects implied by the completed write and the current
/// `AppState` (the executor cannot see state, so the expansion happens here).
fn dispatch_all(
    effects: Vec<Effect>,
    state: &AppState,
    client: &SharedGitHubClient,
    data_tx: &tokio::sync::mpsc::UnboundedSender<self::effect::DataEvent>,
) {
    for effect in effects {
        match effect {
            Effect::Refetch { kind } => {
                for expanded in self::update::refetch_effects(state, &kind) {
                    execute_effect(expanded, client.clone(), data_tx.clone());
                }
            }
            other => execute_effect(other, client.clone(), data_tx.clone()),
        }
    }
}

#[cfg(test)]
mod landing_tests {
    use super::*;
    use crate::data::models::Repo;

    fn repo() -> Repo {
        Repo {
            owner: "acme".into(),
            name: "widgets".into(),
        }
    }

    #[test]
    fn dashboard_flag_forces_dashboard() {
        let v = cli_landing(View::PrList { repo: repo() }, true, Some(repo()), Some(1));
        assert!(matches!(v, View::Dashboard));
    }

    #[test]
    fn explicit_repo_and_pr_opens_detail() {
        let v = cli_landing(View::Dashboard, false, Some(repo()), Some(42));
        match v {
            View::PrDetail { id, .. } => {
                assert_eq!(id.number, 42);
                assert_eq!(id.repo, repo());
            }
            other => panic!("expected PrDetail, got {other:?}"),
        }
    }

    #[test]
    fn pr_without_repo_uses_cwd_repo() {
        let v = cli_landing(View::PrList { repo: repo() }, false, None, Some(7));
        assert!(matches!(v, View::PrDetail { id, .. } if id.number == 7));
    }

    #[test]
    fn pr_without_any_repo_keeps_cwd_view() {
        let v = cli_landing(View::Dashboard, false, None, Some(7));
        assert!(
            matches!(v, View::Dashboard),
            "no repo context → cannot open PR"
        );
    }

    #[test]
    fn explicit_repo_only_opens_pr_list() {
        let v = cli_landing(View::Dashboard, false, Some(repo()), None);
        assert!(matches!(v, View::PrList { repo: r } if r == repo()));
    }

    // --- workspace seeding (seed_workspace_from_view) ---

    #[test]
    fn seed_dashboard_view_leaves_single_pinned_tab() {
        let mut s = crate::app::state::AppState::default();
        seed_workspace_from_view(&mut s, View::Dashboard);
        assert_eq!(s.workspace.tabs.len(), 1);
        assert_eq!(s.workspace.active, 0);
        assert!(matches!(
            s.workspace.active_tab().kind,
            crate::app::workspace::TabKind::Dashboard
        ));
    }

    #[test]
    fn seed_pr_list_view_adds_focused_repo_tab() {
        let mut s = crate::app::state::AppState::default();
        seed_workspace_from_view(&mut s, View::PrList { repo: repo() });
        // [Dashboard, Repo], Repo focused.
        assert_eq!(s.workspace.tabs.len(), 2);
        assert!(matches!(
            s.workspace.active_tab().kind,
            crate::app::workspace::TabKind::Repo(ref r) if *r == repo()
        ));
    }

    #[test]
    fn seed_pr_detail_view_seeds_dashboard_repo_and_focused_pr() {
        use crate::app::workspace::TabKind;
        let id = crate::data::models::PrId {
            repo: repo(),
            number: 42,
        };
        let mut s = crate::app::state::AppState::default();
        seed_workspace_from_view(
            &mut s,
            View::PrDetail {
                id: id.clone(),
                tab: crate::app::state::DetailTab::Files,
            },
        );
        // [Dashboard, Repo, Pr], Pr focused, detail sub-tab preserved.
        assert_eq!(s.workspace.tabs.len(), 3);
        assert!(matches!(s.workspace.active_tab().kind, TabKind::Pr(ref i) if *i == id));
        assert!(
            s.workspace
                .tabs
                .iter()
                .any(|t| matches!(t.kind, TabKind::Dashboard))
        );
        assert!(
            s.workspace
                .tabs
                .iter()
                .any(|t| matches!(t.kind, TabKind::Repo(ref r) if *r == repo())),
            "a Repo tab for the PR's repo must also be seeded"
        );
        assert_eq!(
            s.workspace.active_tab().state.detail_tab,
            crate::app::state::DetailTab::Files
        );
    }
}
