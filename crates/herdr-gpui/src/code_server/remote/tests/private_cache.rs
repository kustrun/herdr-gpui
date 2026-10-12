use super::*;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

#[test]
fn read_only_probe_reuses_private_cache_and_rejects_planted_links_before_execution() {
    let fixture = tempfile::tempdir().unwrap();
    let bin = fixture.path().join("bin");
    fs::create_dir(&bin).unwrap();
    crate::test_executable::write(bin.join("code"), "#!/bin/sh\nexit 1\n", 0o700).unwrap();
    crate::test_executable::write(
        bin.join("uname"),
        "#!/bin/sh\ncase $1 in -s) echo Linux;; -m) echo x86_64;; esac\n",
        0o700,
    )
    .unwrap();
    let marker = fixture.path().join("executed");
    let body = format!(
        "#!/bin/sh\ntouch {}\n",
        herdr_client::shell_quote(&marker.to_string_lossy())
    );
    let outside = fixture.path().join("outside");
    crate::test_executable::write(&outside, &body, 0o700).unwrap();
    for mode in [
        "valid",
        "root",
        "pointer",
        "program",
        "permissions",
        "oversize",
    ] {
        let home = tempfile::tempdir().unwrap();
        let cache = home.path().join(".cache/herdr-gpui/vscode");
        super::super::super::private::directory(&cache).unwrap();
        let pointer = cache.join("current-cli-linux-x64");
        fs::write(&pointer, format!("{}\n", "a".repeat(40))).unwrap();
        fs::set_permissions(&pointer, fs::Permissions::from_mode(0o600)).unwrap();
        let program = cache.join(format!("{}-cli-linux-x64", "a".repeat(40)));
        crate::test_executable::write(&program, &body, 0o700).unwrap();
        match mode {
            "root" => {
                fs::remove_dir_all(&cache).unwrap();
                symlink(fixture.path(), &cache).unwrap();
            }
            "pointer" => {
                fs::remove_file(&pointer).unwrap();
                symlink(&outside, &pointer).unwrap();
            }
            "program" => {
                fs::remove_file(&program).unwrap();
                symlink(&outside, &program).unwrap();
            }
            "permissions" => {
                fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap()
            }
            "oversize" => fs::write(&pointer, "a".repeat(4096)).unwrap(),
            _ => {}
        }
        let result = Command::new("/bin/sh")
            .args(["-c", &format!("{PRIVATE}\n{PROBE}")])
            .env_clear()
            .env("HOME", home.path())
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .output()
            .unwrap();
        assert!(result.status.success());
        let status = if mode == "valid" {
            "installed"
        } else if mode == "oversize" {
            "missing"
        } else {
            "unsafe"
        };
        assert_eq!(
            result.stdout,
            format!("Linux\nx86_64\n{status}\n").as_bytes()
        );
        assert_eq!(marker.exists(), mode == "valid");
        if marker.exists() {
            fs::remove_file(&marker).unwrap();
        }
    }
}

#[test]
fn data_directory_links_and_shared_modes_are_never_repaired() {
    let fixture = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let linked = fixture.path().join("data");
    symlink(outside.path(), &linked).unwrap();
    let check = |path: &std::path::Path| {
        Command::new("/bin/sh")
            .args([
                "-c",
                &format!(
                    "{PRIVATE}\nprepare_private_directory {}",
                    herdr_client::shell_quote(&path.to_string_lossy())
                ),
            ])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .status()
            .unwrap()
    };
    assert!(!check(&linked).success());
    fs::remove_file(&linked).unwrap();
    fs::create_dir(&linked).unwrap();
    fs::set_permissions(&linked, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!check(&linked).success());
    assert_eq!(
        fs::metadata(&linked).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(outside.path().read_dir().unwrap().count(), 0);
}
