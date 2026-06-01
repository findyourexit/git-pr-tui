# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
