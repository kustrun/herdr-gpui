//! A machine-only picker retains its originating workspace and editor group.
use super::{GroupId, Scope};
use crate::{
    HerdrWindow,
    code_server::{Launcher, Target},
    menu::Page,
    search_input::{Changed, SearchInput},
    window::Flash,
};
use gpui::{prelude::*, *};
use herdr_client::{ConnectTarget, SavedHost, protocol::ClientShellSnapshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Owner {
    pub(super) scope: Scope,
    pub(super) workspace: String,
    pub(super) group: GroupId,
    pub(super) endpoint: String,
    generation: u64,
    boot: String,
}

impl Owner {
    pub(super) fn capture(view: &mut HerdrWindow, group: Option<GroupId>) -> Option<Self> {
        let (scope, workspace) = view.browser_key()?;
        let group = group.or_else(|| view.ensure_layout().map(|layout| layout.active()))?;
        let endpoint = &view.endpoints[view.selected_endpoint];
        let owner = Self {
            scope,
            workspace,
            group,
            endpoint: endpoint.id.clone(),
            generation: endpoint.generation,
            boot: view.live.snapshot.as_ref()?.boot_id.clone(),
        };
        owner.current(view).then_some(owner)
    }

    pub(super) fn snapshot<'a>(&self, view: &'a HerdrWindow) -> Option<&'a ClientShellSnapshot> {
        let (index, endpoint) = view
            .endpoints
            .iter()
            .enumerate()
            .find(|(_, endpoint)| endpoint.id == self.endpoint)?;
        if endpoint.generation != self.generation || super::scope(endpoint) != self.scope {
            return None;
        }
        if index == view.selected_endpoint {
            view.live.snapshot.as_deref()
        } else {
            endpoint.live.snapshot.as_deref()
        }
    }

    pub(super) fn current(&self, view: &HerdrWindow) -> bool {
        self.snapshot(view).is_some_and(|snapshot| {
            snapshot.boot_id == self.boot
                && snapshot
                    .workspaces
                    .iter()
                    .any(|workspace| workspace.workspace_id == self.workspace)
        }) && view
            .browser
            .layouts
            .get(&(self.scope.clone(), self.workspace.clone()))
            .is_some_and(|layout| layout.slots().any(|slot| slot.id == self.group))
    }
}

pub(super) struct Choice {
    label: String,
    detail: &'static str,
    target: Option<Target>,
}

pub(super) struct Picker {
    owner: Owner,
    search: Entity<SearchInput>,
    choices: Vec<Choice>,
    filtered: Vec<usize>,
    selected: usize,
    scroll: UniformListScrollHandle,
    _subscription: Subscription,
}

impl Picker {
    fn filter(&mut self, query: &str) {
        let query = query.trim().to_lowercase();
        self.filtered = self
            .choices
            .iter()
            .enumerate()
            .filter(|(_, choice)| {
                choice.label.to_lowercase().contains(&query)
                    || choice.detail.to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
    }
}

impl HerdrWindow {
    pub(crate) fn clear_code_picker(&mut self) {
        self.browser.code_picker = None;
    }
    /// Derive descriptors from saved endpoint IDs, never from a matching alias.
    pub(super) fn code_hosts(&self) -> Vec<SavedHost> {
        self.endpoints
            .iter()
            .filter_map(|endpoint| {
                let id = crate::endpoint::saved_profile_id(&endpoint.id)?;
                let ConnectTarget::Ssh { target, session } = &endpoint.connection.target else {
                    return None;
                };
                Some(SavedHost {
                    id: id.to_owned(),
                    label: endpoint.label.clone(),
                    target: target.clone(),
                    session: session.clone(),
                    enabled: endpoint.enabled,
                })
            })
            .collect()
    }

    pub(super) fn code_label(&self, target: &Target) -> SharedString {
        let machine = match target {
            Target::Configured => "Configured".to_owned(),
            Target::Local => "Local".to_owned(),
            Target::Ssh { id, .. } => self
                .code_hosts()
                .iter()
                .find(|host| {
                    &host.id == id && Target::saved(host).is_ok_and(|saved| saved == *target)
                })
                .map(|host| host.label.clone())
                .unwrap_or_else(|| "Unavailable SSH machine".into()),
        };
        format!("VS Code · {machine}").into()
    }

    pub(super) fn browser_tab_label(&self, tab: &super::Tab) -> SharedString {
        match tab.code_target() {
            Some(target) => super::view::tab_label(&self.code_label(target)),
            None => super::view::tab_label(&tab.title),
        }
    }

    fn code_choices(&self, cx: &App) -> Vec<Choice> {
        let mut choices = vec![Choice {
            label: "Local".into(),
            detail: "This computer",
            target: Some(Target::Local),
        }];
        let hosts = self.code_hosts();
        for endpoint in self
            .endpoints
            .iter()
            .filter(|endpoint| endpoint.connection.target.is_remote())
        {
            let saved = crate::endpoint::saved_profile_id(&endpoint.id)
                .and_then(|id| hosts.iter().find(|host| host.id == id))
                .and_then(|host| Target::saved(host).ok());
            let (detail, target) = if !endpoint.enabled {
                ("Machine disabled", None)
            } else if let Some(target) = saved {
                (
                    if endpoint.live.status.is_connected() {
                        "Saved SSH machine"
                    } else {
                        "Saved SSH machine · offline; connects on demand"
                    },
                    Some(target),
                )
            } else {
                (
                    "Unsupported: VS Code requires saved SSH Linux or macOS",
                    None,
                )
            };
            choices.push(Choice {
                label: endpoint.label.clone(),
                detail,
                target,
            });
        }
        if Launcher::startup(cx, &self.config.code).offered(&self.config.code) {
            choices.push(Choice {
                label: "Configured".into(),
                detail: "Use Settings > Code",
                target: Some(Target::Configured),
            });
        }
        choices
    }

    pub(crate) fn open_code_on(
        &mut self,
        group: Option<GroupId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !cfg!(target_os = "macos") {
            self.show_flash(
                Flash::warning("Choosing a VS Code machine requires macOS"),
                cx,
            );
            return;
        }
        let Some(owner) = Owner::capture(self, group) else {
            self.show_flash(Flash::warning("Open a workspace first"), cx);
            return;
        };
        if !self.open_menu(window, cx) {
            return;
        }
        let search = cx.new(SearchInput::new);
        search.update(cx, |input, cx| {
            input.set_placeholder("Search machines…", cx);
            input.set_appearance(self.config.ui.clone(), self.theme.clone(), cx);
            window.focus(&input.focus, cx);
        });
        let subscription = cx.subscribe(&search, |this, search, _: &Changed, cx| {
            if let Some(picker) = &mut this.browser.code_picker {
                picker.filter(search.read(cx).text());
            }
            cx.notify();
        });
        let choices = self.code_choices(cx);
        let filtered = (0..choices.len()).collect();
        self.browser.code_picker = Some(Picker {
            owner,
            search,
            choices,
            filtered,
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            _subscription: subscription,
        });
        self.menu.page = Some(Page::CodeMachine);
        cx.notify();
    }

    fn choose_code_machine(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.browser.code_picker.as_ref() else {
            return;
        };
        let Some(choice) = picker
            .filtered
            .get(row)
            .and_then(|index| picker.choices.get(*index))
        else {
            return;
        };
        let Some(target) = choice.target.clone() else {
            return;
        };
        let owner = picker.owner.clone();
        if !owner.current(self) {
            self.dismiss_menu(window, cx);
            self.show_flash(
                Flash::warning("The originating workspace or group closed"),
                cx,
            );
            return;
        }
        self.dismiss_menu(window, cx);
        self.open_code_for(owner, target, window, cx);
    }

    pub(crate) fn code_machine_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = &mut self.browser.code_picker else {
            return;
        };
        if picker.search.read(cx).is_composing() {
            return;
        }
        match event.keystroke.key.as_str() {
            "escape" => self.dismiss_menu(window, cx),
            "enter" => {
                let row = picker.selected;
                self.choose_code_machine(row, window, cx);
            }
            "up" | "down" if !picker.filtered.is_empty() => {
                let count = picker.filtered.len();
                picker.selected = (picker.selected
                    + if event.keystroke.key == "up" {
                        count - 1
                    } else {
                        1
                    })
                    % count;
                picker
                    .scroll
                    .scroll_to_item(picker.selected, ScrollStrategy::Top);
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
        window.prevent_default();
    }

    pub(crate) fn render_code_machines(&self, cx: &mut Context<Self>) -> Div {
        let Some(picker) = &self.browser.code_picker else {
            return div();
        };
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .max_h(px(440.))
            .child(
                div()
                    .p(px(16.))
                    .child("Open VS Code On")
                    .child(picker.search.clone()),
            )
            .child(
                uniform_list(
                    "code-machines",
                    picker.filtered.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|row| {
                                let Some(picker) = &this.browser.code_picker else {
                                    return div().into_any_element();
                                };
                                let choice = &picker.choices[picker.filtered[row]];
                                div()
                                    .id(row)
                                    .debug_selector(move || format!("code-machine-{row}"))
                                    .px(px(16.))
                                    .py(px(8.))
                                    .flex()
                                    .flex_col()
                                    .when(row == picker.selected, |row| {
                                        row.bg(rgb(this.theme.active))
                                    })
                                    .when(choice.target.is_some(), |row| row.cursor_pointer())
                                    .text_color(rgb(if choice.target.is_some() {
                                        this.theme.foreground
                                    } else {
                                        this.theme.muted
                                    }))
                                    .child(choice.label.clone())
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(this.theme.muted))
                                            .child(choice.detail),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.choose_code_machine(row, window, cx);
                                    }))
                                    .into_any_element()
                            })
                            .collect()
                    }),
                )
                .track_scroll(&picker.scroll)
                .h(px((picker.filtered.len() as f32 * 60.).min(340.)))
                .min_h_0(),
            )
    }
}

#[cfg(test)]
mod tests;
