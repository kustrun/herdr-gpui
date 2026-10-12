//! Per-target workers. UI demand creates entries; only workers touch disk or SSH.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]
use super::{
    Error, Target,
    attempts::Permit,
    error::Result,
    install,
    launcher::{Phase, Startup},
    remote::Remote,
    supervisor::{Plan, Report, Status, Supervisor},
    token::Token,
};
use crate::config::CodeConfig;
use herdr_client::SavedHost;
use std::{
    collections::HashMap,
    num::NonZeroU16,
    path::Path,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
mod origin;

const MAX_ENTRIES: usize = 256;
const POLL: Duration = Duration::from_millis(100);

#[derive(Default)]
struct State {
    report: Option<Report>,
    origin: Option<NonZeroU16>,
    permanent: bool,
    phase: Option<Phase>,
}
#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    running: Mutex<Option<Running>>,
    stopped: AtomicBool,
}
impl Shared {
    fn acknowledge(
        &self,
        path: &Path,
        port: NonZeroU16,
        save_config: impl FnOnce(NonZeroU16) -> crate::Result<()>,
    ) -> crate::Result<()> {
        install::check(&self.stopped)?;
        origin::save_active(path, port, &self.stopped)?;
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .origin = Some(port);
        install::check(&self.stopped)?;
        save_config(port)
    }
    fn publish(
        &self,
        status: Status,
        address: Option<super::supervisor::Address>,
        permanent: bool,
    ) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let revision = state
            .report
            .as_ref()
            .map_or(1, |report| report.revision + 1);
        if matches!(status, Status::Ready) {
            state.phase = None;
        }
        state.report = Some(Report {
            revision,
            status,
            address,
        });
        state.permanent = permanent;
    }
    fn phase(&self, phase: Phase) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.phase = Some(phase);
        if let Some(report) = &mut state.report {
            report.revision += 1;
        }
    }
    fn sleep(&self, duration: Duration) -> bool {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            if self.stopped.load(Ordering::Acquire) {
                return true;
            }
            thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
        }
        self.stopped.load(Ordering::Acquire)
    }
}

enum Running {
    Local(Supervisor),
    Remote(Box<Remote>),
}
impl Running {
    fn alive(&mut self) -> bool {
        match self {
            Self::Local(server) => server.alive(),
            Self::Remote(server) => server.alive(),
        }
    }
    fn stop(&mut self) {
        match self {
            Self::Local(server) => server.stop(),
            Self::Remote(server) => server.stop(),
        }
    }
    fn shutdown(&self, budget: Duration) {
        if let Self::Local(server) = self {
            server.shutdown(budget);
        }
    }
}

struct Entry {
    shared: Arc<Shared>,
    generation: u64,
    configured: bool,
    independent: bool,
    requested_port: Option<NonZeroU16>,
}
impl Entry {
    fn keeps_port(&self, requested: Option<NonZeroU16>) -> bool {
        requested.is_none_or(|port| {
            Some(port)
                == self
                    .shared
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .origin
                    .or(self.requested_port)
        })
    }
    fn stop(&self) {
        self.shared.stopped.store(true, Ordering::Release);
        // Signal ready children now; owned SSH stdin also closes on app exit.
        if let Ok(mut running) = self.shared.running.try_lock()
            && let Some(running) = &mut *running
        {
            running.stop();
        }
    }
}
impl Drop for Entry {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Default)]
pub(super) struct Manager {
    entries: HashMap<Target, Entry>,
    next_generation: u64,
}
impl Manager {
    fn canonical(target: &Target) -> Target {
        if target == &Target::Configured {
            Target::Local
        } else {
            target.clone()
        }
    }
    pub(super) fn startup(&self, target: &Target, code: &CodeConfig) -> Startup {
        if target == &Target::Configured && !starts(code) {
            return Startup::Address;
        }
        if !code.license_accepted {
            return Startup::Consent;
        }
        let Some(entry) = self.entries.get(&Self::canonical(target)) else {
            return Startup::Idle;
        };
        let state = entry
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(report) = &state.report else {
            return Startup::Starting { url: None };
        };
        let url = report.address.as_ref().map(|address| address.url.clone());
        match (&report.status, url) {
            (Status::Ready, Some(url)) => Startup::Ready { url },
            (Status::Failed { error, .. }, url) => Startup::Failed {
                error: error.clone(),
                url,
            },
            (_, url) => Startup::Starting { url },
        }
    }
    pub(super) fn alive(&self, target: &Target) -> bool {
        self.entries
            .get(&Self::canonical(target))
            .is_some_and(|entry| {
                !entry.shared.stopped.load(Ordering::Acquire)
                    && entry
                        .shared
                        .running
                        .try_lock()
                        .is_ok_and(|mut running| running.as_mut().is_some_and(Running::alive))
            })
    }
    pub(super) fn phase(&self, target: &Target) -> Option<Phase> {
        self.entries
            .get(&Self::canonical(target))
            .and_then(|entry| {
                entry
                    .shared
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .phase
            })
    }
    pub(super) fn generation(&self, target: &Target) -> Option<u64> {
        self.entries
            .get(&Self::canonical(target))
            .map(|entry| entry.generation)
    }

    pub(super) fn want(
        &mut self,
        target: &Target,
        code: &CodeConfig,
        hosts: &[SavedHost],
    ) -> Result<()> {
        if !target.valid() {
            return Err(Error::TargetUnavailable);
        }
        // Validation precedes reuse, so removed/edited destinations retire ownership.
        let destination = match target.resolve(hosts) {
            Ok(destination) => destination,
            Err(error) => {
                self.entries.remove(target);
                return Err(error);
            }
        };
        if !code.license_accepted || (target == &Target::Configured && !starts(code)) {
            return Ok(());
        }
        let canonical = Self::canonical(target);
        let mut configured = target == &Target::Configured;
        let mut independent = target != &Target::Configured;
        if canonical == Target::Local
            && let Some((was_configured, was_independent)) = self.retire_moved_local(code.port)
        {
            configured |= was_configured;
            independent |= was_independent;
        }
        if let Some(entry) = self.entries.get_mut(&canonical) {
            entry.configured |= target == &Target::Configured;
            entry.independent |= target != &Target::Configured;
            return Ok(());
        }
        if self.entries.len() >= MAX_ENTRIES {
            return Err(Error::TargetLimit);
        }
        self.next_generation += 1;
        let generation = self.next_generation;
        let shared = Arc::new(Shared::default());
        let worker = shared.clone();
        let configured_port = if canonical == Target::Local {
            code.port
        } else {
            None
        };
        thread::Builder::new()
            .name("herdr-code-machine".into())
            .spawn(move || run(worker, destination, canonical, generation, configured_port))
            .map_err(Error::Worker)?;
        self.entries.insert(
            Self::canonical(target),
            Entry {
                shared,
                generation,
                configured,
                independent,
                requested_port: configured_port,
            },
        );
        Ok(())
    }
    pub(super) fn fail(&mut self, target: &Target, error: crate::Error) {
        if !self.entries.contains_key(&Self::canonical(target)) && self.entries.len() >= MAX_ENTRIES
        {
            return;
        }
        let entry = self
            .entries
            .entry(Self::canonical(target))
            .or_insert_with(|| {
                self.next_generation += 1;
                Entry {
                    shared: Arc::default(),
                    generation: self.next_generation,
                    configured: target == &Target::Configured,
                    independent: target != &Target::Configured,
                    requested_port: None,
                }
            });
        entry.shared.publish(
            Status::Failed {
                error: Arc::new(error),
                retry: Instant::now(),
            },
            None,
            true,
        );
    }

    pub(super) fn retry(&mut self, target: &Target) {
        self.entries.remove(&Self::canonical(target));
    }
    fn retire_moved_local(&mut self, port: Option<NonZeroU16>) -> Option<(bool, bool)> {
        if self.entries.get(&Target::Local)?.keeps_port(port) {
            return None;
        }
        let entry = self.entries.remove(&Target::Local)?;
        Some((entry.configured, entry.independent))
    }
    pub(super) fn sync(&mut self, code: &CodeConfig) {
        if starts(code) && code.license_accepted {
            return;
        }
        if let Some(entry) = self.entries.get_mut(&Target::Local) {
            entry.configured = false;
            if !entry.independent {
                self.entries.remove(&Target::Local);
            }
        }
        if !code.license_accepted {
            self.entries.clear();
        }
    }
    pub(super) fn sync_hosts(&mut self, hosts: &[SavedHost]) {
        self.entries
            .retain(|target, _| target.resolve(hosts).is_ok());
    }

    pub(super) fn stop_configured(&mut self) {
        if let Some(entry) = self.entries.get_mut(&Target::Local) {
            entry.configured = false;
            if !entry.independent {
                self.entries.remove(&Target::Local);
            }
        }
    }
    pub(super) fn revision(&self) -> u64 {
        self.entries
            .values()
            .map(|entry| {
                entry
                    .shared
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .report
                    .as_ref()
                    .map_or(0, |report| report.revision)
            })
            .sum()
    }
    pub(super) fn quit(self, budget: Duration) {
        for entry in self.entries.values() {
            entry.stop();
        }
        let deadline = Instant::now() + budget;
        for entry in self.entries.values() {
            if let Ok(running) = entry.shared.running.try_lock()
                && let Some(running) = &*running
            {
                running.shutdown(deadline.saturating_duration_since(Instant::now()));
            }
        }
    }
}

pub(super) fn starts(code: &CodeConfig) -> bool {
    code.mode != Some(crate::config::CodeMode::Address)
        && (code.mode.is_some() || code.url.is_none())
}

fn run(
    shared: Arc<Shared>,
    destination: Option<String>,
    target: Target,
    generation: u64,
    configured_port: Option<NonZeroU16>,
) {
    let mut failures = 0;
    loop {
        if shared.stopped.load(Ordering::Acquire) {
            return;
        }
        let result = attempt(
            &shared,
            destination.as_deref(),
            &target,
            generation,
            configured_port,
        );
        if shared.stopped.load(Ordering::Acquire) {
            return;
        }
        let Err(error) = result else {
            return;
        };
        if matches!(error, Error::Cancelled) {
            return;
        }
        let permanent = error.permanent();
        failures += 1;
        let delay = Duration::from_secs((1u64 << failures.min(6)).min(60));
        shared
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let address = {
            let state = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
            state
                .report
                .as_ref()
                .and_then(|report| report.address.clone())
        };
        shared.publish(
            Status::Failed {
                error: Arc::new(error.into()),
                retry: Instant::now() + delay,
            },
            address,
            permanent,
        );
        if permanent || shared.sleep(delay) {
            return;
        }
    }
}

fn attempt(
    shared: &Arc<Shared>,
    destination: Option<&str>,
    target: &Target,
    generation: u64,
    configured_port: Option<NonZeroU16>,
) -> Result<()> {
    let Some(permit) = Permit::acquire(|| shared.stopped.load(Ordering::Acquire)) else {
        return Err(Error::Cancelled);
    };
    shared.publish(Status::Starting, None, false);
    shared.phase(Phase::Preparing);
    let root = crate::preferences::state_dir()
        .ok_or(Error::NoDataDir)?
        .join("code-machines")
        .join(target.key());
    super::private::directory(&root)?;
    let token_file = if target == &Target::Local {
        super::token::path()?
    } else {
        root.join("token")
    };
    let token = if target == &Target::Local {
        Token::load_or_create(&token_file)?
    } else {
        Token::load_private(&token_file)?
    };
    let origin_file = root.join("origin");
    let persisted = origin::load(&origin_file)?;
    let port = configured_port.or(persisted);
    if configured_port.is_none() {
        shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .origin = persisted;
    }
    let running = match destination {
        None => {
            let data = root.join("data");
            super::private::directory(&data)?;
            let program = install::ensure(&root.join("cli"), &shared.stopped, || {
                shared.phase(Phase::Installing);
            })?;
            shared.phase(Phase::Starting);
            drop(permit);
            let saved = origin_file.clone();
            let acknowledged = shared.clone();
            let server = Supervisor::start_with(
                Plan {
                    program,
                    port,
                    token_file,
                },
                move |plan, port| {
                    let mut command = super::supervisor::serve_web(plan, port);
                    command.arg("--server-data-dir").arg(&data);
                    crate::login_env::apply(&mut command);
                    command
                },
                super::version,
                move |port| {
                    acknowledged.acknowledge(&saved, port, |port| {
                        if crate::config::Config::save_managed_code_port(
                            port,
                            &acknowledged.stopped,
                        )? {
                            Ok(())
                        } else {
                            Err(Error::Cancelled.into())
                        }
                    })
                },
                super::supervisor::Timing::default(),
            )
            .map_err(|error| match error {
                crate::Error::Code(error) => error,
                _ => Error::RemoteOwnership,
            })?;
            Running::Local(server)
        }
        Some(destination) => {
            let server = Remote::start(
                destination,
                &target.key(),
                generation,
                &token,
                port,
                &shared.stopped,
                |event| match event {
                    super::remote::Event::Installing => {
                        shared.phase(Phase::Installing);
                        Ok(())
                    }
                    super::remote::Event::Starting => {
                        shared.phase(Phase::Starting);
                        Ok(())
                    }
                    super::remote::Event::Origin(port) => {
                        origin::save_active(&origin_file, port, &shared.stopped)
                    }
                },
            )?;
            // Keep the attempt permit through token-free HTTP readiness below.
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut server = server;
            while !server.readiness(super::version)? {
                if shared.sleep(POLL) {
                    return Err(Error::Cancelled);
                }
                if Instant::now() >= deadline {
                    return Err(Error::Silent(Duration::from_secs(30)));
                }
            }
            server.tick()?;
            drop(permit);
            shared.publish(Status::Ready, Some(server.address.clone()), false);
            Running::Remote(Box::new(server))
        }
    };
    *shared
        .running
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(running);
    while !shared.sleep(POLL) {
        let mut running = shared
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match running.as_mut() {
            Some(Running::Local(server)) => {
                let report = server.report();
                let acknowledged = shared
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .origin;
                if matches!(report.status, Status::Ready)
                    && let Some(address) = &report.address
                    && acknowledged != Some(address.port)
                {
                    // Explicit ports still acknowledge the new stable origin,
                    // without changing the user's already persisted config.
                    shared
                        .acknowledge(&origin_file, address.port, |_| Ok(()))
                        .map_err(|error| match error {
                            crate::Error::Code(error) => error,
                            _ => Error::RemoteOwnership,
                        })?;
                }
                let permanent = matches!(&report.status, Status::Failed { error, .. } if matches!(error.as_ref(), crate::Error::Code(error) if error.permanent()));
                let mut state = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.permanent = permanent;
                match report.status {
                    Status::Ready => state.phase = None,
                    Status::Starting => state.phase = Some(Phase::Starting),
                    Status::Failed { .. } => {}
                }
                state.report = Some(report);
            }
            Some(Running::Remote(server)) => server.tick()?,
            None => return Err(Error::Cancelled),
        }
    }
    shared
        .running
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    Ok(())
}

#[cfg(test)]
mod tests;
