//! A wake-up call for a loop that would otherwise sleep on a timer.
//!
//! Whoever learns that something changed (an event from the compositor, a menu asked for) calls
//! [`Wake::notify`]; the loop that reacts waits in [`Wake::wait`] for that or for its own timeout,
//! whichever comes first. A call that arrives while nobody waits is kept, so it is never lost.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct Wake {
    pending: Mutex<bool>,
    signal: Condvar,
}

impl Wake {
    pub fn notify(&self) {
        // A poisoned lock only means another thread panicked; the flag is still meaningful.
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        *pending = true;
        self.signal.notify_all();
    }

    /// Waits up to `limit` for a call; true when one came (and is now used up), false when the
    /// time ran out.
    pub fn wait(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        while !*pending {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            let (guard, result) = self
                .signal
                .wait_timeout(pending, left)
                .unwrap_or_else(|e| e.into_inner());
            pending = guard;
            if result.timed_out() && !*pending {
                return false;
            }
        }
        *pending = false;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_call_made_before_the_wait_is_not_lost() {
        let wake = Wake::default();
        wake.notify();
        assert!(wake.wait(Duration::from_millis(10)));
        assert!(!wake.wait(Duration::from_millis(10)));
    }

    #[test]
    fn a_wait_ends_when_called_from_another_thread() {
        let wake = Arc::new(Wake::default());
        let caller = wake.clone();
        let started = Instant::now();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            caller.notify();
        });
        assert!(wake.wait(Duration::from_secs(5)));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_wait_with_no_call_ends_at_its_limit() {
        let wake = Wake::default();
        let started = Instant::now();
        assert!(!wake.wait(Duration::from_millis(40)));
        assert!(started.elapsed() >= Duration::from_millis(35));
    }
}
