//! gpr — TUI for viewing and managing GitHub PRs.
//!
//! The library crate exists so integration tests can construct
//! `App` and exercise the reducer + UI without going through `main`.

pub mod app;
pub mod cli;
pub mod config;
pub mod data;
pub mod error;
pub mod logging;
pub mod ui;

use std::sync::Mutex;
use std::sync::OnceLock;

type RestoreFn = Box<dyn Fn() + Send + Sync + 'static>;
static RESTORE_HOOK: OnceLock<Mutex<Option<RestoreFn>>> = OnceLock::new();

/// Install a panic hook that runs `restore` before delegating to the prior
/// hook. Used to ensure the terminal is reset out of raw-mode / alternate-
/// screen before the panic message prints to stderr.
pub fn install_panic_hook(restore: RestoreFn) {
    let slot = RESTORE_HOOK.get_or_init(|| Mutex::new(None));
    *slot.lock().expect("hook poisoned") = Some(restore);
    let prior = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(slot) = RESTORE_HOOK.get()
            && let Some(cb) = slot.lock().expect("hook poisoned").as_ref()
        {
            cb();
        }
        prior(info);
    }));
}
