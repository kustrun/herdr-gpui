//! Managed machines share cookies only with pages bound to the same target.
use super::Tab;
use crate::code_server::Target;
use uuid::Uuid;

pub(super) fn identifier(tab: &Tab) -> Option<[u8; 16]> {
    let target = tab.code_target()?;
    if target == &Target::Configured {
        return None;
    }
    Some(
        *Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("herdr-gpui:code-target:{}", target.key()).as_bytes(),
        )
        .as_bytes(),
    )
}
