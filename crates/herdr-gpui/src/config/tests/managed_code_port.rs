use super::*;
use std::sync::atomic::AtomicBool;

#[test]
fn automatic_acknowledgement_preserves_explicit_edits_and_cancelled_generations()
-> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("config.local.toml");
    let port = std::num::NonZeroU16::new(51001).context("fixture port")?;
    let active = AtomicBool::new(false);
    fs::write(&path, "# keep this\n[code]\nlicense_accepted = true\n")?;
    assert!(Config::save_managed_code_port_path(port, &active, &path)?);
    let acknowledged = fs::read_to_string(&path)?;
    assert!(acknowledged.contains("# keep this"));
    assert_eq!(Config::parse(&acknowledged)?.code.port, Some(port));

    let explicit = "[code]\nport = 51002\nlicense_accepted = true\n";
    fs::write(&path, explicit)?;
    assert!(!Config::save_managed_code_port_path(port, &active, &path)?);
    assert_eq!(fs::read_to_string(&path)?, explicit);

    let unset = "[code]\nlicense_accepted = true\n";
    fs::write(&path, unset)?;
    assert!(!Config::save_managed_code_port_path(
        port,
        &AtomicBool::new(true),
        &path
    )?);
    assert_eq!(fs::read_to_string(&path)?, unset);
    Ok(())
}
