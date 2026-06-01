//! Real `GitHubClient` using `octocrab` + raw `reqwest` for the diff endpoint.
//!
//! Auth refresh policy: on 401, run `gh auth token` once and
//! rebuild the octocrab client with the new token. If the refreshed token is
//! identical to the prior token (i.e. `gh` has nothing new to give us), surface
//! `Unauthorized` — the user must run `gh auth login`. A `Mutex` around the
//! refresh path prevents concurrent 401s from racing each other into multiple
//! `gh auth token` invocations.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Mutex as AsyncMutex;

use super::parse;
use super::{
    DashboardBucket, DashboardRequest, GitHubClient, GitHubError, NewLineComment,
    RateLimitSnapshot, SubmitReviewRequest,
};
use crate::data::auth::fetch_gh_token;
use crate::data::auth_probe::{GhTokenFetcher, TokenFetcher};
use crate::data::models::{MergeMethod, PrChecks, PrDetail, PrFiles, PrId, PrSummary, Repo};

const DASHBOARD_QUERY: &str = include_str!("graphql/dashboard.graphql");
const DASHBOARD_BUCKET_QUERY: &str = include_str!("graphql/dashboard_bucket.graphql");
const PR_LIST_QUERY: &str = include_str!("graphql/pr_list.graphql");
const PR_DETAIL_QUERY: &str = include_str!("graphql/pr_detail.graphql");
const PR_DETAIL_THREADS_PAGE_QUERY: &str = include_str!("graphql/pr_detail_threads_page.graphql");
const PR_DETAIL_COMMITS_PAGE_QUERY: &str = include_str!("graphql/pr_detail_commits_page.graphql");
const PR_DETAIL_TIMELINE_PAGE_QUERY: &str = include_str!("graphql/pr_detail_timeline_page.graphql");
const PR_DETAIL_THREAD_COMMENTS_PAGE_QUERY: &str =
    include_str!("graphql/pr_detail_thread_comments_page.graphql");

/// Hard cap on per-collection pagination iterations. A real PR with more
/// than 50 pages of any one collection (50 * page-size = 1000-2500 items)
/// is almost certainly a runaway loop; surface it as a parse error rather
/// than hanging the UI.
const PAGINATION_CAP: usize = 50;

/// GitHub search-query fragment for each bucket. Must match the `query:` strings
/// in `graphql/dashboard.graphql` so single-bucket pagination returns the same
/// set the full dashboard query paginates over.
fn bucket_search_query(bucket: DashboardBucket) -> &'static str {
    match bucket {
        DashboardBucket::ReviewRequested => "is:open is:pr review-requested:@me archived:false",
        DashboardBucket::Authored => "is:open is:pr author:@me archived:false",
        DashboardBucket::Assigned => "is:open is:pr assignee:@me archived:false",
    }
}

fn bucket_alias(bucket: DashboardBucket) -> &'static str {
    match bucket {
        DashboardBucket::ReviewRequested => "reviewRequested",
        DashboardBucket::Authored => "authored",
        DashboardBucket::Assigned => "assigned",
    }
}

fn map_graphql_error(e: octocrab::Error) -> GitHubError {
    match e {
        octocrab::Error::GitHub { source, .. } => classify_graphql_message(&source.message),
        // octocrab 0.51 surfaces logical GraphQL errors (HTTP 200 with a
        // top-level `errors` array — "Could not resolve to a node", rate-limit
        // exhaustion, etc.) as `Error::Graphql` rather than `Error::GitHub`.
        // Classify them with the same heuristics so callers see typed errors
        // (and the 401 refresh-and-retry path still fires) instead of a generic
        // network error.
        octocrab::Error::Graphql { source, .. } => classify_graphql_message(&source.to_string()),
        other => GitHubError::Network(other.to_string()),
    }
}

/// Classify a GitHub/GraphQL error message into a typed [`GitHubError`] using
/// the substrings GitHub uses across its REST and GraphQL surfaces.
fn classify_graphql_message(message: &str) -> GitHubError {
    let m = message.to_ascii_lowercase();
    if m.contains("401") || m.contains("unauthorized") || m.contains("bad credentials") {
        GitHubError::Unauthorized
    } else if m.contains("rate limit") {
        GitHubError::RateLimited(chrono::Utc::now())
    } else if m.contains("could not resolve")
        || m.contains("not resolve")
        || m.contains("not_found")
    {
        GitHubError::NotFound
    } else if m.contains("forbidden") {
        GitHubError::Forbidden(message.to_string())
    } else {
        GitHubError::Network(message.to_string())
    }
}

/// octocrab 0.38's `graphql()` returned the entire GraphQL document
/// (`{ "data": .., "errors": .. }`); octocrab 0.51 unwraps it and returns only
/// the contents of the `data` object. Every parser plus the rate-limit and
/// viewer-login helpers in this crate are written against the full envelope, so
/// re-attach a `data` wrapper around octocrab's response to restore that shape.
/// A successful call only ever carries `data` (octocrab converts an `errors`
/// payload into `Err(Error::Graphql)`), so this never masks server-side errors.
fn wrap_graphql_data(data: serde_json::Value) -> serde_json::Value {
    let mut obj = serde_json::Map::with_capacity(1);
    obj.insert("data".to_string(), data);
    serde_json::Value::Object(obj)
}

/// GraphQL responses return 200 with an `errors` array on logical failure.
/// Surface the first error message (used after mutations, which octocrab does
/// not treat as transport errors).
fn graphql_payload_error(response: &serde_json::Value) -> Option<GitHubError> {
    let errors = response.get("errors")?.as_array()?;
    let first = errors.first()?;
    let msg = first
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("GraphQL error")
        .to_string();
    let lower = msg.to_ascii_lowercase();
    Some(if lower.contains("rate limit") {
        GitHubError::RateLimited(chrono::Utc::now())
    } else if lower.contains("not resolve") || lower.contains("not_found") {
        GitHubError::NotFound
    } else {
        GitHubError::Validation(msg)
    })
}

/// Parse a `rateLimit { limit remaining resetAt }` selection from a GraphQL
/// response's `data` object. Returns `None` when the selection is absent.
fn rate_limit_from_graphql(response: &serde_json::Value) -> Option<RateLimitSnapshot> {
    let rl = response.get("data")?.get("rateLimit")?;
    let remaining = u32::try_from(rl.get("remaining")?.as_i64()?).ok()?;
    let limit = u32::try_from(rl.get("limit")?.as_i64()?).ok()?;
    let resets_at = chrono::DateTime::parse_from_rfc3339(rl.get("resetAt")?.as_str()?)
        .ok()?
        .with_timezone(&chrono::Utc);
    Some(RateLimitSnapshot {
        remaining,
        limit,
        resets_at,
    })
}

/// Parse the `x-ratelimit-*` headers GitHub attaches to every REST response.
fn rate_limit_from_headers(headers: &reqwest::header::HeaderMap) -> Option<RateLimitSnapshot> {
    let num = |key: &str| -> Option<i64> {
        headers
            .get(key)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok())
    };
    let remaining = u32::try_from(num("x-ratelimit-remaining")?).ok()?;
    let limit = u32::try_from(num("x-ratelimit-limit")?).ok()?;
    let resets_at = chrono::DateTime::from_timestamp(num("x-ratelimit-reset")?, 0)?;
    Some(RateLimitSnapshot {
        remaining,
        limit,
        resets_at,
    })
}

/// Pull GitHub's human-readable `message` out of a REST error body so typed
/// errors carry the server's verbatim text (surfaced on conflicts).
fn extract_api_message(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get("message")
        .and_then(serde_json::Value::as_str)
        .map(std::string::ToString::to_string)
}

fn merge_method_str(method: MergeMethod) -> &'static str {
    match method {
        MergeMethod::Merge => "merge",
        MergeMethod::Squash => "squash",
        MergeMethod::Rebase => "rebase",
    }
}

fn diff_side_str(side: crate::data::models::DiffSide) -> &'static str {
    match side {
        crate::data::models::DiffSide::Left => "LEFT",
        crate::data::models::DiffSide::Right => "RIGHT",
    }
}

#[derive(Debug, Clone)]
pub struct OctocrabConfig {
    pub timeout: Duration,
    pub user_agent: String,
}
impl Default for OctocrabConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            user_agent: "gpr/0.1.0".into(),
        }
    }
}

#[derive(Debug)]
struct AuthState {
    token: String,
    client: octocrab::Octocrab,
}

pub struct OctocrabGitHubClient {
    /// Single lock guards token + matching octocrab client so refresh is atomic.
    auth: RwLock<AuthState>,
    /// Serializes concurrent refreshes so a burst of 401s causes one `gh auth
    /// token` call, not N.
    refresh_lock: AsyncMutex<()>,
    http: reqwest::Client,
    rate_limit: RwLock<Option<RateLimitSnapshot>>,
    token_fetcher: Arc<dyn TokenFetcher>,
}

impl std::fmt::Debug for OctocrabGitHubClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OctocrabGitHubClient")
            .field("auth", &self.auth)
            .field("http", &self.http)
            .field("rate_limit", &self.rate_limit)
            .finish_non_exhaustive()
    }
}

impl OctocrabGitHubClient {
    pub async fn from_gh_cli(cfg: OctocrabConfig) -> Result<Self, GitHubError> {
        let token = fetch_gh_token()
            .await
            .map_err(|e| GitHubError::Network(e.to_string()))?;
        Self::with_token(token, cfg)
    }

    pub fn with_token(token: String, cfg: OctocrabConfig) -> Result<Self, GitHubError> {
        Self::with_token_and_fetcher(token, cfg, Arc::new(GhTokenFetcher))
    }

    /// Test-friendly constructor: inject a [`TokenFetcher`] so unit tests can
    /// exercise the 401 refresh-and-retry path without shelling out to `gh`.
    pub fn with_token_and_fetcher(
        token: String,
        cfg: OctocrabConfig,
        token_fetcher: Arc<dyn TokenFetcher>,
    ) -> Result<Self, GitHubError> {
        let client = Self::build_octocrab(&token)?;
        let http = reqwest::Client::builder()
            .user_agent(cfg.user_agent)
            .timeout(cfg.timeout)
            .build()
            .map_err(|e| GitHubError::Network(e.to_string()))?;
        Ok(Self {
            auth: RwLock::new(AuthState { token, client }),
            refresh_lock: AsyncMutex::new(()),
            http,
            rate_limit: RwLock::new(None),
            token_fetcher,
        })
    }

    fn build_octocrab(token: &str) -> Result<octocrab::Octocrab, GitHubError> {
        octocrab::Octocrab::builder()
            .personal_token(token.to_string())
            .build()
            .map_err(|e| GitHubError::Network(e.to_string()))
    }

    /// Snapshot the current octocrab client. Returned value owns its own
    /// `Octocrab` (cheap `Arc` clone internally) so callers can issue requests
    /// without holding any locks during await.
    fn current_client(&self) -> octocrab::Octocrab {
        self.auth.read().expect("auth lock poisoned").client.clone()
    }

    fn current_token(&self) -> String {
        self.auth.read().expect("auth lock poisoned").token.clone()
    }

    /// Run `gh auth token` and, if it returns a *different* token, rebuild the
    /// octocrab client so subsequent requests use the new auth. Held behind a
    /// dedicated `Mutex` so concurrent 401s collapse to one refresh.
    async fn refresh_token(&self) -> Result<(), GitHubError> {
        let _guard = self.refresh_lock.lock().await;
        let prior = self.current_token();
        let new = self
            .token_fetcher
            .fetch()
            .await
            .map_err(|e| GitHubError::Network(e.to_string()))?;
        if new == prior {
            return Err(GitHubError::Unauthorized);
        }
        let client = Self::build_octocrab(&new)?;
        let mut slot = self.auth.write().expect("auth lock poisoned");
        slot.token = new;
        slot.client = client;
        Ok(())
    }

    /// Map a `reqwest::Response` status to a typed error, leaving 2xx as `Ok`.
    /// Used by every raw-HTTP method to enforce a single error-translation path.
    fn map_http_status(status: reqwest::StatusCode) -> Result<(), GitHubError> {
        if status.is_success() {
            return Ok(());
        }
        Err(match status.as_u16() {
            401 => GitHubError::Unauthorized,
            403 => GitHubError::Forbidden(status.to_string()),
            404 => GitHubError::NotFound,
            409 => GitHubError::Conflict(status.to_string()),
            422 => GitHubError::Validation(status.to_string()),
            429 => GitHubError::RateLimited(chrono::Utc::now()),
            500..=599 => GitHubError::Server(status.as_u16()),
            other => GitHubError::Network(format!("unexpected status {other}")),
        })
    }

    /// Store the freshest rate-limit snapshot, if one was parsed.
    fn record_rate_limit(&self, snapshot: Option<RateLimitSnapshot>) {
        if let Some(snap) = snapshot
            && let Ok(mut slot) = self.rate_limit.write()
        {
            *slot = Some(snap);
        }
    }

    /// Translate a finished write response into `Ok(())` or a typed error
    /// carrying GitHub's verbatim `message` (consumes the body).
    async fn error_for_write(resp: reqwest::Response) -> Result<(), GitHubError> {
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let code = status.as_u16();
        let body = resp.text().await.unwrap_or_default();
        let msg = extract_api_message(&body).unwrap_or_else(|| status.to_string());
        Err(match code {
            401 => GitHubError::Unauthorized,
            403 => GitHubError::Forbidden(msg),
            404 => GitHubError::NotFound,
            409 => GitHubError::Conflict(msg),
            422 => GitHubError::Validation(msg),
            429 => GitHubError::RateLimited(chrono::Utc::now()),
            500..=599 => GitHubError::Server(code),
            other => GitHubError::Network(format!("unexpected status {other}: {msg}")),
        })
    }

    /// Issue an authenticated write (`POST`/`PATCH`/`PUT`) with a JSON body,
    /// capturing rate-limit headers and retrying once on a 401 after refresh.
    async fn send_write(
        &self,
        method: reqwest::Method,
        url: &str,
        json_body: &serde_json::Value,
    ) -> Result<(), GitHubError> {
        let attempt = || async {
            let token = self.current_token();
            let resp = self
                .http
                .request(method.clone(), url)
                .bearer_auth(&token)
                .header("Accept", "application/vnd.github+json")
                .json(json_body)
                .send()
                .await
                .map_err(|e| GitHubError::Network(e.to_string()))?;
            self.record_rate_limit(rate_limit_from_headers(resp.headers()));
            Self::error_for_write(resp).await
        };
        match attempt().await {
            Err(GitHubError::Unauthorized) => {
                self.refresh_token().await?;
                attempt().await
            }
            other => other,
        }
    }

    async fn fetch_pr_detail_first_page(
        &self,
        owner: &str,
        name: &str,
        number: u64,
    ) -> Result<parse::pr_detail::PrDetailPage, GitHubError> {
        let attempt = || async {
            let client = self.current_client();
            let body = serde_json::json!({
                "query": PR_DETAIL_QUERY,
                "variables": {
                    "owner": owner,
                    "name": name,
                    "number": number,
                    "afterThreads": serde_json::Value::Null,
                    "afterCommits": serde_json::Value::Null,
                    "afterTimeline": serde_json::Value::Null,
                },
            });
            let response =
                wrap_graphql_data(client.graphql(&body).await.map_err(map_graphql_error)?);
            self.record_rate_limit(rate_limit_from_graphql(&response));
            parse::pr_detail::decode_first_page(&response)
        };
        match attempt().await {
            Err(GitHubError::Unauthorized) => {
                self.refresh_token().await?;
                attempt().await
            }
            other => other,
        }
    }

    async fn drain_commits(
        &self,
        owner: &str,
        name: &str,
        number: u64,
        mut cursor: Option<String>,
        acc: &mut Vec<crate::data::models::TimelineEvent>,
    ) -> Result<(), GitHubError> {
        let mut iters = 0_usize;
        while let Some(c) = cursor {
            iters += 1;
            if iters > PAGINATION_CAP {
                return Err(GitHubError::Parse(
                    "commits pagination exceeded 50 iterations".into(),
                ));
            }
            let next = self.fetch_commits_page(owner, name, number, &c).await?;
            acc.extend(next.commits);
            cursor = next.cursor;
        }
        Ok(())
    }

    async fn drain_timeline(
        &self,
        owner: &str,
        name: &str,
        number: u64,
        mut cursor: Option<String>,
        acc: &mut Vec<crate::data::models::TimelineEvent>,
    ) -> Result<(), GitHubError> {
        let mut iters = 0_usize;
        while let Some(c) = cursor {
            iters += 1;
            if iters > PAGINATION_CAP {
                return Err(GitHubError::Parse(
                    "timeline pagination exceeded 50 iterations".into(),
                ));
            }
            let next = self.fetch_timeline_page(owner, name, number, &c).await?;
            acc.extend(next.events);
            cursor = next.cursor;
        }
        Ok(())
    }

    async fn drain_threads(
        &self,
        owner: &str,
        name: &str,
        number: u64,
        mut cursor: Option<String>,
        acc: &mut Vec<crate::data::models::ReviewThread>,
        continuations: &mut Vec<(String, String)>,
    ) -> Result<(), GitHubError> {
        let mut iters = 0_usize;
        while let Some(c) = cursor {
            iters += 1;
            if iters > PAGINATION_CAP {
                return Err(GitHubError::Parse(
                    "review threads pagination exceeded 50 iterations".into(),
                ));
            }
            let mut next = self.fetch_threads_page(owner, name, number, &c).await?;
            continuations.append(&mut next.thread_comment_continuations);
            acc.extend(next.threads);
            cursor = next.cursor;
        }
        Ok(())
    }

    async fn drain_thread_comments(
        &self,
        continuations: &[(String, String)],
        threads: &mut [crate::data::models::ReviewThread],
    ) -> Result<(), GitHubError> {
        for (thread_id, initial_cursor) in continuations {
            let mut cursor = Some(initial_cursor.clone());
            let mut iters = 0_usize;
            while let Some(c) = cursor {
                iters += 1;
                if iters > PAGINATION_CAP {
                    return Err(GitHubError::Parse(format!(
                        "thread {thread_id} comments pagination exceeded 50 iterations"
                    )));
                }
                let (more, next_cursor) = self.fetch_thread_comments_page(thread_id, &c).await?;
                if let Some(t) = threads.iter_mut().find(|t| &t.id == thread_id) {
                    t.comments.extend(more);
                }
                cursor = next_cursor;
            }
        }
        Ok(())
    }

    async fn fetch_commits_page(
        &self,
        owner: &str,
        name: &str,
        number: u64,
        after: &str,
    ) -> Result<parse::pr_detail::PagedCommits, GitHubError> {
        let attempt = || async {
            let client = self.current_client();
            let body = serde_json::json!({
                "query": PR_DETAIL_COMMITS_PAGE_QUERY,
                "variables": { "owner": owner, "name": name, "number": number, "after": after },
            });
            let response =
                wrap_graphql_data(client.graphql(&body).await.map_err(map_graphql_error)?);
            parse::pr_detail::decode_commits_page(&response)
        };
        match attempt().await {
            Err(GitHubError::Unauthorized) => {
                self.refresh_token().await?;
                attempt().await
            }
            other => other,
        }
    }

    async fn fetch_timeline_page(
        &self,
        owner: &str,
        name: &str,
        number: u64,
        after: &str,
    ) -> Result<parse::pr_detail::PagedTimeline, GitHubError> {
        let attempt = || async {
            let client = self.current_client();
            let body = serde_json::json!({
                "query": PR_DETAIL_TIMELINE_PAGE_QUERY,
                "variables": { "owner": owner, "name": name, "number": number, "after": after },
            });
            let response =
                wrap_graphql_data(client.graphql(&body).await.map_err(map_graphql_error)?);
            parse::pr_detail::decode_timeline_page(&response)
        };
        match attempt().await {
            Err(GitHubError::Unauthorized) => {
                self.refresh_token().await?;
                attempt().await
            }
            other => other,
        }
    }

    async fn fetch_threads_page(
        &self,
        owner: &str,
        name: &str,
        number: u64,
        after: &str,
    ) -> Result<parse::pr_detail::PagedThreads, GitHubError> {
        let attempt = || async {
            let client = self.current_client();
            let body = serde_json::json!({
                "query": PR_DETAIL_THREADS_PAGE_QUERY,
                "variables": { "owner": owner, "name": name, "number": number, "after": after },
            });
            let response =
                wrap_graphql_data(client.graphql(&body).await.map_err(map_graphql_error)?);
            parse::pr_detail::decode_threads_page(&response)
        };
        match attempt().await {
            Err(GitHubError::Unauthorized) => {
                self.refresh_token().await?;
                attempt().await
            }
            other => other,
        }
    }

    async fn fetch_thread_comments_page(
        &self,
        thread_id: &str,
        after: &str,
    ) -> Result<(Vec<crate::data::models::ReviewComment>, Option<String>), GitHubError> {
        let attempt = || async {
            let client = self.current_client();
            let body = serde_json::json!({
                "query": PR_DETAIL_THREAD_COMMENTS_PAGE_QUERY,
                "variables": { "threadId": thread_id, "after": after },
            });
            let response =
                wrap_graphql_data(client.graphql(&body).await.map_err(map_graphql_error)?);
            parse::pr_detail::decode_thread_comments_page(&response)
        };
        match attempt().await {
            Err(GitHubError::Unauthorized) => {
                self.refresh_token().await?;
                attempt().await
            }
            other => other,
        }
    }
}

#[async_trait]
impl GitHubClient for OctocrabGitHubClient {
    async fn viewer_login(&self) -> Result<String, GitHubError> {
        let attempt = || async {
            let client = self.current_client();
            let v = wrap_graphql_data(
                client
                    .graphql(&serde_json::json!({"query": "query { viewer { login } }"}))
                    .await
                    .map_err(|e| match e {
                        octocrab::Error::GitHub { source, .. }
                            if source.message.contains("401") =>
                        {
                            GitHubError::Unauthorized
                        }
                        other => GitHubError::Network(other.to_string()),
                    })?,
            );
            v["data"]["viewer"]["login"]
                .as_str()
                .map(std::string::ToString::to_string)
                .ok_or_else(|| GitHubError::Parse("viewer.login missing".into()))
        };
        match attempt().await {
            Err(GitHubError::Unauthorized) => {
                self.refresh_token().await?;
                attempt().await
            }
            other => other,
        }
    }

    async fn auth_scopes(&self) -> Result<Vec<String>, GitHubError> {
        let attempt = || async {
            let token = self.current_token();
            let resp = self
                .http
                .get("https://api.github.com/user")
                .bearer_auth(&token)
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
                .map_err(|e| GitHubError::Network(e.to_string()))?;
            Self::map_http_status(resp.status())?;
            let scopes = resp
                .headers()
                .get("x-oauth-scopes")
                .and_then(|h| h.to_str().ok())
                .map(|s| {
                    s.split(',')
                        .map(|x| x.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            Ok::<_, GitHubError>(scopes)
        };
        match attempt().await {
            Err(GitHubError::Unauthorized) => {
                self.refresh_token().await?;
                attempt().await
            }
            other => other,
        }
    }

    async fn dashboard(
        &self,
        request: DashboardRequest,
    ) -> Result<Vec<(DashboardBucket, Vec<PrSummary>, Option<String>)>, GitHubError> {
        let attempt = || async {
            let client = self.current_client();
            let response = wrap_graphql_data(match request.bucket {
                None => client
                    .graphql(&serde_json::json!({ "query": DASHBOARD_QUERY }))
                    .await
                    .map_err(map_graphql_error)?,
                Some(bucket) => {
                    let alias = bucket_alias(bucket);
                    let search = bucket_search_query(bucket);
                    let query = DASHBOARD_BUCKET_QUERY
                        .replace("__BUCKET__", alias)
                        .replace("__SEARCH__", search);
                    let body = serde_json::json!({
                        "query": query,
                        "variables": { "after": request.cursor },
                    });
                    client.graphql(&body).await.map_err(map_graphql_error)?
                }
            });
            self.record_rate_limit(rate_limit_from_graphql(&response));
            let mut decoded = parse::dashboard::decode(&response)?;
            if let Some(bucket) = request.bucket {
                decoded.retain(|(b, _, _)| *b == bucket);
            }
            Ok::<_, GitHubError>(decoded)
        };
        match attempt().await {
            Err(GitHubError::Unauthorized) => {
                self.refresh_token().await?;
                attempt().await
            }
            other => other,
        }
    }
    async fn pr_list(
        &self,
        repo: &Repo,
        cursor: Option<String>,
        filter: Option<crate::data::github::PrListFilter>,
        sort: Option<crate::data::github::PrListSort>,
    ) -> Result<(Vec<PrSummary>, Option<String>), GitHubError> {
        let attempt = || async {
            let client = self.current_client();
            let body = serde_json::json!({
                "query": PR_LIST_QUERY,
                "variables": {
                    "owner": repo.owner,
                    "name": repo.name,
                    "cursor": cursor,
                },
            });
            let response =
                wrap_graphql_data(client.graphql(&body).await.map_err(map_graphql_error)?);
            self.record_rate_limit(rate_limit_from_graphql(&response));
            parse::pr_list::decode(&response)
        };
        let (mut rows, next) = match attempt().await {
            Err(GitHubError::Unauthorized) => {
                self.refresh_token().await?;
                attempt().await
            }
            other => other,
        }?;
        if let Some(f) = filter {
            let viewer = self.viewer_login().await.unwrap_or_default();
            rows.retain(|pr| apply_pr_list_filter_real(f, pr, &viewer));
        }
        rows = apply_pr_list_sort_real(sort, rows);
        Ok((rows, next))
    }
    async fn pr_detail(&self, id: &PrId) -> Result<PrDetail, GitHubError> {
        let owner = id.repo.owner.clone();
        let name = id.repo.name.clone();
        let number = id.number;

        let mut page0 = self
            .fetch_pr_detail_first_page(&owner, &name, number)
            .await?;

        let mut accumulated_commits = std::mem::take(&mut page0.commits_page.commits);
        self.drain_commits(
            &owner,
            &name,
            number,
            page0.commits_page.cursor.take(),
            &mut accumulated_commits,
        )
        .await?;

        let mut accumulated_timeline = std::mem::take(&mut page0.timeline_page.events);
        self.drain_timeline(
            &owner,
            &name,
            number,
            page0.timeline_page.cursor.take(),
            &mut accumulated_timeline,
        )
        .await?;

        let mut accumulated_threads = std::mem::take(&mut page0.review_threads_page.threads);
        let mut continuations =
            std::mem::take(&mut page0.review_threads_page.thread_comment_continuations);
        self.drain_threads(
            &owner,
            &name,
            number,
            page0.review_threads_page.cursor.take(),
            &mut accumulated_threads,
            &mut continuations,
        )
        .await?;

        self.drain_thread_comments(&continuations, &mut accumulated_threads)
            .await?;

        // Insertion order does not matter — downstream reducers sort by
        // timestamp before render — so we simply concat commits + the rest.
        let timeline = accumulated_commits
            .into_iter()
            .chain(accumulated_timeline)
            .collect();

        Ok(PrDetail {
            summary: page0.summary,
            body: page0.body,
            body_html: page0.body_html,
            requested_reviewers: page0.requested_reviewers,
            assignees: page0.assignees,
            head_sha: page0.head_sha,
            merge_commit_sha: page0.merge_commit_sha,
            base_ref_oid: page0.base_ref_oid,
            review_threads: accumulated_threads,
            timeline,
            available_merge_methods: page0.available_merge_methods,
            // version=0 is the freshly-fetched marker; the cache layer
            // bumps to a monotonic value on put.
            version: 0,
        })
    }
    async fn pr_files(&self, id: &PrId, _head_sha: &str) -> Result<PrFiles, GitHubError> {
        // REST `/pulls/N/files` paginates 100 per page; GitHub documents a
        // 3000-file cap on what it will return. The defensive `MAX_PAGES`
        // guard turns runaway servers into a clean truncation.
        const PAGE_SIZE: usize = 100;
        const MAX_FILES: usize = 3000;
        const MAX_PAGES: u32 = 30;

        let owner = &id.repo.owner;
        let name = &id.repo.name;
        let number = id.number;
        let url = format!("https://api.github.com/repos/{owner}/{name}/pulls/{number}/files");

        let mut acc: Vec<crate::data::models::FileDiff> = Vec::new();
        let mut truncated = false;
        let mut page: u32 = 1;
        loop {
            let fetch = || async {
                let token = self.current_token();
                let resp = self
                    .http
                    .get(&url)
                    .bearer_auth(&token)
                    .header("Accept", "application/vnd.github+json")
                    .query(&[
                        ("per_page", PAGE_SIZE.to_string()),
                        ("page", page.to_string()),
                    ])
                    .send()
                    .await
                    .map_err(|e| GitHubError::Network(e.to_string()))?;
                self.record_rate_limit(rate_limit_from_headers(resp.headers()));
                Self::map_http_status(resp.status())?;
                let body: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| GitHubError::Network(e.to_string()))?;
                parse::pr_files::decode_page(&body)
            };
            let decoded = match fetch().await {
                Err(GitHubError::Unauthorized) => {
                    self.refresh_token().await?;
                    fetch().await?
                }
                other => other?,
            };
            let got = decoded.len();
            if got == 0 {
                break;
            }
            acc.extend(decoded);
            if acc.len() >= MAX_FILES {
                acc.truncate(MAX_FILES);
                truncated = true;
                break;
            }
            if got < PAGE_SIZE {
                break;
            }
            page += 1;
            if page > MAX_PAGES {
                break;
            }
        }
        Ok(PrFiles {
            files: acc,
            truncated,
        })
    }
    async fn pr_file_diff(
        &self,
        id: &PrId,
        _head_sha: &str,
        path: &str,
    ) -> Result<crate::data::models::FileDiff, GitHubError> {
        // GitHub's `/files` endpoint cannot filter by path, so page through
        // it and return the first entry matching `path`.
        const PAGE_SIZE: usize = 100;
        const MAX_PAGES: u32 = 30;
        let owner = &id.repo.owner;
        let name = &id.repo.name;
        let number = id.number;
        let url = format!("https://api.github.com/repos/{owner}/{name}/pulls/{number}/files");
        let mut page: u32 = 1;
        loop {
            let fetch = || async {
                let token = self.current_token();
                let resp = self
                    .http
                    .get(&url)
                    .bearer_auth(&token)
                    .header("Accept", "application/vnd.github+json")
                    .query(&[
                        ("per_page", PAGE_SIZE.to_string()),
                        ("page", page.to_string()),
                    ])
                    .send()
                    .await
                    .map_err(|e| GitHubError::Network(e.to_string()))?;
                self.record_rate_limit(rate_limit_from_headers(resp.headers()));
                Self::map_http_status(resp.status())?;
                let body: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| GitHubError::Network(e.to_string()))?;
                parse::pr_files::decode_page(&body)
            };
            let decoded = match fetch().await {
                Err(GitHubError::Unauthorized) => {
                    self.refresh_token().await?;
                    fetch().await?
                }
                other => other?,
            };
            let got = decoded.len();
            if let Some(file) = decoded.into_iter().find(|f| f.path == path) {
                return Ok(file);
            }
            if got < PAGE_SIZE {
                break;
            }
            page += 1;
            if page > MAX_PAGES {
                break;
            }
        }
        Err(GitHubError::NotFound)
    }
    async fn pr_diff(&self, id: &PrId, _head_sha: &str) -> Result<String, GitHubError> {
        let owner = &id.repo.owner;
        let name = &id.repo.name;
        let number = id.number;
        let url = format!("https://api.github.com/repos/{owner}/{name}/pulls/{number}");

        let fetch = || async {
            let token = self.current_token();
            let resp = self
                .http
                .get(&url)
                .bearer_auth(&token)
                .header("Accept", "application/vnd.github.v3.diff")
                .send()
                .await
                .map_err(|e| GitHubError::Network(e.to_string()))?;
            self.record_rate_limit(rate_limit_from_headers(resp.headers()));
            Self::map_http_status(resp.status())?;
            resp.text()
                .await
                .map_err(|e| GitHubError::Network(e.to_string()))
        };
        match fetch().await {
            Err(GitHubError::Unauthorized) => {
                self.refresh_token().await?;
                fetch().await
            }
            other => other,
        }
    }
    async fn pr_checks(&self, id: &PrId) -> Result<PrChecks, GitHubError> {
        // v0.1: single page (default per_page=30). Pagination deferred to v1.x.
        let owner = &id.repo.owner;
        let name = &id.repo.name;
        let number = id.number;
        let runs_url = format!(
            "https://api.github.com/repos/{owner}/{name}/commits/refs%2Fpull%2F{number}%2Fhead/check-runs"
        );
        let statuses_url = format!(
            "https://api.github.com/repos/{owner}/{name}/commits/refs%2Fpull%2F{number}%2Fhead/status"
        );

        let fetch_runs = || async {
            let attempt = || async {
                let token = self.current_token();
                let resp = self
                    .http
                    .get(&runs_url)
                    .bearer_auth(&token)
                    .header("Accept", "application/vnd.github+json")
                    .send()
                    .await
                    .map_err(|e| GitHubError::Network(e.to_string()))?;
                self.record_rate_limit(rate_limit_from_headers(resp.headers()));
                Self::map_http_status(resp.status())?;
                let body: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| GitHubError::Network(e.to_string()))?;
                parse::pr_checks::decode_runs(&body)
            };
            match attempt().await {
                Err(GitHubError::Unauthorized) => {
                    self.refresh_token().await?;
                    attempt().await
                }
                other => other,
            }
        };

        let fetch_statuses = || async {
            let attempt = || async {
                let token = self.current_token();
                let resp = self
                    .http
                    .get(&statuses_url)
                    .bearer_auth(&token)
                    .header("Accept", "application/vnd.github+json")
                    .send()
                    .await
                    .map_err(|e| GitHubError::Network(e.to_string()))?;
                self.record_rate_limit(rate_limit_from_headers(resp.headers()));
                Self::map_http_status(resp.status())?;
                let body: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| GitHubError::Network(e.to_string()))?;
                parse::pr_checks::decode_statuses(&body)
            };
            match attempt().await {
                Err(GitHubError::Unauthorized) => {
                    self.refresh_token().await?;
                    attempt().await
                }
                other => other,
            }
        };

        let (runs, statuses) = tokio::try_join!(fetch_runs(), fetch_statuses())?;
        Ok(PrChecks { runs, statuses })
    }

    async fn submit_review(&self, request: SubmitReviewRequest) -> Result<(), GitHubError> {
        use crate::data::models::ReviewState;
        let event = match request.state {
            ReviewState::Approved => "APPROVE",
            ReviewState::ChangesRequested => "REQUEST_CHANGES",
            _ => "COMMENT",
        };
        let comments: Vec<serde_json::Value> = request
            .line_comments
            .iter()
            .map(|c| {
                let mut obj = serde_json::json!({
                    "path": c.path,
                    "body": c.body,
                    "line": c.line,
                    "side": diff_side_str(c.side),
                });
                if let Some(sl) = c.start_line {
                    obj["start_line"] = sl.into();
                }
                if let Some(ss) = c.start_side {
                    obj["start_side"] = diff_side_str(ss).into();
                }
                obj
            })
            .collect();
        let body = serde_json::json!({
            "commit_id": request.head_sha,
            "event": event,
            "body": request.body,
            "comments": comments,
        });
        let url = format!(
            "https://api.github.com/repos/{}/{}/pulls/{}/reviews",
            request.id.repo.owner, request.id.repo.name, request.id.number
        );
        self.send_write(reqwest::Method::POST, &url, &body).await
    }
    async fn post_pr_comment(&self, id: &PrId, body: String) -> Result<(), GitHubError> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/comments",
            id.repo.owner, id.repo.name, id.number
        );
        let payload = serde_json::json!({ "body": body });
        self.send_write(reqwest::Method::POST, &url, &payload).await
    }
    async fn post_line_comment(
        &self,
        id: &PrId,
        head_sha: &str,
        comment: NewLineComment,
    ) -> Result<(), GitHubError> {
        let mut payload = serde_json::json!({
            "body": comment.body,
            "commit_id": head_sha,
            "path": comment.path,
            "line": comment.line,
            "side": diff_side_str(comment.side),
        });
        if let Some(sl) = comment.start_line {
            payload["start_line"] = sl.into();
        }
        if let Some(ss) = comment.start_side {
            payload["start_side"] = diff_side_str(ss).into();
        }
        let url = format!(
            "https://api.github.com/repos/{}/{}/pulls/{}/comments",
            id.repo.owner, id.repo.name, id.number
        );
        self.send_write(reqwest::Method::POST, &url, &payload).await
    }
    async fn reply_to_thread(
        &self,
        _id: &PrId,
        thread_id: &str,
        body: String,
    ) -> Result<(), GitHubError> {
        const MUTATION: &str = "mutation($threadId: ID!, $body: String!) { \
            addPullRequestReviewThreadReply(input: { pullRequestReviewThreadId: $threadId, body: $body }) \
            { comment { id } } }";
        let payload = serde_json::json!({
            "query": MUTATION,
            "variables": { "threadId": thread_id, "body": body },
        });
        let attempt = || async {
            let client = self.current_client();
            let response =
                wrap_graphql_data(client.graphql(&payload).await.map_err(map_graphql_error)?);
            self.record_rate_limit(rate_limit_from_graphql(&response));
            if let Some(e) = graphql_payload_error(&response) {
                return Err(e);
            }
            Ok::<(), GitHubError>(())
        };
        match attempt().await {
            Err(GitHubError::Unauthorized) => {
                self.refresh_token().await?;
                attempt().await
            }
            other => other,
        }
    }
    async fn merge(
        &self,
        id: &PrId,
        method: MergeMethod,
        head_sha: &str,
    ) -> Result<(), GitHubError> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/pulls/{}/merge",
            id.repo.owner, id.repo.name, id.number
        );
        let payload = serde_json::json!({
            "merge_method": merge_method_str(method),
            "sha": head_sha,
        });
        self.send_write(reqwest::Method::PUT, &url, &payload).await
    }
    async fn close(&self, id: &PrId) -> Result<(), GitHubError> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/pulls/{}",
            id.repo.owner, id.repo.name, id.number
        );
        let payload = serde_json::json!({ "state": "closed" });
        self.send_write(reqwest::Method::PATCH, &url, &payload)
            .await
    }
    async fn reopen(&self, id: &PrId) -> Result<(), GitHubError> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/pulls/{}",
            id.repo.owner, id.repo.name, id.number
        );
        let payload = serde_json::json!({ "state": "open" });
        self.send_write(reqwest::Method::PATCH, &url, &payload)
            .await
    }

    fn rate_limit(&self) -> Option<RateLimitSnapshot> {
        self.rate_limit.read().ok().and_then(|g| g.clone())
    }
}

fn apply_pr_list_filter_real(
    filter: crate::data::github::PrListFilter,
    pr: &PrSummary,
    viewer: &str,
) -> bool {
    use crate::data::github::PrListFilter;
    use crate::data::models::PrState;
    match filter {
        PrListFilter::Open => matches!(pr.state, PrState::Open),
        PrListFilter::Closed => matches!(pr.state, PrState::Closed),
        PrListFilter::Merged => matches!(pr.state, PrState::Merged),
        PrListFilter::AuthoredByMe => pr.author == viewer,
        PrListFilter::NotDraft => !pr.is_draft,
    }
}

fn apply_pr_list_sort_real(
    sort: Option<crate::data::github::PrListSort>,
    mut prs: Vec<PrSummary>,
) -> Vec<PrSummary> {
    use crate::data::github::PrListSort;
    let Some(sort) = sort else {
        return prs;
    };
    match sort {
        PrListSort::UpdatedDesc => prs.sort_by_key(|pr| std::cmp::Reverse(pr.updated_at)),
        PrListSort::UpdatedAsc => prs.sort_by_key(|pr| pr.updated_at),
        PrListSort::CreatedDesc => prs.sort_by_key(|pr| std::cmp::Reverse(pr.created_at)),
        PrListSort::CommentsDesc => prs.sort_by_key(|pr| std::cmp::Reverse(pr.comments)),
    }
    prs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_is_ten_second_timeout() {
        assert_eq!(OctocrabConfig::default().timeout, Duration::from_secs(10));
    }

    #[test]
    fn with_token_constructs() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            OctocrabGitHubClient::with_token("token".into(), OctocrabConfig::default()).unwrap();
        });
    }

    #[test]
    fn map_http_status_classifies_documented_codes() {
        use reqwest::StatusCode;
        assert!(OctocrabGitHubClient::map_http_status(StatusCode::OK).is_ok());
        assert!(matches!(
            OctocrabGitHubClient::map_http_status(StatusCode::UNAUTHORIZED),
            Err(GitHubError::Unauthorized)
        ));
        assert!(matches!(
            OctocrabGitHubClient::map_http_status(StatusCode::NOT_FOUND),
            Err(GitHubError::NotFound)
        ));
        assert!(matches!(
            OctocrabGitHubClient::map_http_status(StatusCode::INTERNAL_SERVER_ERROR),
            Err(GitHubError::Server(500))
        ));
        assert!(matches!(
            OctocrabGitHubClient::map_http_status(StatusCode::CONFLICT),
            Err(GitHubError::Conflict(_))
        ));
    }

    use crate::data::auth::AuthError;

    struct StaticFetcher(String);

    #[async_trait]
    impl TokenFetcher for StaticFetcher {
        async fn fetch(&self) -> Result<String, AuthError> {
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn refresh_token_swaps_to_new_token_returned_by_injected_fetcher() {
        let client = OctocrabGitHubClient::with_token_and_fetcher(
            "old".into(),
            OctocrabConfig::default(),
            Arc::new(StaticFetcher("new".into())),
        )
        .unwrap();
        assert_eq!(client.current_token(), "old");
        client.refresh_token().await.expect("refresh ok");
        assert_eq!(client.current_token(), "new");
    }

    #[tokio::test]
    async fn refresh_token_returns_unauthorized_when_fetcher_returns_same_token() {
        let client = OctocrabGitHubClient::with_token_and_fetcher(
            "same".into(),
            OctocrabConfig::default(),
            Arc::new(StaticFetcher("same".into())),
        )
        .unwrap();
        let err = client.refresh_token().await.unwrap_err();
        assert!(matches!(err, GitHubError::Unauthorized), "got {err:?}");
        assert_eq!(client.current_token(), "same");
    }

    #[test]
    fn rate_limit_from_graphql_parses_data_block() {
        let v = serde_json::json!({
            "data": { "rateLimit": { "limit": 5000, "remaining": 4990, "resetAt": "2030-01-01T00:00:00Z" } }
        });
        let rl = rate_limit_from_graphql(&v).expect("parsed");
        assert_eq!(rl.remaining, 4990);
        assert_eq!(rl.limit, 5000);
        assert!(rate_limit_from_graphql(&serde_json::json!({"data": {}})).is_none());
    }

    #[test]
    fn rate_limit_from_headers_parses_rest_headers() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert("x-ratelimit-remaining", "4990".parse().unwrap());
        h.insert("x-ratelimit-limit", "5000".parse().unwrap());
        h.insert("x-ratelimit-reset", "1900000000".parse().unwrap());
        let rl = rate_limit_from_headers(&h).expect("parsed");
        assert_eq!(rl.remaining, 4990);
        assert_eq!(rl.limit, 5000);
        assert!(rate_limit_from_headers(&reqwest::header::HeaderMap::new()).is_none());
    }

    #[test]
    fn extract_api_message_pulls_message_field() {
        assert_eq!(
            extract_api_message(r#"{"message":"Required status check is pending"}"#).as_deref(),
            Some("Required status check is pending")
        );
        assert!(extract_api_message("not json").is_none());
    }

    #[test]
    fn graphql_payload_error_classifies_errors_array() {
        let not_found =
            serde_json::json!({ "errors": [{ "message": "Could not resolve to a node" }] });
        assert!(matches!(
            graphql_payload_error(&not_found),
            Some(GitHubError::NotFound)
        ));
        let ok = serde_json::json!({ "data": { "x": 1 } });
        assert!(graphql_payload_error(&ok).is_none());
    }

    #[test]
    fn method_and_side_strings() {
        assert_eq!(merge_method_str(MergeMethod::Squash), "squash");
        assert_eq!(diff_side_str(crate::data::models::DiffSide::Left), "LEFT");
    }

    // Regression: octocrab 0.51's `graphql()` returns only the contents of the
    // GraphQL `data` object, whereas octocrab 0.38 (and every parser in this
    // crate) expected the whole `{ "data": .. }` envelope. Without
    // `wrap_graphql_data`, every GraphQL-backed fetch failed with a
    // "data missing" parse error and surfaced a failure toast. This test pins
    // the contract: stripping the envelope (as 0.51 does) and re-wrapping it
    // round-trips through the parser, while the stripped value alone does not.
    #[test]
    fn wrap_graphql_data_restores_envelope_octocrab_051_strips() {
        const FULL: &str = include_str!("../../../tests/fixtures/api/dashboard.graphql.json");
        let full: serde_json::Value = serde_json::from_str(FULL).expect("fixture parses");
        let stripped = full
            .get("data")
            .cloned()
            .expect("recorded fixture carries a data envelope");

        // What octocrab 0.51 now hands us: the `data` contents with no envelope.
        // The parser cannot find `data` and reports a parse error — the bug.
        let err = parse::dashboard::decode(&stripped).expect_err("stripped data must not decode");
        assert!(matches!(err, GitHubError::Parse(_)), "got {err:?}");

        // Re-attaching the envelope makes the parser succeed again.
        let rewrapped = wrap_graphql_data(stripped);
        let decoded = parse::dashboard::decode(&rewrapped).expect("re-wrapped data decodes");
        assert_eq!(decoded.len(), 3, "three dashboard buckets");
    }

    // octocrab 0.51 surfaces logical GraphQL errors as `Error::Graphql`, which
    // `map_graphql_error` routes through `classify_graphql_message`. Pin the
    // typed mapping so a "Could not resolve" / rate-limit / bad-credentials
    // GraphQL error keeps producing the right `GitHubError` (and, for 401, lets
    // the refresh-and-retry path fire) rather than a generic network error.
    #[test]
    fn classify_graphql_message_maps_known_substrings() {
        assert!(matches!(
            classify_graphql_message("Could not resolve to a node with the global id"),
            GitHubError::NotFound
        ));
        assert!(matches!(
            classify_graphql_message("API rate limit exceeded for user"),
            GitHubError::RateLimited(_)
        ));
        assert!(matches!(
            classify_graphql_message("Bad credentials"),
            GitHubError::Unauthorized
        ));
        assert!(matches!(
            classify_graphql_message("something unexpected"),
            GitHubError::Network(_)
        ));
    }
}
