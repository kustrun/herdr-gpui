//! One app-wide bound across installation and server startup, including retries.
use std::{
    sync::{Mutex, PoisonError},
    thread,
    time::Duration,
};
static ACTIVE: Mutex<usize> = Mutex::new(0);
pub(super) struct Permit;
impl Permit {
    pub(super) fn acquire(cancelled: impl Fn() -> bool) -> Option<Self> {
        loop {
            if cancelled() {
                return None;
            }
            {
                let mut active = ACTIVE.lock().unwrap_or_else(PoisonError::into_inner);
                if *active < 4 {
                    *active += 1;
                    return Some(Self);
                }
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        *ACTIVE.lock().unwrap_or_else(PoisonError::into_inner) -= 1;
    }
}

#[cfg(test)]
mod tests;
