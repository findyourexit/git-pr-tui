use std::path::PathBuf;

use clap::Parser;

/// CLI for `gprr` — the GitHub PR TUI.
///
/// All flags are optional. Behaviour:
///   * `--dashboard` and `--pr <N>` force the landing view; `--pr` requires
///     a positional `owner/name` (or cwd-detected repo).
///   * Positional `owner/name` selects the repo for `PrList` / `--pr`.
///   * `--debug` / `--trace` and `--no-color` affect logging/rendering.
///   * `--config <path>` overrides the config file location.
#[derive(Debug, Parser, PartialEq, Eq)]
#[command(name = "gprr", version, about = "TUI for managing GitHub PRs")]
#[allow(clippy::struct_excessive_bools)]
pub struct Cli {
    /// Open the personal dashboard regardless of cwd.
    #[arg(long, conflicts_with = "pr")]
    pub dashboard: bool,

    /// Open a specific PR by number (requires `owner/name`).
    #[arg(long, value_name = "N")]
    pub pr: Option<u64>,

    /// Repo in `owner/name` form. Defaults to cwd-detected repo.
    #[arg(value_name = "owner/name")]
    pub repo: Option<RepoArg>,

    /// Verbose logging.
    #[arg(long)]
    pub debug: bool,

    /// Very verbose logging.
    #[arg(long)]
    pub trace: bool,

    /// Disable ANSI colour output.
    #[arg(long)]
    pub no_color: bool,

    /// Override the config file location (default: `$XDG_CONFIG_HOME/gprr/config.toml`).
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Explore the full TUI offline against built-in sample data — no `gh`
    /// token or network required. Forces the dashboard landing view.
    #[arg(long)]
    pub demo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoArg {
    pub owner: String,
    pub name: String,
}

impl From<RepoArg> for crate::data::models::Repo {
    fn from(arg: RepoArg) -> Self {
        Self {
            owner: arg.owner,
            name: arg.name,
        }
    }
}

impl std::str::FromStr for RepoArg {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (owner, name) = s
            .split_once('/')
            .ok_or_else(|| format!("expected `owner/name`, got `{s}`"))?;
        if owner.is_empty() || name.is_empty() || name.contains('/') {
            return Err(format!("expected `owner/name`, got `{s}`"));
        }
        Ok(Self {
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("gprr").chain(args.iter().copied())).expect("parse")
    }

    #[test]
    fn defaults_are_all_off() {
        let cli = parse(&[]);
        assert!(!cli.dashboard);
        assert!(cli.pr.is_none());
        assert!(cli.repo.is_none());
        assert!(!cli.debug);
        assert!(!cli.trace);
        assert!(!cli.no_color);
        assert!(cli.config.is_none());
    }

    #[test]
    fn dashboard_flag_sets_dashboard_true() {
        let cli = parse(&["--dashboard"]);
        assert!(cli.dashboard);
    }

    #[test]
    fn pr_flag_parses_number() {
        let cli = parse(&["--pr", "42", "acme/widgets"]);
        assert_eq!(cli.pr, Some(42));
        let repo = cli.repo.expect("positional");
        assert_eq!(repo.owner, "acme");
        assert_eq!(repo.name, "widgets");
    }

    #[test]
    fn positional_owner_name_parses() {
        let cli = parse(&["acme/widgets"]);
        let repo = cli.repo.expect("positional");
        assert_eq!(repo.owner, "acme");
        assert_eq!(repo.name, "widgets");
    }

    #[test]
    fn positional_without_slash_errors() {
        let res = Cli::try_parse_from(["gprr", "acme-widgets"]);
        assert!(res.is_err(), "missing slash must error");
    }

    #[test]
    fn debug_and_trace_independent_flags() {
        let cli = parse(&["--debug", "--trace"]);
        assert!(cli.debug && cli.trace);
    }

    #[test]
    fn no_color_flag_sets_no_color_true() {
        let cli = parse(&["--no-color"]);
        assert!(cli.no_color);
    }

    #[test]
    fn config_flag_takes_path() {
        let cli = parse(&["--config", "/tmp/gprr.toml"]);
        assert_eq!(
            cli.config.as_deref(),
            Some(std::path::Path::new("/tmp/gprr.toml"))
        );
    }

    #[test]
    fn dashboard_and_pr_are_mutually_exclusive() {
        let res = Cli::try_parse_from(["gprr", "--dashboard", "--pr", "1"]);
        assert!(res.is_err(), "dashboard + --pr must conflict");
    }
}
