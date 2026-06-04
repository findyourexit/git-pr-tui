<h1 align="center">gpr</h1>

<p align="center">
  <strong>A keyboard-driven terminal UI for triaging, reviewing, and merging GitHub pull requests — without leaving the shell.</strong>
</p>

<p align="center">
  <a href="https://github.com/findyourexit/git-pr-tui/actions/workflows/ci.yml"><img src="https://github.com/findyourexit/git-pr-tui/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/findyourexit/git-pr-tui/releases"><img src="https://img.shields.io/github/v/release/findyourexit/git-pr-tui?include_prereleases&sort=semver" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/findyourexit/git-pr-tui" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/rust-1.88%2B-orange" alt="Rust 1.88+">
</p>

<p align="center">
  <img src="assets/demo.gif" alt="gpr — dashboard to diff tour" width="900">
</p>

`gpr` is a single, self-contained binary. It reuses your `gh` CLI login, keeps everything in memory, and gets you from "what needs my attention?" to an approved-and-merged PR in a handful of keystrokes.

> **Just looking?** Run `gpr --demo` to explore the whole interface offline against built-in sample data — no GitHub account, token, or network required.

## Features

- **Cross-repo dashboard** — three buckets at a glance: *needs your review*, *your open PRs*, and *assigned to you*.
- **Per-repo PR list** — search, filter, and sort across a repository's PRs.
- **PR detail** — Conversation, Files, Checks, and Commits sub-tabs with Markdown-rendered descriptions, timelines, and review threads.
- **Native in-TUI diff viewer** — `syntect` syntax highlighting, unified or side-by-side, with line-range comments.
- **Full review workflow** — comment, approve, request changes, reply to threads, merge, close/reopen, and check out the PR branch.
- **Discoverable by design** — a context-aware help overlay (`?`), command palette (`:`), and action menu (`Space`), all generated from a single action registry so they never drift from reality.
- **Tabs & motion** — browser-style tabs per repo/PR plus tasteful, optional animations.

## Quick start

### Install

<details open>
<summary><strong>Homebrew (macOS)</strong></summary>

```bash
brew tap findyourexit/tap
brew install gpr
```

</details>

<details>
<summary><strong>Pre-built binaries</strong></summary>

Download the archive for your platform from the [GitHub Releases](https://github.com/findyourexit/git-pr-tui/releases) page, unpack it, and put `gpr` on your `$PATH`.

Archives are published for:
- macOS (Apple Silicon and Intel)
- Linux (`x86_64` and `aarch64`)
- Windows (`x86_64`)

Each release includes a `checksums.txt` for verification.

</details>

<details>
<summary><strong>From source (Cargo)</strong></summary>

```bash
# Install the latest from git
cargo install --git https://github.com/findyourexit/git-pr-tui --locked --bin gpr

# …or build a local checkout
git clone https://github.com/findyourexit/git-pr-tui.git
cd git-pr-tui
cargo build --release   # binary at target/release/gpr
```

`gpr` is **not** on crates.io yet — install via Homebrew, a release archive, or git.

</details>

### Prerequisites

- **[`gh` CLI](https://cli.github.com/)**, authenticated with the `repo` scope:

  ```bash
  gh auth login --scopes repo
  ```

  `gpr` calls `gh auth token` for every GitHub API request. There is no native OAuth and no `GITHUB_TOKEN` fallback. (The Homebrew formula installs `gh` for you.)
- **`git`** on your `$PATH` (used by the `C` branch-checkout flow).

### Run it

```bash
# Outside a repo → the cross-repo dashboard
gpr

# Inside a git repo with a github.com remote → that repo's PR list
cd ~/code/my-project && gpr

# A specific repo, or a specific PR
gpr owner/name
gpr --pr 128 owner/name

# Explore the UI offline, no auth required
gpr --demo
```

Press `?` anywhere for the context-aware key overlay, and `:` for the command palette.

## A closer look

**Get around — dashboard, command palette, and a searchable, filterable PR list:**

<p align="center">
  <img src="assets/demo-dashboard.gif" alt="Dashboard, command palette, and PR list" width="900">
</p>

**Review a change — diffs, line-range comments, and a live Markdown preview:**

<p align="center">
  <img src="assets/demo-review.gif" alt="Diff viewer, line comment, and Markdown preview" width="900">
</p>

## Key reference

The in-app `?` overlay is the source of truth: it is rebuilt every frame from the action registry ([`src/app/actions.rs`](src/app/actions.rs)) and the live keymap ([`src/app/keymap.rs`](src/app/keymap.rs)), so it always lists exactly the actions available in the current view and can never drift. The tables below summarise the defaults; bindings are remappable via `[keys]` in `config.toml` (see [Config](#config)).

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

In the **Files** sub-tab, `h` / `l` move between the file tree and the diff pane, `Enter` opens the focused file's diff, and `L` loads the per-file diffs for large PRs that are not fetched automatically.

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

`:` opens an input that fuzzy-matches the current view's actions plus the commands `refresh`, `log`, `quit`, `dashboard`, `pr <n>` (open a PR by number in the current repo), and `repo <owner/name>` (open another repo's PR list).

### Composer

| Key | Action |
|---|---|
| `Ctrl-S` | Submit |
| `Ctrl-P` | Toggle the Markdown preview |
| `Esc` | Cancel |

## Config

`gpr` reads `~/.config/gpr/config.toml` (or `$XDG_CONFIG_HOME/gpr/config.toml`). The file is written with defaults on first launch if absent.

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

Logs are written to `$XDG_DATA_HOME/gpr/log/gpr.log.YYYY-MM-DD` (rolling daily). The last 200 lines are also visible in-app via `:log`. Set `GPR_LOG=debug` (or pass `--debug` / `--trace`) to widen verbosity; `GPR_LOG` takes precedence over the flags.

## Troubleshooting

- **`gh: command not found`** — install GitHub CLI (<https://cli.github.com/>) and run `gh auth login --scopes repo`.
- **`401 Unauthorized` toast** — your `gh` token expired or lost the `repo` scope. Run `gh auth refresh -s repo`.
- **`403 rate limit exceeded`** — GitHub throttled you. Wait for the reset (`gh api rate_limit`) or reduce poll frequency in `config.toml`.
- **Terminal smaller than 60×18** — modals fall back to full-screen. Resize larger for the standard overlay layout.
- **PR branch checkout refused** — `C` refuses to overwrite a dirty worktree. Commit or stash first.

## Non-goals (v0.1.0)

`gpr` deliberately omits the following:

- GitHub Enterprise Server
- Editing PR title/body, reviewers/labels/assignees
- Draft toggles, dismiss reviews, resolve/unresolve review threads
- Dedicated UI to compose suggested-change blocks (rendering only)
- Persistent SQLite cache / ETag handling
- Native OAuth, `GITHUB_TOKEN` fallback
- Mouse support, user-defined themes
- crates.io publish
- Notifications inbox

## Releasing (maintainer only)

Releases are driven by tags. Pushing a `v*` tag runs [`.github/workflows/release.yml`](.github/workflows/release.yml), which:

1. cross-compiles release binaries for macOS (arm64/x86_64), Linux (`x86_64`/`aarch64`), and Windows (`x86_64`);
2. packages archives + `checksums.txt` and publishes a GitHub Release whose notes are taken from the `[Unreleased]` section of [`CHANGELOG.md`](CHANGELOG.md);
3. promotes that changelog section to the new version; and
4. updates the `gpr` formula in the [`findyourexit/homebrew-tap`](https://github.com/findyourexit/homebrew-tap) tap (requires a `HOMEBREW_TAP_TOKEN` repository secret).

```bash
# 1. Land your changes under "## [Unreleased]" in CHANGELOG.md
# 2. Bump the version in Cargo.toml, then tag and push
git tag v0.1.0
git push origin v0.1.0
```

Linux targets are built with [`cross`](https://github.com/cross-rs/cross); see [`Cross.toml`](Cross.toml) for the container setup (it provisions CMake so the `aws-lc-rs` crypto backend compiles).

## Recording demo assets (maintainer only)

The README GIFs are produced with [`vhs`](https://github.com/charmbracelet/vhs) from the tapes in [`tapes/`](tapes/), recorded against `gpr --demo` so they are deterministic and contain no real GitHub data.

```bash
brew install vhs                       # also pulls ttyd + ffmpeg
cargo build --release                  # the tapes expect ./gpr on $PATH
cp target/release/gpr /tmp/gpr-demo/   # (tapes set a clean XDG/PATH env)
vhs tapes/demo.tape                     # → assets/demo.gif
vhs tapes/demo-dashboard.tape
vhs tapes/demo-review.tape
```

## Recording API fixtures (maintainer only)

Test fixtures under `tests/fixtures/api/` capture live GitHub API responses so the integration suite can run without network access. CI never re-records — only maintainers run the script manually after the GitHub API shape changes or when a captured fixture grows stale.

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

Commit the regenerated fixtures in a dedicated commit so the diff review stays focused.

## License

MIT — see [LICENSE](LICENSE).
