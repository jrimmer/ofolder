//! What the two panels share: one view-model, one poll of the dialog, one
//! place that runs the jobs that touch the dialog. Each panel
//! ([`crate::left`], [`crate::right`]) only draws and forwards clicks.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AnyView, App, Context, ElementId, Render, Stateful, Window, div, prelude::*, px, rgb};

use std::path::{Path, PathBuf};

use ofolder_core::{browse, search};

use crate::StripActions;
use crate::colors::colors;
use crate::expand::RowKey;
use crate::menu::RowAction;
use crate::model::{Effect, Job, Mode, ViewModel};
use crate::sources::RecentsRead;

/// How often the dialog is asked for its folder: about five times a second.
const POLL: Duration = Duration::from_millis(200);
/// Favorites are checked for existence and the recents reread every this
/// many polls (two seconds), because a stat on a slow mount can take a while.
const RECENTS_EVERY: u32 = 10;
/// The drives are reread every this many polls (three seconds).
const PLACES_EVERY: u32 = 15;

struct Tip(String);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(600.))
            .px_2()
            .py_1()
            .rounded(px(4.))
            .bg(rgb(colors().tooltip_bg))
            .border_1()
            .border_color(rgb(colors().hover))
            .text_size(px(12.))
            .text_color(rgb(colors().text))
            .child(self.0.clone())
    }
}

/// A hover tooltip with the given text.
pub fn tip(text: String) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| cx.new(|_| Tip(text.clone())).into()
}

/// A small button; a disabled one is dimmed and the caller adds no click.
pub fn button(id: impl Into<ElementId>, label: &str, enabled: bool) -> Stateful<gpui::Div> {
    let base = div()
        .id(id)
        .px_2()
        .py(px(3.))
        .rounded(px(4.))
        .text_size(px(12.))
        .whitespace_nowrap()
        .bg(rgb(colors().raised))
        .child(label.to_owned());
    if enabled {
        base.cursor_pointer()
            .hover(|s| s.bg(rgb(colors().hover)))
            .text_color(rgb(colors().text))
    } else {
        base.text_color(rgb(colors().muted)).opacity(0.5)
    }
}

/// The state both panels show.
pub struct Shared {
    pub vm: ViewModel,
    actions: Arc<dyn StripActions>,
}

impl Shared {
    pub fn new(vm: ViewModel, actions: Arc<dyn StripActions>, cx: &mut Context<Self>) -> Self {
        let shared = Shared { vm, actions };
        shared.start_polling(cx);
        shared
    }

    /// Asks the dialog for its folder on a background thread, rereads the
    /// lists now and then, and applies the answers here. Ends the process
    /// when the dialog is gone.
    fn start_polling(&self, cx: &mut Context<Self>) {
        let actions = self.actions.clone();
        let sources = self.vm.sources();
        let read_recents = self.vm.recents_enabled();
        cx.spawn(async move |this, cx| {
            let mut tick: u32 = 0;
            loop {
                let lists = tick.is_multiple_of(RECENTS_EVERY);
                let places = tick.is_multiple_of(PLACES_EVERY);
                tick = tick.wrapping_add(1);
                let Ok(paths) = this.update(cx, |t, _| t.vm.favorite_paths()) else {
                    return;
                };
                let actions = actions.clone();
                let sources = sources.clone();
                let read = cx
                    .background_executor()
                    .spawn(async move {
                        let folder = actions.current_folder();
                        let alive = actions.dialog_alive();
                        let seen = lists.then(|| {
                            paths
                                .into_iter()
                                .map(|p| {
                                    let there = p.is_dir();
                                    (p, there)
                                })
                                .collect::<Vec<_>>()
                        });
                        let recents = (lists && read_recents).then(|| sources.read_recents());
                        let places = places.then(|| sources.read_places());
                        Polled {
                            folder,
                            alive,
                            seen,
                            recents,
                            places,
                        }
                    })
                    .await;
                let applied = this.update(cx, |t, cx| {
                    if !read.alive {
                        cx.quit();
                        return;
                    }
                    let mut changed = t.vm.set_folder(read.folder);
                    if let Some(seen) = read.seen {
                        changed |= t.vm.set_availability(seen);
                    }
                    if let Some(recents) = read.recents {
                        changed |= t.vm.set_recents(recents);
                    }
                    if let Some(places) = read.places {
                        changed |= t.vm.set_places(places);
                    }
                    changed |= t.vm.expire_message(Instant::now());
                    if changed {
                        cx.notify();
                    }
                });
                if applied.is_err() {
                    return;
                }
                cx.background_executor().timer(POLL).await;
            }
        })
        .detach();
    }

    /// Runs a job that touches the dialog or starts a program, off the UI
    /// thread.
    pub fn run(&mut self, job: Job, cx: &mut Context<Self>) {
        let actions = self.actions.clone();
        cx.spawn(async move |this, cx| match job {
            Job::Go(path) => {
                let result = cx
                    .background_executor()
                    .spawn(async move { actions.go(&path) })
                    .await;
                this.update(cx, |t, cx| {
                    t.vm.finish_go(result);
                    cx.notify();
                })
                .ok();
            }
            Job::NewFolder(name) => {
                let result = cx
                    .background_executor()
                    .spawn(async move { actions.new_folder(&name) })
                    .await;
                this.update(cx, |t, cx| {
                    t.vm.finish_new_folder(result);
                    cx.notify();
                })
                .ok();
            }
            Job::OpenFileManager(path) => {
                let result = cx
                    .background_executor()
                    .spawn(async move { actions.open_in_file_manager(&path) })
                    .await;
                this.update(cx, |t, cx| {
                    t.vm.finish_launch("file manager", result);
                    cx.notify();
                })
                .ok();
            }
            Job::OpenTerminal(path) => {
                let result = cx
                    .background_executor()
                    .spawn(async move { actions.open_in_terminal(&path) })
                    .await;
                this.update(cx, |t, cx| {
                    t.vm.finish_launch("terminal", result);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Reads a row's subfolders off the UI thread.
    fn read_subfolders(&mut self, key: RowKey, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let path = key.path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { browse::subfolders(&path, false) })
                .await;
            this.update(cx, |t, cx| {
                t.vm.finish_browse(&key, result);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Runs a search below the current folder off the UI thread.
    pub fn search(&mut self, cx: &mut Context<Self>) {
        let Some(job) = self.vm.start_search() else {
            cx.notify();
            return;
        };
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    search::search(
                        &job.root,
                        &job.query,
                        search::Limits::default(),
                        &job.cancel,
                    )
                })
                .await;
            this.update(cx, |t, cx| {
                let found = outcome.found.into_iter().map(|f| f.path).collect();
                t.vm.finish_search(job.id, found, outcome.ended);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn stop_search(&mut self, cx: &mut Context<Self>) {
        self.vm.stop_search();
        cx.notify();
    }

    /// Applies text typed into a prompt, for whichever prompt is open.
    pub fn submit(&mut self, text: &str, cx: &mut Context<Self>) {
        match self.vm.mode() {
            Mode::NewFolder => {
                if let Some(job) = self.vm.submit_new_folder(text) {
                    self.run(job, cx);
                }
            }
            Mode::Rename(_) => self.vm.submit_rename(text),
            Mode::Browse => {}
        }
        cx.notify();
    }

    pub fn start(&mut self, job: Option<Job>, cx: &mut Context<Self>) {
        if let Some(job) = job {
            self.run(job, cx);
        }
        cx.notify();
    }

    pub fn expand(&mut self, key: RowKey, cx: &mut Context<Self>) {
        if let Some(key) = self.vm.toggle_expand(key) {
            self.read_subfolders(key, cx);
        }
        cx.notify();
    }

    /// Runs a menu action; returns text for the clipboard when it has any.
    pub fn menu_action(
        &mut self,
        action: RowAction,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let mut copy = None;
        match self.vm.row_action(action, path) {
            Effect::Job(job) => self.run(job, cx),
            Effect::Copy(text) => copy = Some(text),
            Effect::None => {}
        }
        cx.notify();
        copy
    }
}

struct Polled {
    folder: Option<PathBuf>,
    alive: bool,
    seen: Option<Vec<(PathBuf, bool)>>,
    recents: Option<RecentsRead>,
    places: Option<Vec<ofolder_core::places::Place>>,
}
