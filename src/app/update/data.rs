use crate::data::models::PrId;

use super::super::{
    effect::{DataEvent, Effect},
    state::{AppState, Selection, Toast, ToastKind},
};
use super::{RATE_LIMIT_CRITICAL_THRESHOLD, RATE_LIMIT_LOW_THRESHOLD, push_data_loaded_cue};

pub(super) fn handle_data_event(state: &mut AppState, data: &DataEvent) -> Vec<Effect> {
    match data {
        DataEvent::DashboardLoaded(Ok(payload)) => {
            absorb_dashboard_ok(state, payload);
            Vec::new()
        }
        DataEvent::DashboardLoaded(Err(e)) => {
            // Release any in-flight pagination markers so a failed page does
            // not wedge "load more" for the rest of the session.
            state.dashboard_loading.clear();
            state.toast = Some(Toast {
                kind: ToastKind::Error,
                message: format!("dashboard fetch failed: {e}"),
                created_at: std::time::Instant::now(),
            });
            Vec::new()
        }
        DataEvent::PrListLoaded {
            repo,
            result: Ok(rows),
            next_cursor,
            replace,
        } => {
            absorb_pr_list_ok(state, repo, rows, next_cursor.as_deref(), *replace);
            Vec::new()
        }
        DataEvent::PrListLoaded {
            repo,
            result: Err(e),
            ..
        } => {
            state.pr_list_loading.remove(repo);
            state.pr_list_paginating.remove(repo);
            state.toast = Some(Toast {
                kind: ToastKind::Error,
                message: format!(
                    "pr_list fetch failed for {}/{}: {e} — press R to retry",
                    repo.owner, repo.name
                ),
                created_at: std::time::Instant::now(),
            });
            Vec::new()
        }
        DataEvent::PrDetailLoaded { id, result } => {
            absorb_pr_detail(state, id, result.as_ref().as_ref())
        }
        DataEvent::PrFilesLoaded { id, result, .. } => {
            absorb_pr_files(state, id, result.as_ref().as_ref());
            Vec::new()
        }
        DataEvent::PrChecksLoaded { id, result } => {
            absorb_pr_checks(state, id, result.as_ref());
            Vec::new()
        }
        DataEvent::PrFileDiffLoaded {
            id,
            path,
            result,
            head_sha: _,
        } => {
            state
                .pr_file_diff_inflight
                .remove(&(id.clone(), path.clone()));
            match result.as_ref().as_ref() {
                Ok(file_diff) => {
                    if let Some(files) = state.pr_files.get_mut(id)
                        && let Some(slot) = files.files.iter_mut().find(|f| f.path == *path)
                    {
                        *slot = file_diff.clone();
                    }
                }
                Err(e) => fetch_failed_toast(state, "pr_file_diff", id, e),
            }
            Vec::new()
        }
        DataEvent::WriteCompleted {
            kind,
            result: Ok(()),
        } => super::write::handle_write_completed_ok(state, kind),
        DataEvent::WriteCompleted {
            kind,
            result: Err(e),
        } => super::write::handle_write_completed_err(state, kind, e),
        DataEvent::PrDiffLoaded { .. } => Vec::new(),
        DataEvent::RateLimitUpdate(snapshot) => handle_rate_limit_update(state, snapshot),
        DataEvent::CheckoutCompleted { id, result } => {
            super::write::handle_checkout_completed(state, id, result)
        }
    }
}

pub(super) fn handle_rate_limit_update(
    state: &mut AppState,
    snapshot: &crate::data::github::RateLimitSnapshot,
) -> Vec<Effect> {
    let remaining = snapshot.remaining;
    state.rate_limit = Some(snapshot.clone());
    if remaining >= RATE_LIMIT_LOW_THRESHOLD {
        state.rate_limit_warned_low = false;
    }
    if remaining >= RATE_LIMIT_CRITICAL_THRESHOLD {
        state.rate_limit_warned_critical = false;
    }
    if remaining < RATE_LIMIT_CRITICAL_THRESHOLD && !state.rate_limit_warned_critical {
        state.rate_limit_warned_critical = true;
        state.rate_limit_warned_low = true;
        state.toast = Some(Toast {
            kind: ToastKind::Error,
            message: format!(
                "GitHub rate-limit critical: {}/{} remaining — background refresh halted",
                remaining, snapshot.limit
            ),
            created_at: std::time::Instant::now(),
        });
    } else if remaining < RATE_LIMIT_LOW_THRESHOLD && !state.rate_limit_warned_low {
        state.rate_limit_warned_low = true;
        state.toast = Some(Toast {
            kind: ToastKind::Warning,
            message: format!(
                "GitHub rate-limit low: {}/{} remaining — refresh interval extended to 5 min",
                remaining, snapshot.limit
            ),
            created_at: std::time::Instant::now(),
        });
    }
    Vec::new()
}

pub(super) fn fetch_failed_toast(
    state: &mut AppState,
    kind: &str,
    id: &PrId,
    err: impl std::fmt::Display,
) {
    state.toast = Some(Toast {
        kind: ToastKind::Error,
        message: format!(
            "{kind} fetch failed for {}/{}#{}: {err} — press R to retry",
            id.repo.owner, id.repo.name, id.number
        ),
        created_at: std::time::Instant::now(),
    });
}

pub(super) fn absorb_pr_detail(
    state: &mut AppState,
    id: &PrId,
    result: Result<&crate::data::models::PrDetail, &crate::data::github::GitHubError>,
) -> Vec<Effect> {
    match result {
        Ok(detail) => {
            let prior_head_sha = state.pr_details.get(id).map(|d| d.head_sha.clone());
            let next_version = state
                .pr_details
                .get(id)
                .map_or(detail.version, |existing| existing.version + 1);
            let mut stored = detail.clone();
            stored.version = next_version;
            state.pr_details.insert(id.clone(), stored);
            push_data_loaded_cue(state);
            state.last_refresh.insert(
                crate::data::cache::CacheKey::PrDetail(id.clone()),
                std::time::Instant::now(),
            );
            if let Some(before) = prior_head_sha
                && before != detail.head_sha
            {
                return handle_force_push_detected(state, id, &before, &detail.head_sha);
            }
            Vec::new()
        }
        Err(e) => {
            fetch_failed_toast(state, "pr_detail", id, e);
            Vec::new()
        }
    }
}

pub(super) fn handle_force_push_detected(
    state: &mut AppState,
    id: &PrId,
    before: &str,
    after: &str,
) -> Vec<Effect> {
    state.force_push_alert = Some(crate::app::state::ForcePushAlert {
        pr_id: id.clone(),
        before: before.to_string(),
        after: after.to_string(),
    });
    state.pr_files.remove(id);
    state.pr_diffs.remove(id);
    state.pr_file_diff_inflight.retain(|(i, _)| i != id);
    state.pr_files_load_requested.remove(id);
    if let Some(composer) = state.composer.as_mut() {
        let composer_pr_id = match &composer.context {
            crate::app::state::ComposerContext::PrComment { pr_id }
            | crate::app::state::ComposerContext::ReplyToThread { pr_id, .. }
            | crate::app::state::ComposerContext::LineComment { pr_id, .. } => pr_id,
        };
        if composer_pr_id == id {
            composer.head_moved = true;
        }
    }
    vec![Effect::FetchPrFiles {
        id: id.clone(),
        head_sha: after.to_string(),
    }]
}

pub(super) fn absorb_dashboard_ok(
    state: &mut AppState,
    payload: &[(
        crate::data::github::DashboardBucket,
        Vec<crate::data::models::PrSummary>,
        Option<String>,
    )],
) {
    // A bucket-scoped "load more" returns fewer buckets than we already hold
    // (typically one); its rows are the NEXT page and must be appended. A full
    // fetch returns every bucket and replaces the cache.
    let bucket_scoped = state
        .dashboard
        .as_ref()
        .is_some_and(|existing| payload.len() < existing.len());
    if bucket_scoped {
        if let Some(existing) = state.dashboard.as_mut() {
            for (bucket, rows, _) in payload {
                if let Some(slot) = existing.iter_mut().find(|(b, _)| b == bucket) {
                    slot.1.extend(rows.iter().cloned());
                } else {
                    existing.push((*bucket, rows.clone()));
                }
            }
        }
    } else {
        state.dashboard = Some(
            payload
                .iter()
                .map(|(bucket, rows, _)| (*bucket, rows.clone()))
                .collect(),
        );
    }
    // Thread each bucket's next-page cursor so `load_more_dashboard` can
    // request the following page (and stops once the cursor is `None`),
    // clear the in-flight marker, and de-dup in case a page was appended
    // more than once.
    for (bucket, _rows, cursor) in payload {
        state.dashboard_loading.remove(bucket);
        match cursor {
            Some(c) => {
                state.dashboard_cursors.insert(*bucket, c.clone());
            }
            None => {
                state.dashboard_cursors.remove(bucket);
            }
        }
    }
    let now = std::time::Instant::now();
    state
        .last_refresh
        .insert(crate::data::cache::CacheKey::DashboardReviewRequested, now);
    state
        .last_refresh
        .insert(crate::data::cache::CacheKey::DashboardAuthored, now);
    state
        .last_refresh
        .insert(crate::data::cache::CacheKey::DashboardAssigned, now);
    // Clamp each panel's remembered cursor to the (possibly shrunk) bucket
    // length so a refresh that removes rows can't strand a panel's selection
    // out of range.
    if let Selection::Dashboard { focused, rows } = *state.selection() {
        let mut clamped = rows;
        if let Some(buckets) = state.dashboard.as_ref() {
            for (i, slot) in clamped.iter_mut().enumerate() {
                let len = buckets.get(i).map_or(0, |(_, prs)| prs.len());
                *slot = (*slot).min(len.saturating_sub(1));
            }
        }
        if clamped != rows {
            *state.selection_mut() = Selection::Dashboard {
                focused,
                rows: clamped,
            };
        }
    }
    push_data_loaded_cue(state);
}

pub(super) fn absorb_pr_list_ok(
    state: &mut AppState,
    repo: &crate::data::models::Repo,
    rows: &[crate::data::models::PrSummary],
    next_cursor: Option<&str>,
    replace: bool,
) {
    state.pr_list_loading.remove(repo);
    if replace {
        // Fresh page 1 (initial load or refresh): replace cached rows so a
        // manual `R` or background refresh never duplicates them.
        state.pr_lists.insert(repo.clone(), rows.to_vec());
    } else {
        state
            .pr_lists
            .entry(repo.clone())
            .or_default()
            .extend(rows.iter().cloned());
    }
    match next_cursor {
        Some(c) => {
            state.pr_list_cursors.insert(repo.clone(), c.to_string());
        }
        None => {
            state.pr_list_cursors.remove(repo);
        }
    }
    // A fetch landing (success) ends any in-flight pagination for this repo.
    state.pr_list_paginating.remove(repo);
    state.last_refresh.insert(
        crate::data::cache::CacheKey::PrList(repo.clone()),
        std::time::Instant::now(),
    );
    push_data_loaded_cue(state);
}

pub(super) fn absorb_pr_files(
    state: &mut AppState,
    id: &PrId,
    result: Result<&crate::data::models::PrFiles, &crate::data::github::GitHubError>,
) {
    match result {
        Ok(files) => {
            state.pr_files.insert(id.clone(), files.clone());
            push_data_loaded_cue(state);
        }
        Err(e) => fetch_failed_toast(state, "pr_files", id, e),
    }
}

pub(super) fn absorb_pr_checks(
    state: &mut AppState,
    id: &PrId,
    result: Result<&crate::data::models::PrChecks, &crate::data::github::GitHubError>,
) {
    match result {
        Ok(checks) => {
            state.pr_checks.insert(id.clone(), checks.clone());
            state.last_refresh.insert(
                crate::data::cache::CacheKey::PrChecks(id.clone()),
                std::time::Instant::now(),
            );
            push_data_loaded_cue(state);
        }
        Err(e) => fetch_failed_toast(state, "pr_checks", id, e),
    }
}
