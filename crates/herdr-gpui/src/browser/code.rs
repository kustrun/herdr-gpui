//! Each displayed editor owns its readiness check; runtimes are shared by target.
#[cfg(any(target_os = "macos", windows, test))]
use super::TabId;
use super::{GroupId, Location, Shown, Store, Tab, WebUrl, view::store};
use crate::{
    HerdrWindow,
    code_server::{Launcher, Server, Startup, Target},
    window::Flash,
};
use gpui::{prelude::*, *};
use std::time::{Duration, Instant};

mod urls;
#[cfg(any(target_os = "macos", windows))]
use urls::with_token;
pub(in crate::browser) use urls::without_token;
use urls::{on_server, with_folder};
const TOKEN: &str = "tkn";
const FOLDER: &str = "folder";
const RETRY: Duration = Duration::from_secs(5);

pub(crate) struct CodeServer {
    url: WebUrl,
    runtime: Option<u64>,
    pub(super) state: Reach,
    pub(super) owner: super::code_launch::Owner,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Reach {
    Unknown,
    Asking,
    Ready { server: Server, at: Instant },
    Failed { message: SharedString, at: Instant },
}

impl HerdrWindow {
    #[cfg(test)]
    pub(crate) fn code_tab_id(&self, cx: &App) -> Option<TabId> {
        let (scope, workspace) = self.browser_key()?;
        Some(store(cx)?.code_tab(&scope, &workspace)?.id)
    }

    pub(crate) fn open_code(
        &mut self,
        group: Option<GroupId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(owner) = super::code_launch::Owner::capture(self, group) else {
            self.show_flash(Flash::warning("Open a workspace first"), cx);
            return;
        };
        let hosts = self.code_hosts();
        let endpoint = &self.endpoints[self.selected_endpoint];
        let target = if cfg!(target_os = "macos") {
            Target::default_for(
                &self.config.code,
                &endpoint.connection.target,
                crate::endpoint::saved_profile_id(&endpoint.id),
                &hosts,
            )
        } else {
            Ok(Target::Configured)
        };
        match target {
            Ok(target) => self.open_code_for(owner, target, window, cx),
            Err(_) => self.show_flash(
                Flash::warning("This workspace's machine cannot run VS Code"),
                cx,
            ),
        }
    }

    pub(in crate::browser) fn open_code_for(
        &mut self,
        owner: super::code_launch::Owner,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !super::EMBEDDED {
            self.show_flash(Flash::warning("This build cannot show pages"), cx);
            return;
        }
        if !owner.current(self) {
            return;
        }
        if target.resolve(&self.code_hosts()).is_err() {
            self.show_flash(
                Flash::warning("This saved machine was removed or changed"),
                cx,
            );
            return;
        }
        let startup = Launcher::startup_target(cx, &self.config.code, &target);
        if !startup.offered(&self.config.code) {
            self.show_flash(Flash::warning("Set the VS Code server in Settings"), cx);
            return;
        }
        let location = startup.url(&self.config.code).map(|url| Location::Web {
            url: self.code_start_for(&owner, &target, url),
        });
        let Some(id) = Store::update(cx, |store| {
            store.open_code_tab_for(owner.scope.clone(), &owner.workspace, target, location)
        }) else {
            self.show_flash(Flash::warning("Too many browser tabs are open"), cx);
            return;
        };
        if self.browser_key().as_ref() == Some(&(owner.scope.clone(), owner.workspace.clone())) {
            self.show_browser_tab_in(Some(owner.group), id, window, cx);
        } else if let Some(layout) = self
            .browser
            .layouts
            .get_mut(&(owner.scope, owner.workspace))
        {
            layout.choose(owner.group, super::Pick::Page(id));
            cx.notify();
        }
    }

    pub(super) fn code_start_for(
        &self,
        owner: &super::code_launch::Owner,
        target: &Target,
        url: &WebUrl,
    ) -> WebUrl {
        let hosts = self.code_hosts();
        let same_machine = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.id == owner.endpoint)
            .is_some_and(|endpoint| match target {
                Target::Configured => !endpoint.connection.target.is_remote(),
                _ => Target::owner(
                    &endpoint.connection.target,
                    crate::endpoint::saved_profile_id(&endpoint.id),
                    &hosts,
                )
                .is_ok_and(|machine| machine == *target),
            });
        let folder = owner
            .snapshot(self)
            .and_then(|snapshot| crate::palette::launch_root(snapshot, &owner.workspace))
            .filter(|folder| same_machine && absolute_folder(target, folder));
        let url = without_token(url);
        folder.map_or_else(|| url.clone(), |folder| with_folder(&url, folder))
    }

    /// Demand only what every displayed group shows, including unfocused groups.
    pub(super) fn ensure_code_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !super::EMBEDDED {
            return;
        }
        let hosts = self.code_hosts();
        Launcher::sync(cx, &self.config.code);
        Launcher::sync_targets(cx, &hosts);
        Launcher::refresh(cx);
        let wanted = self.displayed_code_tabs(cx);
        self.browser
            .code_servers
            .retain(|id, _| store(cx).is_some_and(|store| store.get(*id).is_some()));
        let mut targets: Vec<_> = self.browser.code_addresses.keys().cloned().collect();
        for (_, tab) in &wanted {
            if let Some(target) = tab.code_target()
                && !targets.contains(target)
            {
                targets.push(target.clone());
            }
        }
        for target in targets {
            if store(cx).is_none_or(|store| store.code_tabs(&target).next().is_none()) {
                self.browser.code_addresses.remove(&target);
                continue;
            }
            let startup = Launcher::startup_target(cx, &self.config.code, &target);
            let available = target.resolve(&hosts).is_ok();
            if !available
                || matches!(startup, Startup::Failed { .. })
                || startup.url(&self.config.code).is_none()
            {
                self.forget_code_target(&target, cx);
            } else if let Some(url) = startup.url(&self.config.code).cloned() {
                let address = (url.clone(), Launcher::generation(cx, &target));
                if self.browser.code_addresses.get(&target) != Some(&address) {
                    self.forget_code_target(&target, cx);
                    Store::update(cx, |store| {
                        store.follow_code_target(&target, &url, |page| on_server(page, &url))
                    });
                    self.browser.code_addresses.insert(target, address);
                }
            }
        }
        for (group, tab) in wanted {
            self.ensure_code_tab(group, tab, &hosts, window, cx);
        }
    }

    fn displayed_code_tabs(&self, cx: &App) -> Vec<(GroupId, Tab)> {
        self.group_slots()
            .into_iter()
            .filter_map(|slot| {
                let Shown::Page(id) = self.group_shown(slot.id, cx) else {
                    return None;
                };
                let tab = store(cx)?.get(id)?.clone();
                tab.code_target()?;
                Some((slot.id, tab))
            })
            .collect()
    }

    fn ensure_code_tab(
        &mut self,
        group: GroupId,
        mut tab: Tab,
        hosts: &[herdr_client::SavedHost],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = tab.code_target().cloned() else {
            return;
        };
        if target.resolve(hosts).is_err() {
            return;
        }
        let Some(owner) = super::code_launch::Owner::capture(self, Some(group)) else {
            return;
        };
        if self.browser.failed.contains_key(&tab.id) {
            return;
        }
        let startup = Launcher::startup_target(cx, &self.config.code, &target);
        // A permanent failure remains visible until Retry; demand never falls back.
        if let Err(error) = Launcher::want_target(cx, &self.config.code, &target, hosts) {
            self.browser
                .failed
                .entry(tab.id)
                .or_insert_with(|| error.to_string().into());
            return;
        }
        Launcher::refresh(cx);
        let startup = if matches!(startup, Startup::Idle | Startup::Starting { .. }) {
            Launcher::startup_target(cx, &self.config.code, &target)
        } else {
            startup
        };
        let Some(url) = startup.url(&self.config.code).cloned() else {
            return;
        };
        if tab.location.is_none() {
            let location = Location::Web {
                url: self.code_start_for(&owner, &target, &url),
            };
            Store::update(cx, |store| {
                store.visited_code(tab.id, &target, Some(location.clone()), None)
            });
            tab.location = Some(location);
        } else {
            // Retarget may have updated the stored navigation above.
            if let Some(current) = store(cx).and_then(|store| store.get(tab.id)) {
                tab = current.clone();
            }
        }
        if !startup.serving() {
            return;
        }
        #[cfg(any(target_os = "macos", windows))]
        if self.browser.pages.contains(tab.id) {
            if self.code_page_current(tab.id, cx) {
                return;
            }
            self.browser.pages.close(tab.id);
        }
        if self.browser.failed.contains_key(&tab.id) {
            return;
        }
        let started = !matches!(startup, Startup::Address);
        if !self.code_server_ready(&tab, owner, &url, started, cx)
            || (started && !Launcher::alive_target(cx, &target))
        {
            return;
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        let _ = window;
        #[cfg(any(target_os = "macos", windows))]
        {
            if let Some(Location::Web { url: page }) = &tab.location {
                tab.location = Some(Location::Web {
                    url: with_token(page, &url),
                });
            }
            if self
                .browser
                .code_servers
                .get(&tab.id)
                .is_none_or(|server| !server.owner.current(self))
            {
                return;
            }
            if self.browser.pages.ensure(&tab, window, cx).is_err() {
                // Native errors can contain the token-bearing URL.
                tracing::warn!("Cannot create a VS Code page");
                self.browser
                    .failed
                    .insert(tab.id, "The native page could not be created".into());
            }
            if let Some(server) = self.browser.code_servers.get_mut(&tab.id) {
                server.state = Reach::Unknown;
            }
            cx.notify();
        }
    }

    fn forget_code_target(&mut self, target: &Target, cx: &App) {
        if self.browser.code_addresses.remove(target).is_none() {
            return;
        }
        self.clear_code_target_pages(target, cx);
    }

    fn clear_code_target_pages(&mut self, target: &Target, cx: &App) {
        let ids: Vec<_> = store(cx)
            .map(|store| store.code_tabs(target).map(|tab| tab.id).collect())
            .unwrap_or_default();
        for id in ids {
            self.browser.code_servers.remove(&id);
            self.browser.failed.remove(&id);
            #[cfg(any(target_os = "macos", windows))]
            self.browser.pages.close(id);
        }
    }

    pub(super) fn retry_code_target(&mut self, target: &Target, cx: &mut Context<Self>) {
        self.browser.code_addresses.remove(target);
        self.clear_code_target_pages(target, cx);
        Launcher::retry_target(cx, target);
        cx.notify();
    }

    fn code_server_ready(
        &mut self,
        tab: &Tab,
        owner: super::code_launch::Owner,
        url: &WebUrl,
        started: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(target) = tab.code_target().cloned() else {
            return false;
        };
        let runtime = Launcher::generation(cx, &target);
        let server = self
            .browser
            .code_servers
            .entry(tab.id)
            .or_insert_with(|| CodeServer {
                url: url.clone(),
                runtime,
                state: Reach::Unknown,
                owner: owner.clone(),
            });
        if server.url != *url || server.runtime != runtime || server.owner != owner {
            *server = CodeServer {
                url: url.clone(),
                runtime,
                state: Reach::Unknown,
                owner: owner.clone(),
            };
        }
        match &server.state {
            Reach::Ready { at, .. } if at.elapsed() < RETRY => return true,
            Reach::Asking => return false,
            Reach::Failed { at, .. } if at.elapsed() < RETRY => return false,
            _ => {}
        }
        server.state = Reach::Asking;
        let probe = if started {
            self.browser.code_started_probe
        } else {
            self.browser.code_probe
        };
        let (id, url) = (tab.id, url.clone());
        let checked_url = url.clone();
        let answer = cx
            .background_executor()
            .spawn(async move { probe(&checked_url) });
        cx.spawn(async move |this, cx| {
            let answer = answer.await;
            let _ = this.update(cx, |this, cx| {
                let valid = owner.current(this)
                    && store(cx)
                        .and_then(|store| store.get(id))
                        .is_some_and(|tab| {
                            tab.scope == owner.scope
                                && tab.workspace_id == owner.workspace
                                && tab.code_target() == Some(&target)
                        })
                    && target.resolve(&this.code_hosts()).is_ok()
                    && Launcher::generation(cx, &target) == runtime;
                if !valid {
                    return;
                }
                let Some(server) = this.browser.code_servers.get_mut(&id).filter(|server| {
                    server.url == url && server.runtime == runtime && server.owner == owner
                }) else {
                    return;
                };
                server.state = match answer {
                    Ok(server) => Reach::Ready {
                        server,
                        at: Instant::now(),
                    },
                    Err(error) => {
                        tracing::info!("A VS Code readiness probe failed");
                        Reach::Failed {
                            message: error.to_string().into(),
                            at: Instant::now(),
                        }
                    }
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
        false
    }

    #[cfg(any(target_os = "macos", windows, test))]
    pub(super) fn code_page_current(&self, id: TabId, cx: &App) -> bool {
        let Some(tab) = store(cx).and_then(|store| store.get(id)) else {
            return false;
        };
        let Some(target) = tab.code_target() else {
            return true;
        };
        self.browser.code_servers.get(&id).is_some_and(|server| {
            server.owner.current(self)
                && tab.scope == server.owner.scope
                && tab.workspace_id == server.owner.workspace
                && target.resolve(&self.code_hosts()).is_ok()
                && Launcher::generation(cx, target) == server.runtime
        })
    }
}

fn absolute_folder(target: &Target, folder: &str) -> bool {
    !folder.chars().any(char::is_control)
        && match target {
            Target::Ssh { .. } => folder.starts_with('/'),
            Target::Local | Target::Configured => std::path::Path::new(folder).is_absolute(),
        }
}

#[cfg(test)]
mod tests;
