use super::*;
use crate::orchestrator::{
    Request, SyncState, Timing,
    service::SourceStatus,
    view::{Look, OrchestratorView},
};
use gpui::TestAppContext;
use std::sync::Arc;

#[gpui::test]
fn github_failure_keeps_local_beads_visible_and_selectable(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let request = Request {
        target: herdr_client::ConnectTarget::Local,
        checkout: dir.path().to_str().unwrap().into(),
        workspace_id: "workspace-1".into(),
        token: None,
        data_root: Some(dir.path().join("data")),
        timing: Timing::default(),
    };
    let config = crate::config::Config::default();
    let look = Look {
        theme: crate::config::Theme::default(),
        ui: config.ui,
        mono: config.terminal,
    };
    let (view, cx) = cx.add_window_view(|_, cx| OrchestratorView::new(request, look, cx));
    view.update(cx, |view, cx| {
        view.service = None;
        view.snapshot.items = Arc::new(vec![item(
            Provider::Beads,
            "example-1",
            "Local task",
            "2026-01-01T00:00:00Z",
        )]);
        view.snapshot.sources = vec![SourceStatus {
            key: source(Provider::Github),
            state: SyncState::Failed(Arc::new(crate::orchestrator::Error::Github(Box::new(
                crate::Error::GitHubToken,
            )))),
        }];
        view.filters.provider = Some(Provider::Beads);
        view.refresh_rows();
        assert_eq!(view.issue_rows.len(), 1);
        view.selected = None;
        cx.notify();
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("orchestrator-issue-0").unwrap();
    assert!(cx.debug_bounds("orchestrator-banner-0").is_some());
    cx.simulate_click(bounds.center(), gpui::Modifiers::none());
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.selected.as_deref(),
            Some(view.snapshot.items[0].key.canonical().as_str())
        );
    });
}
