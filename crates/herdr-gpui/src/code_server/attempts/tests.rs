#![allow(clippy::unwrap_used)]
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn concurrent_attempts_never_exceed_four_and_cancel_without_starting() {
    assert!(Permit::acquire(|| true).is_none());
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let workers: Vec<_> = (0..16)
        .map(|_| {
            let active = active.clone();
            let peak = peak.clone();
            thread::spawn(move || {
                let _permit = Permit::acquire(|| false).unwrap();
                let now = active.fetch_add(1, Ordering::AcqRel) + 1;
                peak.fetch_max(now, Ordering::AcqRel);
                thread::sleep(Duration::from_millis(10));
                active.fetch_sub(1, Ordering::AcqRel);
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    assert!(peak.load(Ordering::Acquire) <= 4);
    assert_eq!(active.load(Ordering::Acquire), 0);
}
