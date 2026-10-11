use super::*;
use std::os::unix::fs::{MetadataExt, symlink};

#[test]
fn planted_cache_links_and_shared_modes_are_rejected_without_execution() {
    let fixture = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let linked = fixture.path().join("linked");
    symlink(outside.path(), &linked).unwrap();
    assert!(cached(&linked, &AtomicBool::new(false)).is_err());
    assert_eq!(outside.path().read_dir().unwrap().count(), 0);

    let bytes = verified_cache::archive("code", tar::EntryType::Regular);
    let release = verified_cache::release(&bytes);
    for name in ["current", "code", "version", "shared-root", "shared-code"] {
        let root = tempfile::tempdir().unwrap();
        let program = publish(root.path(), &release, &AtomicBool::new(false), |path| {
            fs::write(path, &bytes).map_err(Error::Cache)
        })
        .unwrap();
        let destination = program.parent().unwrap();
        match name {
            "current" => {
                fs::remove_file(root.path().join("current")).unwrap();
                symlink(outside.path().join("sentinel"), root.path().join("current")).unwrap();
            }
            "code" => {
                fs::remove_file(&program).unwrap();
                symlink(outside.path().join("sentinel"), &program).unwrap();
            }
            "version" => {
                fs::remove_dir_all(destination).unwrap();
                symlink(outside.path(), destination).unwrap();
            }
            "shared-root" => {
                fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap()
            }
            "shared-code" => {
                fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap()
            }
            _ => unreachable!(),
        }
        assert!(cached(root.path(), &AtomicBool::new(false)).is_err());
        assert!(
            publish(root.path(), &release, &AtomicBool::new(false), |_| panic!(
                "downloaded into unsafe cache"
            ))
            .is_err()
        );
        assert_eq!(outside.path().read_dir().unwrap().count(), 0);
    }
}

#[test]
fn private_state_is_created_without_repairing_an_untrusted_directory() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("machines/local/cli");
    super::super::super::private::directory(&path).unwrap();
    let meta = fs::metadata(&path).unwrap();
    assert_eq!(meta.mode() & 0o7777, 0o700);
    assert_eq!(meta.uid(), rustix::process::geteuid().as_raw());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(super::super::super::private::directory(&path).is_err());
    assert_eq!(fs::metadata(path).unwrap().mode() & 0o7777, 0o777);
}
