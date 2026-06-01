//! Verifies that the panic hook installed by `gpr::install_panic_hook` calls
//! the terminal-restore callback before propagating the panic.

use std::sync::{Arc, Mutex};

#[test]
fn panic_hook_calls_restore_then_propagates() {
    let restored = Arc::new(Mutex::new(false));
    let r2 = restored.clone();
    gpr::install_panic_hook(Box::new(move || {
        *r2.lock().unwrap() = true;
    }));

    let result = std::panic::catch_unwind(|| {
        panic!("intentional");
    });
    assert!(result.is_err(), "panic must propagate after the hook runs");
    assert!(
        *restored.lock().unwrap(),
        "restore callback must have been invoked"
    );
}
