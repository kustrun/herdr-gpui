//! What a VS Code tab shows until its page is there: why it is empty, the
//! license to accept, or that VS Code is starting.
use super::code::Reach;
use crate::{
    HerdrWindow,
    code_server::{Launcher, Startup, Target, launcher::Phase},
};
use gpui::{prelude::*, *};

impl HerdrWindow {
    /// Why the VS Code tab's page is not there yet: `failure` says why it
    /// could not be created.
    pub(super) fn render_code_status(
        &self,
        tab: &super::Tab,
        failure: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(target) = tab.code_target() else {
            return div().into_any_element();
        };
        let label = self.code_label(target);
        if target.resolve(&self.code_hosts()).is_err() {
            return self.render_code_unreachable(target, "This saved machine was removed, disabled, or changed. Choose its current entry with Open VS Code On.".into(), cx);
        }
        let startup = Launcher::startup_target(cx, &self.config.code, target);
        let text: SharedString = if !super::EMBEDDED {
            "This build cannot show pages in the window.".into()
        } else if let Some(message) = failure {
            return self.render_code_unreachable(target, message, cx);
        } else {
            match startup {
                Startup::Address if self.config.code.url.is_none() => {
                    "Set the VS Code server in Settings to show VS Code here.".into()
                }
                Startup::Finding => "Looking for VS Code\u{2026}".into(),
                Startup::Missing => "VS Code is not installed. Install it, or set the address \
                                     of a server you run in Settings."
                    .into(),
                Startup::Consent => return self.render_code_consent(label.clone(), cx),
                Startup::Idle | Startup::Starting { .. } => match Launcher::phase(cx, target) {
                    Some(Phase::Preparing) => "Preparing VS Code…".into(),
                    Some(Phase::Installing) => "Installing VS Code…".into(),
                    Some(Phase::Starting) | None => "Starting VS Code…".into(),
                },
                Startup::Failed { error, .. } => {
                    return self.render_code_unreachable(target, error.to_string().into(), cx);
                }
                Startup::Address | Startup::Ready { .. } => match self
                    .browser
                    .code_servers
                    .get(&tab.id)
                    .map(|server| &server.state)
                {
                    Some(Reach::Failed { message, .. }) => {
                        return self.render_code_unreachable(target, message.clone(), cx);
                    }
                    Some(Reach::Ready { server, .. }) => {
                        format!("Loading VS Code {}\u{2026}", server.short_commit()).into()
                    }
                    Some(Reach::Unknown | Reach::Asking) | None => {
                        "Connecting to VS Code\u{2026}".into()
                    }
                },
            }
        };
        centered(
            div()
                .debug_selector(|| "code-placeholder".into())
                .text_color(rgb(self.theme.muted))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.))
                .child(label)
                .child(text),
        )
    }

    /// Why the server did not answer, and that it is asked again.
    fn render_code_unreachable(
        &self,
        target: &Target,
        message: SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = self.code_label(target);
        let target = target.clone();
        let theme = &self.theme;
        centered(
            div()
                .debug_selector(|| "code-unreachable".into())
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.))
                .text_center()
                .child(
                    svg()
                        .path("icons/error.svg")
                        .size(px(32.))
                        .text_color(rgb(theme.ink(theme.palette[1]))),
                )
                .child(
                    div()
                        .text_color(rgb(theme.foreground))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Cannot reach {label}")),
                )
                .child(div().text_color(rgb(theme.muted)).child(message))
                .child(
                    div()
                        .id("code-retry")
                        .debug_selector(|| "code-retry".into())
                        .text_color(rgb(theme.foreground))
                        .cursor_pointer()
                        .px(px(14.))
                        .py(px(8.))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.retry_code_target(&target, cx)),
                        )
                        .child("Retry"),
                ),
        )
    }

    /// Whether VS Code is offered, in each group's "…" menu and as Open
    /// VS Code: once a server is set, or the app starts VS Code, in a build
    /// that can show pages.
    pub(crate) fn code_offered(&self, cx: &App) -> bool {
        super::EMBEDDED
            && (cfg!(target_os = "macos")
                || Launcher::startup(cx, &self.config.code).offered(&self.config.code))
    }
}

/// `content` in the middle of the tab, both ways.
pub(super) fn centered(content: impl IntoElement) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .items_center()
        .justify_center()
        .px_4()
        .child(content)
        .into_any_element()
}
