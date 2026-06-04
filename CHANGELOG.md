# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-06-04

### Changed

- Renamed the binary, crate, and Homebrew formula from `gpr` to `gprr` to avoid
  a collision with Oh My Zsh's `gpr` alias for `git pull --rebase`. This also
  moves the config directory to `~/.config/gprr/`, the log directory to
  `$XDG_DATA_HOME/gprr/log/`, and renames the `GPR_LOG` / `GPR_FIXTURE_*`
  environment variables to `GPRR_LOG` / `GPRR_FIXTURE_*`.

### Added

- Cross-repo dashboard with three buckets: needs-your-review, your open PRs,
  and assigned to you.
- Per-repo PR list with search, filter, and sort.
- PR detail view with Conversation, Files, Checks, and Commits sub-tabs.
- Native in-TUI diff viewer with `syntect` syntax highlighting, unified and
  side-by-side modes, and line-range comments.
- Review actions: comment, approve, request changes, reply to review threads,
  merge, close/reopen, and checkout of the PR branch.
- Markdown rendering (`pulldown-cmark`) with a live composer preview.
- Context-aware command palette, action menu, and a help overlay generated
  from the action registry.
- Configurable keymap, theme (dark/light), animation intensity, and background
  refresh cadence via `config.toml`.
- GitHub authentication delegated to the `gh` CLI; in-memory cache only.
- Offline demo mode (`gprr --demo`) that runs the full TUI against built-in
  sample data, with no `gh` token or network access required.
- Pre-built release binaries for macOS, Linux, and Windows, plus Homebrew tap
  installation (`brew install findyourexit/tap/gprr`), driven by a tag-triggered
  release workflow.
