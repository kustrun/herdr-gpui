use super::*;
use std::os::unix::fs::{MetadataExt, symlink};

#[test]
fn machine_tokens_and_locks_reject_links_without_touching_the_destination() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("sentinel");
    fs::write(&sentinel, "fixture-token").unwrap();
    let before = fs::metadata(&sentinel).unwrap().mode();
    for lock in [false, true] {
        let path = root.path().join(if lock { "token-b" } else { "token-a" });
        symlink(
            &sentinel,
            if lock {
                path.with_extension("lock")
            } else {
                path.clone()
            },
        )
        .unwrap();
        assert!(Token::load_private(&path).is_err());
        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "fixture-token");
        assert_eq!(fs::metadata(&sentinel).unwrap().mode(), before);
    }
    let path = root.path().join("token");
    let token = Token::load_private(&path).unwrap();
    assert_eq!(Token::load_private(&path).unwrap().as_str(), token.as_str());
    assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o600);
}
