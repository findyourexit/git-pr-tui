#!/usr/bin/env bash
# scripts/record_fixtures.sh
#
# Captures the live API responses we need as test fixtures.
# Run manually after `gh auth login --scopes repo`. CI never runs this.
#
# Targets a stable public repo with rich PR activity by default.
# Override via env vars:
#   GPRR_FIXTURE_REPO   default: cli/cli
#   GPRR_FIXTURE_PR     default: 1   (override to a recently merged PR with reviews + checks)
#   GPRR_FIXTURE_DIR    default: tests/fixtures/api
set -euo pipefail

REPO="${GPRR_FIXTURE_REPO:-cli/cli}"
PR="${GPRR_FIXTURE_PR:-1}"
OUT="${GPRR_FIXTURE_DIR:-tests/fixtures/api}"
mkdir -p "$OUT"

OWNER="${REPO%%/*}"
NAME="${REPO##*/}"

TOKEN="$(gh auth token)"
HDR=(-H "Authorization: token $TOKEN" -H "Accept: application/vnd.github+json")

# ---------- viewer (REST) ----------
curl -sS "${HDR[@]}" "https://api.github.com/user" > "$OUT/viewer.json"

# ---------- pr_detail (GraphQL with eager pagination shape) ----------
# Emits every field required to construct PrDetail and the embedded
# PrSummary from a single response. Uses `mergeStateStatus` (8-state).
# `repository.{merge,squash,rebase}*Allowed` feed `available_merge_methods`.
# Timeline includes only the v1 variants; v2 events (labeled/unlabeled/
# assigned/unassigned/review_requested/dismissed) are intentionally absent.
# `bodyHTML`, `reviewRequests`+`assignees`, and `mergeCommit { oid }` cover
# the remaining PrDetail fields. Per-thread `position`/`startLine`
# +`startDiffSide` collapse to a single-side LineRange at the parser.
# `pushedDate` on commits feeds `TimelineEvent::Commit.pushed_at`.
read -r -d '' Q_DETAIL <<'GQL' || true
query Detail($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    nameWithOwner
    mergeCommitAllowed
    squashMergeAllowed
    rebaseMergeAllowed
    pullRequest(number: $number) {
      number title body bodyHTML state isDraft updatedAt
      mergeStateStatus
      reviewDecision
      additions deletions changedFiles
      totalCommentsCount
      headRefOid baseRefOid
      baseRefName headRefName
      author { login }
      mergeCommit { oid }
      assignees(first: 20) { nodes { login } }
      reviewRequests(first: 20) {
        nodes {
          requestedReviewer {
            ... on User { login }
            ... on Team { name }
          }
        }
      }
      labels(first: 50) { nodes { name color } }
      commits(first: 50) {
        pageInfo { hasNextPage endCursor }
        nodes {
          commit {
            oid
            messageHeadline
            authoredDate
            pushedDate
            author { user { login } }
            statusCheckRollup { state }
          }
        }
      }
      reviewThreads(first: 20) {
        pageInfo { hasNextPage endCursor }
        nodes {
          id isResolved isOutdated
          path
          line startLine
          originalLine originalStartLine
          diffSide startDiffSide
          comments(first: 20) {
            nodes {
              id
              author { login }
              body
              createdAt
              position originalPosition
              commit { oid }
              originalCommit { oid }
            }
          }
        }
      }
      timelineItems(first: 50, itemTypes: [
        PULL_REQUEST_COMMIT,
        ISSUE_COMMENT,
        PULL_REQUEST_REVIEW,
        HEAD_REF_FORCE_PUSHED_EVENT,
        MERGED_EVENT, CLOSED_EVENT, REOPENED_EVENT
      ]) {
        pageInfo { hasNextPage endCursor }
        nodes {
          __typename
          ... on PullRequestCommit {
            commit {
              oid
              messageHeadline
              pushedDate
              author { user { login } name }
            }
          }
          ... on IssueComment {
            id author { login } body createdAt
          }
          ... on PullRequestReview {
            id author { login } state body submittedAt
          }
          ... on HeadRefForcePushedEvent {
            createdAt actor { login }
            beforeCommit { oid }
            afterCommit { oid }
          }
          ... on MergedEvent {
            createdAt actor { login }
            commit { oid }
          }
          ... on ClosedEvent {
            createdAt actor { login }
          }
          ... on ReopenedEvent {
            createdAt actor { login }
          }
        }
      }
    }
  }
}
GQL
curl -sS "${HDR[@]}" https://api.github.com/graphql \
  -d "$(jq -n --arg q "$Q_DETAIL" --arg o "$OWNER" --arg n "$NAME" --argjson p "$PR" \
        '{query:$q, variables:{owner:$o, name:$n, number:$p}}')" \
  > "$OUT/pr_detail.graphql.json"

# ---------- pr_list (GraphQL) ----------
# PrSummary node block — must match dashboard block exactly so parsers share code.
read -r -d '' Q_LIST <<'GQL' || true
query PrList($owner: String!, $name: String!) {
  repository(owner: $owner, name: $name) {
    pullRequests(first: 10, states: [OPEN, CLOSED, MERGED], orderBy: {field: UPDATED_AT, direction: DESC}) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number title state isDraft updatedAt
        author { login }
        baseRefName headRefName
        headRefOid
        additions deletions changedFiles
        totalCommentsCount
        reviewDecision
        mergeStateStatus
        repository { nameWithOwner }
        labels(first: 10) { nodes { name color } }
        commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
      }
    }
  }
}
GQL
curl -sS "${HDR[@]}" https://api.github.com/graphql \
  -d "$(jq -n --arg q "$Q_LIST" --arg o "$OWNER" --arg n "$NAME" \
        '{query:$q, variables:{owner:$o, name:$n}}')" \
  > "$OUT/pr_list.graphql.json"

# ---------- dashboard (GraphQL) ----------
read -r -d '' Q_DASH <<'GQL' || true
query Dashboard {
  reviewRequested: search(query: "is:open is:pr review-requested:@me archived:false", type: ISSUE, first: 5) {
    pageInfo { hasNextPage endCursor }
    nodes { ... on PullRequest {
      number title state isDraft updatedAt
      author { login }
      baseRefName headRefName
      headRefOid
      additions deletions changedFiles
      totalCommentsCount
      reviewDecision
      mergeStateStatus
      repository { nameWithOwner }
      labels(first: 10) { nodes { name color } }
      commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
    } }
  }
  authored: search(query: "is:open is:pr author:@me archived:false", type: ISSUE, first: 5) {
    pageInfo { hasNextPage endCursor }
    nodes { ... on PullRequest {
      number title state isDraft updatedAt
      author { login }
      baseRefName headRefName
      headRefOid
      additions deletions changedFiles
      totalCommentsCount
      reviewDecision
      mergeStateStatus
      repository { nameWithOwner }
      labels(first: 10) { nodes { name color } }
      commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
    } }
  }
  assigned: search(query: "is:open is:pr assignee:@me archived:false", type: ISSUE, first: 5) {
    pageInfo { hasNextPage endCursor }
    nodes { ... on PullRequest {
      number title state isDraft updatedAt
      author { login }
      baseRefName headRefName
      headRefOid
      additions deletions changedFiles
      totalCommentsCount
      reviewDecision
      mergeStateStatus
      repository { nameWithOwner }
      labels(first: 10) { nodes { name color } }
      commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
    } }
  }
}
GQL
curl -sS "${HDR[@]}" https://api.github.com/graphql \
  -d "$(jq -n --arg q "$Q_DASH" '{query:$q}')" \
  > "$OUT/dashboard.graphql.json"

# ---------- pr_files (REST, paginated) ----------
curl -sS "${HDR[@]}" "https://api.github.com/repos/$REPO/pulls/$PR/files?per_page=100" \
  > "$OUT/pr_files.json"

# ---------- pr_diff (REST, diff media type) ----------
curl -sS -H "Authorization: token $TOKEN" -H "Accept: application/vnd.github.v3.diff" \
  "https://api.github.com/repos/$REPO/pulls/$PR" > "$OUT/pr_diff.patch"

# ---------- pr_checks (head sha) ----------
HEAD_SHA="$(curl -sS "${HDR[@]}" "https://api.github.com/repos/$REPO/pulls/$PR" | jq -r .head.sha)"
curl -sS "${HDR[@]}" "https://api.github.com/repos/$REPO/commits/$HEAD_SHA/check-runs" \
  > "$OUT/pr_checks.runs.json"
curl -sS "${HDR[@]}" "https://api.github.com/repos/$REPO/commits/$HEAD_SHA/status" \
  > "$OUT/pr_checks.statuses.json"

echo "Recorded fixtures into $OUT (repo=$REPO pr=$PR head=$HEAD_SHA)"
