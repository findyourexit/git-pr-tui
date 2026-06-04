use std::io;
use std::sync::Arc;

use clap::Parser;
use crossterm::ExecutableCommand;
use gprr::app::App;
use gprr::cli::Cli;
use gprr::data::SharedGitHubClient;
use gprr::data::auth_probe::{GhTokenFetcher, ProbeOutcome, probe};
use gprr::data::github::octocrab_impl::{OctocrabConfig, OctocrabGitHubClient};
use gprr::data::github::{GitHubClient, GitHubError};
use gprr::install_panic_hook;

#[tokio::main]
async fn main() -> io::Result<()> {
    let cli = Cli::parse();

    // `--trace` / `--debug` set the default log level; `GPRR_LOG` still overrides.
    let log_default = if cli.trace {
        "trace"
    } else if cli.debug {
        "debug"
    } else {
        "info"
    };
    // Bind to a named variable (NOT `_`) so the file-appender's WorkerGuard
    // lives for the full process lifetime. `_` would drop immediately,
    // silently disabling the log file.
    let _log_guard = gprr::logging::init_subscriber_with_default(log_default);

    // `--demo` runs the whole TUI against in-memory sample data, skipping the
    // `gh` auth probe and every network call.
    let client: SharedGitHubClient = if cli.demo {
        gprr::demo::demo_client()
    } else {
        let outcome = probe(&GhTokenFetcher, |token| async move {
            let client = OctocrabGitHubClient::with_token(token, OctocrabConfig::default())
                .map_err(|e| GitHubError::Network(e.to_string()))?;
            client.auth_scopes().await
        })
        .await;
        let token = match outcome {
            ProbeOutcome::Ok(t) => t,
            other => {
                eprintln!("{}", other.stderr_message());
                std::process::exit(1);
            }
        };

        match OctocrabGitHubClient::with_token(token, OctocrabConfig::default()) {
            Ok(c) => Arc::new(c),
            Err(e) => {
                eprintln!("Failed to initialize GitHub client: {e}");
                std::process::exit(1);
            }
        }
    };

    install_panic_hook(Box::new(|| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = std::io::stdout().execute(crossterm::terminal::LeaveAlternateScreen);
    }));
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let config = gprr::config::ConfigFile::load(cli.config.as_deref());
    let mut app = App::with_cwd_and_config(client, &cwd, cli.config.as_deref());

    // Apply CLI landing overrides (`--dashboard`, `owner/name`, `--pr`).
    // In demo mode, ignore the cwd-derived repo so a bare `gprr --demo` always
    // lands on the populated sample dashboard (even when launched inside an
    // unrelated git repo). An explicit `owner/name`, `--pr`, or `--dashboard`
    // still wins via `cli_landing`.
    let cwd_view = if cli.demo {
        gprr::app::state::View::Dashboard
    } else {
        app.state.current_view()
    };
    let view = gprr::app::cli_landing(cwd_view, cli.dashboard, cli.repo.map(Into::into), cli.pr);
    gprr::app::seed_workspace_from_view(&mut app.state, view);

    // `--no-color` / `NO_COLOR` defer all colors to the terminal defaults,
    // overriding the config theme.
    if cli.no_color || std::env::var_os("NO_COLOR").is_some() {
        app.theme = gprr::ui::theme::Theme::no_color();
        app.state.animations = gprr::app::anim::AnimationIntensity::Off;
    } else {
        app.state.animations =
            gprr::app::anim::AnimationIntensity::from_config_str(&config.ui.animations);
    }

    app.run().await
}
