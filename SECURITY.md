# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in `gpr`, please report it
responsibly. **Do not open a public issue.**

Instead, please use one of the following methods:

1. **GitHub Security Advisories** (preferred): use the
   [Report a vulnerability](https://github.com/findyourexit/git-pr-tui/security/advisories/new)
   button on the Security tab of this repository.
2. **Contact the maintainer** directly via [@findyourexit](https://github.com/findyourexit).

## What to Include

When reporting a vulnerability, please provide:

- A description of the vulnerability and its potential impact.
- Steps to reproduce the issue.
- Any relevant configuration or environment details.
- A suggested fix, if you have one.

## Response

We will acknowledge receipt within 7 days and aim to provide a resolution or
mitigation plan within 30 days. We will keep you informed of progress and may
ask for additional information.

## Supported Versions

Security fixes are applied to the latest release only. We recommend always
running the most recent version.

## A Note on Credentials

`gpr` performs **no** authentication of its own. It shells out to the
`gh` CLI (`gh auth token`) for a GitHub token at call time and never stores,
caches, or transmits credentials itself. There is no OAuth flow and no
`GITHUB_TOKEN` environment fallback. Token lifecycle and storage are owned
entirely by the `gh` CLI.
