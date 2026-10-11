use super::*;

#[test]
fn wrapper_phase_record_is_generation_checked_and_precedes_readiness() {
    let state = output(
        std::io::Cursor::new(b"START 6 0\nSTART 7 0\nREADY 7 51001\n".to_vec()),
        7,
    )
    .unwrap();
    ownership::until("phase and ready records", || state.lock().unwrap().closed);
    let state = state.lock().unwrap();
    assert!(state.starting);
    assert_eq!(state.port, NonZeroU16::new(51001));
    drop(state);
    let stale = output(std::io::Cursor::new(b"START 6 0\n".to_vec()), 7).unwrap();
    ownership::until("stale phase record", || stale.lock().unwrap().closed);
    assert!(!stale.lock().unwrap().starting);
}
