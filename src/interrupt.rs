//! Whether the operator has asked a running scan to stop. ADR-0059.
//!
//! Set from a signal (Ctrl-C, a closed terminal, a polite kill), read by the
//! scan between files. A scan that sees it stops where it is, keeps what it
//! found in the report and says plainly that the result is not clean.

use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);

/// Has a stop been asked for since the last [`clear`]?
pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

/// Ask the running scan to stop. Safe to call from a signal handler: it is
/// one atomic store.
pub fn request() {
    REQUESTED.store(true, Ordering::SeqCst);
}

pub fn clear() {
    REQUESTED.store(false, Ordering::SeqCst);
}
