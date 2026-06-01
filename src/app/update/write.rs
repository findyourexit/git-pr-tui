use crate::data::models::PrId;

use super::super::{
    effect::Effect,
    state::{AppState, Toast, ToastKind},
};
use super::push_action_landed_cue;

pub(super) fn handle_checkout_completed(
    state: &mut AppState,
    id: &PrId,
    result: &Result<String, String>,
) -> Vec<Effect> {
    if state.checkout_inflight.as_ref() == Some(id) {
        state.checkout_inflight = None;
    }
    let toast = match result {
        Ok(branch) => Toast {
            kind: ToastKind::Info,
            message: format!("Checked out {branch}"),
            created_at: std::time::Instant::now(),
        },
        Err(msg) => Toast {
            kind: ToastKind::Error,
            message: format!("Checkout failed: {msg}"),
            created_at: std::time::Instant::now(),
        },
    };
    state.toast = Some(toast);
    Vec::new()
}

pub(super) fn handle_write_completed_ok(
    state: &mut AppState,
    kind: &crate::data::cache::WriteKind,
) -> Vec<Effect> {
    use crate::data::cache::WriteKind as W;
    let pr_id = match kind {
        W::SubmitReview { id }
        | W::PostPrComment { id }
        | W::PostLineComment { id }
        | W::ReplyToThread { id }
        | W::Merge { id }
        | W::Close { id }
        | W::Reopen { id } => id.clone(),
    };
    let version_at_submit = state.pending_write.as_ref().map(|p| p.version_at_submit);
    state.pending_write = None;
    let current_version = state.pr_details.get(&pr_id).map(|d| d.version);
    if let (Some(submitted), Some(current)) = (version_at_submit, current_version)
        && current > submitted + 1
    {
        state.toast = Some(Toast {
            kind: ToastKind::Warning,
            message: "PR updated remotely while your write was in flight — refreshed".into(),
            created_at: std::time::Instant::now(),
        });
    }
    push_action_landed_cue(state, true);
    vec![Effect::Refetch { kind: kind.clone() }]
}

pub(super) fn handle_write_completed_err(
    state: &mut AppState,
    kind: &crate::data::cache::WriteKind,
    err: &crate::data::github::GitHubError,
) -> Vec<Effect> {
    use crate::app::state::PendingStatus;
    use crate::data::github::GitHubError as E;

    let new_status = match err {
        E::Validation(msg) => Some(PendingStatus::Validation(msg.clone())),
        E::StaleHeadSha { expected } => Some(PendingStatus::Stale(expected.clone())),
        E::Conflict(msg) => Some(PendingStatus::Conflict(msg.clone())),
        E::Server(code) if *code >= 500 => Some(PendingStatus::UnknownOutcome),
        E::Network(_) | E::Timeout(_) => Some(PendingStatus::UnknownOutcome),
        _ => None,
    };

    if let Some(status) = new_status {
        if let Some(pw) = state.pending_write.as_mut() {
            pw.status = status.clone();
        }
        match status {
            PendingStatus::UnknownOutcome => {
                state.toast = Some(Toast {
                    kind: ToastKind::Warning,
                    message: "Outcome unknown — refreshed the PR".into(),
                    created_at: std::time::Instant::now(),
                });
                return vec![Effect::Refetch { kind: kind.clone() }];
            }
            PendingStatus::Conflict(msg) => {
                state.toast = Some(Toast {
                    kind: ToastKind::Error,
                    message: format!("conflict: {msg}"),
                    created_at: std::time::Instant::now(),
                });
            }
            PendingStatus::Validation(_) | PendingStatus::Stale(_) | PendingStatus::Submitting => {}
        }
        return Vec::new();
    }

    state.pending_write = None;
    state.toast = Some(Toast {
        kind: ToastKind::Error,
        message: format!("write failed: {err}"),
        created_at: std::time::Instant::now(),
    });
    push_action_landed_cue(state, false);
    Vec::new()
}

pub(super) fn review_option_to_state(
    opt: crate::app::state::ReviewModalOption,
) -> crate::data::models::ReviewState {
    use crate::app::state::ReviewModalOption;
    match opt {
        ReviewModalOption::Approve => crate::data::models::ReviewState::Approved,
        ReviewModalOption::RequestChanges => crate::data::models::ReviewState::ChangesRequested,
        ReviewModalOption::Comment => crate::data::models::ReviewState::Commented,
    }
}

pub(super) fn start_checkout(state: &mut AppState, id: &PrId) -> Vec<Effect> {
    if state.checkout_inflight.is_some() {
        return Vec::new();
    }
    let Some(status) = state.repo_status.as_ref() else {
        state.toast = Some(Toast {
            kind: ToastKind::Warning,
            message: "Not in a git repo \u{2014} cannot checkout".into(),
            created_at: std::time::Instant::now(),
        });
        return Vec::new();
    };
    if status.dirty {
        state.toast = Some(Toast {
            kind: ToastKind::Warning,
            message: "Worktree is dirty \u{2014} commit or stash first".into(),
            created_at: std::time::Instant::now(),
        });
        return Vec::new();
    }
    state.checkout_inflight = Some(id.clone());
    vec![Effect::CheckoutBranch { id: id.clone() }]
}

pub(super) fn blocked_reasons(detail: &crate::data::models::PrDetail) -> Vec<String> {
    use crate::data::models::{ChecksRollup, ReviewDecision};
    let mut reasons = Vec::new();
    if matches!(
        detail.summary.review_decision,
        Some(ReviewDecision::ChangesRequested | ReviewDecision::ReviewRequired) | None
    ) {
        reasons.push("Required reviews not satisfied".into());
    }
    if matches!(
        detail.summary.checks,
        ChecksRollup::Failure | ChecksRollup::Pending
    ) {
        reasons.push("Required checks failing or pending".into());
    }
    if reasons.is_empty() {
        reasons.push("Branch protection rules block this merge".into());
    }
    reasons
}

#[must_use]
pub(super) fn execute_merge_from_modal(state: &mut AppState) -> Vec<Effect> {
    use crate::app::state::MergeModalKind;
    let Some(modal) = state.merge_modal.as_ref() else {
        return Vec::new();
    };
    let MergeModalKind::MethodPicker { methods, selected } = &modal.kind else {
        return Vec::new();
    };
    let Some(method) = methods.get(*selected).copied() else {
        return Vec::new();
    };
    let pr_id = modal.pr_id.clone();
    let head_sha = modal.head_sha.clone();
    let version_at_submit = state.pr_details.get(&pr_id).map_or(0, |d| d.version);
    state.merge_modal = None;
    vec![Effect::ExecuteWrite {
        kind: crate::data::cache::WriteKind::Merge { id: pr_id },
        request: crate::app::effect::WriteRequest::Merge { method, head_sha },
        version_at_submit,
    }]
}
