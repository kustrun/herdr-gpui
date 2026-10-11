use super::*;
use flate2::{Compression, write::GzEncoder};
use std::sync::Arc;

pub(super) fn archive(name: &str, kind: tar::EntryType) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    let body = b"#!/bin/sh\necho aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nexit 0\n";
    let mut header = tar::Header::new_gnu();
    header.set_path(name).unwrap();
    header.set_entry_type(kind);
    header.set_mode(0o755);
    header.set_size(if kind.is_file() { body.len() as u64 } else { 0 });
    header.set_cksum();
    tar.append(&header, if kind.is_file() { body.as_slice() } else { &[] })
        .unwrap();
    let bytes = tar.into_inner().unwrap();
    let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
    gzip.write_all(&bytes).unwrap();
    gzip.finish().unwrap()
}
pub(super) fn release(bytes: &[u8]) -> Release {
    Release {
        commit: "a".repeat(40),
        platform: "cli-linux-x64",
        url: "https://vscode.download.prss.microsoft.com/immutable/cli.tar.gz".into(),
        sha256: super::super::super::target::digest(bytes),
    }
}

#[test]
fn official_metadata_requires_immutable_commit_https_and_hash() {
    let valid = serde_json::json!({"version": "a".repeat(40), "url": "https://vscode.download.prss.microsoft.com/immutable/cli.tar.gz", "sha256hash": "b".repeat(64)});
    assert!(metadata(&serde_json::to_vec(&valid).unwrap(), "cli-linux-x64").is_ok());
    for (key, value) in [
        ("version", "latest"),
        ("url", "http://vscode.download.prss.microsoft.com/cli"),
        ("url", "https://example.invalid/cli"),
        ("sha256hash", "bad"),
    ] {
        let mut bad = valid.clone();
        bad[key] = value.into();
        assert!(matches!(
            metadata(&serde_json::to_vec(&bad).unwrap(), "cli-linux-x64"),
            Err(Error::Metadata)
        ));
    }
    assert!(matches!(
        platform("Windows", "x86_64"),
        Err(Error::UnsupportedTarget)
    ));
    assert_eq!(platform("Darwin", "arm64").unwrap(), "cli-darwin-arm64");
}

#[test]
fn hash_cancel_and_exact_regular_payload_are_checked_before_publish() {
    let root = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let good = archive("code", tar::EntryType::Regular);
    let metadata = release(&good);
    let program = publish(root.path(), &metadata, &cancel, |path| {
        fs::write(path, &good).map_err(Error::Cache)
    })
    .unwrap();
    assert!(program.is_file());
    assert_eq!(cached(root.path(), &cancel).unwrap(), Some(program.clone()));
    assert_eq!(
        fs::metadata(&program).unwrap().permissions().mode() & 0o777,
        0o700
    );
    // A working immutable version is reused without downloading anything.
    assert_eq!(
        publish(root.path(), &metadata, &cancel, |_| panic!(
            "downloaded cached CLI"
        ))
        .unwrap(),
        program
    );
    let mut next = metadata.clone();
    next.commit = "b".repeat(40);
    assert!(matches!(
        publish(root.path(), &next, &cancel, |path| {
            fs::write(path, b"truncated").map_err(Error::Cache)
        }),
        Err(Error::Digest)
    ));
    assert!(program.is_file());
    assert!(!root.path().read_dir().unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".install-")
    }));
    cancel.store(true, Ordering::Release);
    assert!(matches!(
        publish(root.path(), &next, &cancel, |_| Ok(())),
        Err(Error::Cancelled)
    ));
    for (name, kind) in [
        ("outside", tar::EntryType::Regular),
        ("code", tar::EntryType::Symlink),
        ("code", tar::EntryType::Directory),
    ] {
        let root = tempfile::tempdir().unwrap();
        assert!(matches!(
            extract(
                archive(name, kind).as_slice(),
                root.path(),
                &AtomicBool::new(false)
            ),
            Err(Error::Archive)
        ));
    }
}

#[test]
fn concurrent_publish_never_exposes_a_partial_executable() {
    let root = tempfile::tempdir().unwrap();
    let bytes = Arc::new(archive("code", tar::EntryType::Regular));
    let metadata = release(&bytes);
    let mut threads = Vec::new();
    for _ in 0..4 {
        let root = root.path().to_owned();
        let bytes = bytes.clone();
        let metadata = metadata.clone();
        threads.push(thread::spawn(move || {
            publish(&root, &metadata, &AtomicBool::new(false), |path| {
                fs::write(path, bytes.as_slice()).map_err(Error::Cache)
            })
            .unwrap()
        }));
    }
    let programs: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert!(programs.windows(2).all(|pair| pair[0] == pair[1]));
    assert!(capable(&programs[0], &AtomicBool::new(false)).unwrap());
}
