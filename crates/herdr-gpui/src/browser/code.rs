//! A workspace's VS Code tab: one page per workspace, served by
//! `code serve-web`, opened on its server's address and then wherever it
//! goes. It is a tab in the editor groups like a browser tab, so the user
//! splits it beside a terminal or shows it alone, and it is saved and
//! dropped with its workspace like any other tab. A page no group shows
//! stays alive, keeping its state.
//!
//! A page that cannot load says nothing back through the web view, so the
//! window first asks the server whether it is there, and creates each page
//! only after an answer of its own. Until then the tab says why it is
//! empty. The server is the one at `[code] url`, or the one the app starts
//! (see [`crate::code_server::launcher`]).
use super::{GroupId, Location, Scope, Store, TabId, WebUrl, view::store};
use crate::{
    HerdrWindow,
    code_server::{self, Launcher, Server, Startup},
    window::Flash,
};
use gpui::{prelude::*, *};
use std::time::{Duration, Instant};

mod urls;
#[cfg(any(target_os = "macos", windows))]
use urls::with_token;
pub(in crate::browser) use urls::without_token;
use urls::{on_server, with_folder};

/// The query parameter that carries `code serve-web`'s connection token.
const TOKEN: &str = "tkn";

/// The query parameter naming the folder `code serve-web` opens.
const FOLDER: &str = "folder";

/// How long an answer is trusted when a page is created, and how long an
/// unreachable server is left before it is asked again.
const RETRY: Duration = Duration::from_secs(5);

/// What the window knows of the configured VS Code server.
pub(crate) struct CodeServer {
    /// The address the state is for; a new one starts over.
    url: Option<WebUrl>,
    pub(super) state: Reach,
    /// Fences out the answer to a question asked about an older address.
    generation: u64,
    /// How the server is asked; tests answer for it.
    pub(super) probe: fn(&WebUrl) -> crate::Result<Server>,
    /// How the server the app started is asked: its version alone, never
    /// its token, which the app wrote itself. Tests answer for it.
    pub(super) started_probe: fn(&WebUrl) -> crate::Result<Server>,
}

impl Default for CodeServer {
    fn default() -> Self {
        Self {
            url: None,
            state: Reach::Unknown,
            generation: 0,
            probe: code_server::probe,
            started_probe: code_server::version,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Reach {
    Unknown,
    Asking,
    Ready { server: Server, at: Instant },
    Failed { message: SharedString, at: Instant },
}

impl HerdrWindow {
    /// The focused workspace's VS Code tab, if it has one.
    pub(crate) fn code_tab_id(&self, cx: &App) -> Option<TabId> {
        let (scope, workspace) = self.browser_key()?;
        Some(store(cx)?.code_tab(&scope, &workspace)?.id)
    }

    /// Opens the focused workspace's VS Code tab in `group`, or in the group
    /// in use: a tab like any other, to split beside a terminal or show
    /// alone. A workspace has one, so opening it again shows that one here.
    /// Its page waits for its server; the tab says why until it answers.
    pub(crate) fn open_code(
        &mut self,
        group: Option<GroupId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((scope, workspace)) = self.browser_key() else {
            self.show_flash(Flash::warning("Open a workspace first"), cx);
            return;
        };
        let startup = Launcher::startup(cx, &self.config.code);
        if !super::EMBEDDED {
            self.show_flash(Flash::warning("This build cannot show pages"), cx);
            return;
        }
        if !startup.offered(&self.config.code) {
            let why = match startup {
                Startup::Finding => "Still looking for VS Code",
                _ => "Set the VS Code server in Settings",
            };
            self.show_flash(Flash::warning(why), cx);
            return;
        }
        let location = startup.url(&self.config.code).map(|url| Location::Web {
            url: self.code_start(url),
        });
        let Some(id) = Store::update(cx, |store| store.open_code_tab(scope, &workspace, location))
        else {
            self.show_flash(Flash::warning("Too many browser tabs are open"), cx);
            return;
        };
        self.show_browser_tab_in(group, id, window, cx);
    }

    /// Closes this window's VS Code pages, keeping their tabs, and forgets why
    /// any could not be created, so each is created anew.
    #[cfg(any(target_os = "macos", windows))]
    fn close_code_pages(&mut self, cx: &App) {
        let Some(store) = store(cx) else {
            return;
        };
        let code = |id: TabId| store.get(id).is_some_and(|tab| tab.place.is_code());
        let pages: Vec<TabId> = self.browser.pages.ids().filter(|id| code(*id)).collect();
        for id in pages {
            self.browser.pages.close(id);
        }
        self.browser.failed.retain(|id, _| !code(*id));
    }

    /// Runs on every window tick, and when a group shows the VS Code tab: a
    /// group showing the tab gets its page, such as one restored at startup
    /// or that of a workspace just switched to.
    pub(super) fn ensure_code_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_code_page(window, cx);
    }

    /// Whether some group of the focused workspace shows its VS Code tab.
    fn code_page_wanted(&self, cx: &App) -> bool {
        self.code_tab_id(cx)
            .is_some_and(|id| self.group_shows_page(id, cx))
    }

    /// Creates the page of the focused workspace's VS Code tab, if a group
    /// shows it and its server answers. A server the app starts is started
    /// here, the first time a page needs it. Creating a page starts the
    /// platform's web content processes, so this runs from input and ticks,
    /// never from render.
    fn open_code_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !super::EMBEDDED {
            return;
        }
        Launcher::sync(cx, &self.config.code);
        let wanted = self.code_page_wanted(cx);
        if wanted {
            Launcher::want(cx, &self.config.code);
        }
        Launcher::refresh(cx);
        let startup = Launcher::startup(cx, &self.config.code);
        let Some(url) = startup.url(&self.config.code).cloned() else {
            self.forget_code_address(cx);
            return;
        };
        let Some((scope, workspace)) = self.browser_key() else {
            return;
        };
        self.follow_code_address(&url, cx);
        // A page left on a server that stopped would show VS Code's own
        // reconnecting screen; the tab says why instead, and the page is
        // made again, where it was, once the server is back.
        #[cfg(any(target_os = "macos", windows))]
        if matches!(startup, Startup::Failed { .. }) {
            self.close_code_pages(cx);
        }
        // A tab opened before its server's address was known starts at it
        // now, on the workspace's folder.
        if let Some(tab) = store(cx)
            .and_then(|store| store.code_tab(&scope, &workspace))
            .filter(|tab| tab.location.is_none())
        {
            let (id, start) = (tab.id, self.code_start(&url));
            Store::update(cx, |store| {
                store.visited(id, Some(Location::Web { url: start }), None)
            });
        }
        let started = !matches!(startup, Startup::Address);
        // The page carries the token, so a server the app started must run
        // right now, holding its port, as the page is made.
        if !wanted
            || !startup.serving()
            || self.code_page_settled(&scope, &workspace, cx)
            || !self.code_server_ready(&url, started, cx)
            || (started && !Launcher::alive(cx))
        {
            return;
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        let _ = window;
        #[cfg(any(target_os = "macos", windows))]
        {
            let Some(mut tab) = store(cx)
                .and_then(|store| store.code_tab(&scope, &workspace))
                .cloned()
            else {
                return;
            };
            let id = tab.id;
            if let Some(Location::Web { url: page }) = &tab.location {
                tab.location = Some(Location::Web {
                    url: with_token(page, &url),
                });
            }
            if self.browser.pages.ensure(&tab, window, cx).is_err() {
                // Native errors can contain the token-bearing URL.
                tracing::warn!("Cannot create a VS Code page");
                self.browser
                    .failed
                    .insert(id, "The native page could not be created".into());
            }
            // An answer serves one page: the next one asks again, in case
            // the server has stopped since.
            self.browser.code_server.state = Reach::Unknown;
            cx.notify();
        }
    }

    /// Whether the workspace's VS Code page exists, so VS Code reconnects to
    /// its server on its own, or could not be created and is not retried
    /// every tick. Either way the server need not be asked.
    fn code_page_settled(&self, scope: &Scope, workspace: &str, cx: &App) -> bool {
        let Some(id) = store(cx)
            .and_then(|store| store.code_tab(scope, workspace))
            .map(|tab| tab.id)
        else {
            return false;
        };
        #[cfg(any(target_os = "macos", windows))]
        if self.browser.pages.contains(id) {
            return true;
        }
        self.browser.failed.contains_key(&id)
    }

    /// Starts over when `url` is a new address: it moves the VS Code tabs
    /// still on another server to this one, where they stay in their groups
    /// with their folders, and closes their pages, so each reopens there
    /// with its token.
    fn follow_code_address(&mut self, url: &WebUrl, cx: &mut Context<Self>) {
        if self.browser.code_server.url.as_ref() != Some(url) {
            let server = &mut self.browser.code_server;
            server.url = Some(url.clone());
            server.state = Reach::Unknown;
            server.generation += 1;
            Store::update(cx, |store| {
                store.follow_code_server(url, |page| on_server(page, url))
            });
            #[cfg(any(target_os = "macos", windows))]
            self.close_code_pages(cx);
        }
    }

    /// Where the focused workspace's new VS Code tab opens: the configured
    /// `url`, on the folder the workspace started in, as its first tab's
    /// first pane reports it. Only a workspace on this computer names one,
    /// since a remote one's folder is on another machine than the server.
    pub(super) fn code_start(&self, url: &WebUrl) -> WebUrl {
        let local = self
            .endpoints
            .get(self.selected_endpoint)
            .is_some_and(|endpoint| !endpoint.connection.target.is_remote());
        let folder = self
            .browser_key()
            .zip(self.live.snapshot.as_deref())
            .and_then(|((_, workspace), snapshot)| {
                crate::palette::launch_root(snapshot, &workspace)
            })
            .filter(|folder| local && std::path::Path::new(folder).is_absolute());
        folder.map_or_else(|| url.clone(), |folder| with_folder(url, folder))
    }

    /// Forgets the server once its address is removed, closing the pages
    /// still showing it, so their tabs ask for an address instead.
    fn forget_code_address(&mut self, cx: &App) {
        let server = &mut self.browser.code_server;
        if server.url.take().is_none() {
            return;
        }
        server.state = Reach::Unknown;
        server.generation += 1;
        #[cfg(any(target_os = "macos", windows))]
        self.close_code_pages(cx);
        #[cfg(not(any(target_os = "macos", windows)))]
        let _ = cx;
    }

    /// Whether the server at `url` has answered lately, asking it when
    /// nothing is known, it last answered a while ago, or it last failed a
    /// while ago. A page created against a server that has since stopped
    /// would stay blank, so an old answer is not trusted.
    /// `started` says the server is the one the app started.
    fn code_server_ready(&mut self, url: &WebUrl, started: bool, cx: &mut Context<Self>) -> bool {
        match &self.browser.code_server.state {
            Reach::Ready { at, .. } if at.elapsed() < RETRY => return true,
            Reach::Asking => return false,
            Reach::Failed { at, .. } if at.elapsed() < RETRY => return false,
            Reach::Unknown | Reach::Ready { .. } | Reach::Failed { .. } => {}
        }
        let server = &mut self.browser.code_server;
        server.state = Reach::Asking;
        let probe = if started {
            server.started_probe
        } else {
            server.probe
        };
        let (generation, url) = (server.generation, url.clone());
        let answer = cx.background_executor().spawn(async move { probe(&url) });
        cx.spawn(async move |this, cx| {
            let answer = answer.await;
            let _ = this.update(cx, |this, cx| {
                let server = &mut this.browser.code_server;
                if server.generation != generation {
                    return;
                }
                server.state = match answer {
                    Ok(server) => Reach::Ready {
                        server,
                        at: Instant::now(),
                    },
                    Err(error) => {
                        tracing::info!(%error, "The VS Code server did not answer");
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
}

#[cfg(test)]
mod tests;
