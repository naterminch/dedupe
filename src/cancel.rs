//! Cooperative cancellation for scans.
//!
//! The GUI shares a [`CancelToken`] between the view and the scan task:
//! pressing Stop sets the flag, and every long pipeline stage polls
//! `is_cancelled` at its phase boundaries (walk loop, hash groups, media
//! fingerprinting, group building) and bails out early. Work already in
//! flight inside a rayon batch finishes — cancellation is cooperative,
//! not preemptive, so latency is bounded by one small unit of work.
//!
//! Headless callers (CLI, tests) use a fresh token that is never set.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A cloneable, thread-safe "please stop" flag.
#[derive(Debug, Default, Clone)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A fresh token in the un-cancelled state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation. Idempotent; safe to call from any thread.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Clear a previous request so the token can back a new scan.
    pub fn reset(&self) {
        self.0.store(false, Ordering::Relaxed);
    }

    /// True once [`CancelToken::cancel`] has been called.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_token_is_not_cancelled() {
        assert!(!CancelToken::new().is_cancelled());
    }

    #[test]
    fn cancel_is_visible_through_clones() {
        let token = CancelToken::new();
        let clone = token.clone();
        assert!(!clone.is_cancelled());
        token.cancel();
        assert!(clone.is_cancelled(), "clones share one flag");
        // Idempotent.
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn reset_clears_the_request() {
        let token = CancelToken::new();
        token.cancel();
        assert!(token.is_cancelled());
        token.reset();
        assert!(!token.is_cancelled());
    }
}
