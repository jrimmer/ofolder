//! Catches SIGTERM for the portal, so it can give its bus name back and return
//! instead of being killed mid-request.
//!
//! praeco restarts the portal by sending SIGTERM and waiting for it to exit. Without a handler the
//! process dies where it stands; with one, the portal stops at its next check, the connection is
//! dropped and the name is released. Only an atomic flag is written from the handler, which is all
//! that is safe to do there.

use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, Ordering};

const SIGTERM: c_int = 15;

static TERMINATE: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    /// C's `signal(2)`. The return value is the previous handler, which is not needed.
    fn signal(signum: c_int, handler: extern "C" fn(c_int)) -> usize;
}

extern "C" fn on_terminate(_signum: c_int) {
    TERMINATE.store(true, Ordering::SeqCst);
}

/// Makes SIGTERM set the flag read by [`terminate_requested`] instead of ending the process.
pub fn install_sigterm() {
    // SAFETY: `on_terminate` is an `extern "C"` function that only stores into an atomic, which is
    // async-signal-safe, and `signal` is given a valid function pointer for a valid signal number.
    unsafe {
        signal(SIGTERM, on_terminate);
    }
}

/// Whether SIGTERM has arrived since [`install_sigterm`].
pub fn terminate_requested() -> bool {
    TERMINATE.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigterm_sets_the_flag_once_the_handler_is_installed() {
        install_sigterm();
        assert!(!terminate_requested());
        // SAFETY: the handler is installed above and only sets the flag, so raising SIGTERM at this
        // test process is harmless; `raise` delivers it to the calling thread synchronously.
        unsafe { raise(SIGTERM) };
        assert!(terminate_requested());
        TERMINATE.store(false, Ordering::SeqCst);
    }

    unsafe extern "C" {
        fn raise(signum: c_int) -> c_int;
    }
}
