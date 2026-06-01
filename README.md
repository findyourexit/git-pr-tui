# gpr — keyboard-driven GitHub PR TUI

[![CI](https://github.com/findyourexit/git-pr-tui/actions/workflows/ci.yml/badge.svg)](https://github.com/findyourexit/git-pr-tui/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`gpr` is a single-binary, keyboard-only terminal UI for triaging,
reviewing, and merging GitHub pull requests without leaving the shell.

- **Dashboard** — three buckets across all your repos: *needs your review*,
  *your open PRs*, *assigned to you*.
- **Per-repo PR list** — filter, sort, search.
- **PR detail** — Conversation / Files / Checks / Commits tabs.
- **Native in-TUI diff viewer** — syntax highlighting via `syntect`,
  unified or side-by-side, line-range comments.
- **Full review actions** — comment, approve, request changes, reply to
  threads, merge, close/reopen, checkout the PR branch.
- **In-memory cache** — manual or background refresh; nothing persisted.

## Install

```bash
cargo install --git https://github.com/<your-fork>/git-pr-tui --locked --bin gpr
```

`gpr` is **not** published to crates.io in v0.1.0; install from git only.

## Prerequisites

- **`gh` CLI**, authenticated with `repo` scope:

  ```bash
  gh auth login --scopes repo
  ```

  `gpr` reuses `gh auth token` for every GitHub API call. No native OAuth,
  no `GITHUB_TOKEN` env fallback.

- **`git`** on `$PATH` (used for `C` checkout flow).

## Quickstart

```bash
# Anywhere outside a repo → dashboard across all your repos
gpr

# Inside a git repo with a `github.com` remote → that repo's PR list
cd ~/code/my-project
gpr

# Explicit repo
gpr owner/name
```

`?` opens a per-view keybinding overlay. `:` opens the command palette.

## Key reference

The in-app `?` overlay is the source of truth: it is rebuilt every frame from
the action registry ([`src/app/actions.rs`](src/app/actions.rs)) and the live
keymap ([`src/app/keymap.rs`](src/app/keymap.rs)), so it always lists exactly
the actions available in the current view and can never drift. The tables
below summarise the defaults; bindings are remappable via `[keys]` in
`config.toml` (see [Config](#config)).

### Global

| Key | Action |
|---|---|
| `?` | Toggle the context-aware key overlay |
| `:` | Command palette |
| `Space` | Action menu (every action for the current view) |
| `j` / `k` (`↓` / `↑`) | Move selection / scroll |
| `g` / `G` | Jump to top / bottom |
| `Ctrl-d` / `Ctrl-u` | Half-page down / up |
| `Enter` | Open / select |
| `o` | Open the selection in the browser |
| `R` | Refresh |
| `q` / `Ctrl-C` | Quit |

### Tabs

Each repo and PR opens in its own tab; the dashboard is the pinned first tab.

| Key | Action |
|---|---|
| `Tab` / `Shift-Tab` | Next / previous tab |
| `1`–`9` | Jump to tab _N_ |
| `w` | Close the current tab (the dashboard is pinned) |

### Dashboard

| Key | Action |
|---|---|
| `h` / `l` (`←` / `→`) | Switch bucket (review-requested / authored / assigned) |
| `Enter` | Open the highlighted PR |
| `f` / `s` | Filter / sort |

### PR list

| Key | Action |
|---|---|
| `/` | Search |
| `f` / `s` | Filter / sort |
| `M` | Load more |
| `C` | Checkout the PR branch |
| `Enter` | Open the PR |

### PR detail

| Key | Action |
|---|---|
| `[` / `]` | Previous / next sub-tab (Conversation · Files · Checks · Commits) |
| `j` / `k` | Scroll |
| `c` | Comment on the PR |
| `v` | Start a review |
| `m` | Merge |
| `x` | Close / reopen |
| `C` | Checkout the PR branch |
| `n` / `N` | Next / previous review thread |
| `R` | Reply to the focused review thread (refreshes when the PR has no threads) |

In the **Files** sub-tab, `h` / `l` move between the file tree and the diff
pane, `Enter` opens the focused file's diff, and `L` loads the per-file diffs
for large PRs that are not fetched automatically.

### Diff viewer

| Key | Action |
|---|---|
| `j` / `k` | Move the line cursor |
| `]` / `[` | Next / previous hunk |
| `}` / `{` | Next / previous file |
| `t` | Toggle unified / side-by-side |
| `W` | Toggle whitespace hidden |
| `V` | Start a line selection (then `j` / `k` extend, `c` comment, `Esc` cancel) |
| `Esc` | Back to the Files sub-tab |

### Command palette

`:` opens an input that fuzzy-matches the current view's actions plus the
commands `refresh`, `log`, `quit`, `dashboard`, `pr <n>` (open a PR by number
in the current repo), and `repo <owner/name>` (open another repo's PR list).

### Composer

| Key | Action |
|---|---|
| `Ctrl-S` | Submit |
| `Ctrl-P` | Toggle the Markdown preview |
| `Esc` | Cancel |

## Config

`gpr` reads `~/.config/gpr/config.toml` (or `$XDG_CONFIG_HOME/gpr/config.toml`).
The file is written with defaults on first launch if absent.

```toml
[ui]
# theme: "dark" | "light"  (overridden at runtime by --no-color / NO_COLOR)
theme = "dark"
# animations: "full" | "subtle" | "off"  (default "full")
#   full   = all motion (tab slide, overlay fade, data coalesce, focus pulse, action sweep)
#   subtle = fades + focus pulse only
#   off    = no motion
# Forced to "off" by --no-color / NO_COLOR. (Terminal prefers-reduced-motion is not
# portably detectable, so use "off" if you are motion-sensitive.)
animations = "full"

[refresh]
# background-refresh cadence per resource, in seconds
dashboard = 60
pr_list = 60
pr_detail = 30

[keys]
# remap an intent to a key spec, e.g.:
# quit = "Q"
# refresh = "F5"
```

Logs are written to `$XDG_DATA_HOME/gpr/log/gpr.log.YYYY-MM-DD`
(rolling daily). The last 200 lines are also visible in-app via
`:log`. Set `GPR_LOG=debug` (or pass `--debug` / `--trace`) to widen
verbosity; `GPR_LOG` takes precedence over the flags.

## Troubleshooting

- **`gh: command not found`** — install GitHub CLI
  (<https://cli.github.com/>) and run `gh auth login --scopes repo`.
- **`401 Unauthorized` toast** — your `gh` token expired or lost the
  `repo` scope. Run `gh auth refresh -s repo`.
- **`403 rate limit exceeded`** — GitHub throttled you. Wait for the
  reset (`gh api rate_limit`) or reduce poll frequency in
  `config.toml`.
- **Terminal smaller than 60×18** — modals fall back to full-screen.
  Resize larger for the standard overlay layout.
- **PR branch checkout refused** — `C` refuses to overwrite a dirty
  worktree. Commit or stash first.

## Non-goals (v0.1.0)

`gpr` deliberately omits the following:

- GitHub Enterprise Server
- Editing PR title/body, reviewers/labels/assignees
- Draft toggles, dismiss reviews, resolve/unresolve review threads
- Dedicated UI to compose suggested-change blocks (rendering only)
- Persistent SQLite cache / ETag handling
- Native OAuth, `GITHUB_TOKEN` fallback
- Mouse support, user-defined themes
- Homebrew tap, prebuilt binaries, crates.io publish
- Notifications inbox

## Recording fixtures (maintainer only)

Test fixtures under `tests/fixtures/api/` capture live GitHub API
responses so the integration suite can run without network access. CI
never re-records — only maintainers run the script manually after the
GitHub API shape changes or when a captured fixture grows stale.

```bash
gh auth login --scopes repo          # one-time
scripts/record_fixtures.sh           # captures against cli/cli#1 by default
```

Override the source repo or PR via env vars:

| Env var | Default | Purpose |
|---|---|---|
| `GPR_FIXTURE_REPO` | `cli/cli` | `owner/name` of the source repo |
| `GPR_FIXTURE_PR` | `1` | PR number to capture (use one with reviews + checks) |
| `GPR_FIXTURE_DIR` | `tests/fixtures/api` | Output directory |

Commit the regenerated fixtures in a dedicated commit so the diff
review stays focused.

## License

MIT — see [LICENSE](LICENSE).
