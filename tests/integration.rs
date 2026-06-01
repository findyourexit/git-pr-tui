#[path = "integration/harness.rs"]
mod harness;

#[path = "integration/ui/mod.rs"]
mod ui;

#[path = "integration/e2e_dashboard.rs"]
mod e2e_dashboard;

#[path = "integration/e2e_pr_list.rs"]
mod e2e_pr_list;

#[path = "integration/e2e_pr_detail.rs"]
mod e2e_pr_detail;

#[path = "integration/e2e_write_recovery.rs"]
mod e2e_write_recovery;

#[path = "integration/e2e_pr_comment.rs"]
mod e2e_pr_comment;

#[path = "integration/e2e_write_refetch.rs"]
mod e2e_write_refetch;

#[path = "integration/e2e_review_submit.rs"]
mod e2e_review_submit;

#[path = "integration/e2e_line_comment.rs"]
mod e2e_line_comment;

#[path = "integration/e2e_merge.rs"]
mod e2e_merge;

#[path = "integration/e2e_close_reopen.rs"]
mod e2e_close_reopen;

#[path = "integration/git_checkout.rs"]
mod git_checkout;

#[path = "integration/e2e_landing.rs"]
mod e2e_landing;

#[path = "integration/e2e_large_pr.rs"]
mod e2e_large_pr;
