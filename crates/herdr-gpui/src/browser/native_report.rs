//! Native callbacks never include page content in queue diagnostics.
use std::sync::mpsc::{SyncSender, TrySendError};

pub(super) fn report<T>(sender: &SyncSender<T>, event: T) {
    if let Err(TrySendError::Full(_)) = sender.try_send(event) {
        tracing::debug!("Dropped a browser event: queue full");
    }
}

#[cfg(test)]
mod tests;
