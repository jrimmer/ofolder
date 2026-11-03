//! What the rails and their menu share: one view-model, one poll of the
//! dialog, one place that runs the jobs that touch the dialog, and the one
//! menu window that may be open. Each window ([`crate::left`],
//! [`crate::right`], [`crate::popup`]) only draws and forwards clicks.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AnyView, App, Context, ElementId, MouseButton, Render, Stateful, Window, WindowHandle,
    WindowId, div, prelude::*, px, rgb,
};

use std::path::{Path, PathBuf};

use ofolder_core::{browse, search};

use crate::colors::colors;
use crate::expand::{RowKey, Section};
use crate::menu::RowAction;
use crate::model::{Effect, Job, Mode, ViewModel};
use crate::popup::MenuWindow;
use crate::rail::{self, Change, MenuKind, Menus};
use crate::sources::RecentsRead;
use crate::{MenuRequest, StripActions, menu_options};

/// How often the dialog is asked for its folder: about five times a second.
const POLL: Duration = Duration::from_millis(200);
/// Favorites are checked for existence and the recents reread every this
/// many polls (two seconds), because a stat on a slow mount can take a while.
const RECENTS_EVERY: u32 = 10;
/// The drives are reread every this many polls (three seconds).
const PLACES_EVERY: u32 = 15;
/// How long none of the strip's windows may hold the keyboard before an open
/// menu closes. Moving from a rail to its menu passes through a moment when
/// neither is active, and that must not close it.
const LEAVE_GRACE: Duration = Duration::from_millis(350);
/// Where the message window's top goes: past the rail's foot, which places
/// it at the bottom of the right rail.
const TOAST_ANCHOR: i32 = 100_000;

/// One of the strip's windows, for tracking which holds the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Left,
    Right,
    /// A menu window, by the serial it was opened with.
    Menu(u64),
}

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

/// The state the rails and the menu show.
pub struct Shared {
    pub vm: ViewModel,
    actions: Arc<dyn StripActions>,
    menus: Menus,
    /// The open menu's window, once it has opened.
    menu_window: Option<WindowHandle<MenuWindow>>,
    /// Counts menus opened, so a closed menu's late reports are ignored.
    menu_serial: u64,
    /// The size last asked for the open menu.
    menu_size: Option<(i32, i32)>,
    /// Whether the left rail, the right rail and the open menu hold the keyboard.
    active: [bool; 3],
    /// Counts changes of `active`, so a pending close is dropped when the
    /// keyboard comes back in time.
    leave_check: u64,
    /// The short backend name, e.g. "flea", for the About menu.
    pub label: String,
}

impl Shared {
    pub fn new(vm: ViewModel, actions: Arc<dyn StripActions>, cx: &mut Context<Self>) -> Self {
        let shared = Shared {
            vm,
            actions,
            menus: Menus::default(),
            menu_window: None,
            menu_serial: 0,
            menu_size: None,
            active: [false; 3],
            leave_check: 0,
            label: String::new(),
        };
        shared.start_polling(cx);
        shared
    }

    // ---- the menu ---------------------------------------------------------

    pub fn open_menu(&self) -> Option<MenuKind> {
        self.menus.open()
    }

    /// The lines a menu shows now, to size its window.
    pub fn menu_rows(&self, kind: MenuKind) -> usize {
        let lines = |section| self.vm.section_lines(section).len();
        match kind {
            MenuKind::Favorites => lines(Section::Favorites).max(1),
            MenuKind::Places => lines(Section::Places),
            MenuKind::RecentFolders => lines(Section::RecentFolders).max(1),
            MenuKind::RecentFiles => lines(Section::RecentFiles).max(1),
            MenuKind::Find => lines(Section::Ranked) + lines(Section::Found),
            MenuKind::NewFolder | MenuKind::About | MenuKind::Toast => 0,
        }
    }

    fn request(&self, kind: MenuKind, size: (i32, i32), fresh: bool) -> MenuRequest {
        MenuRequest {
            side: kind.side(),
            anchor_y: kind.button().map(rail::anchor_y).unwrap_or(TOAST_ANCHOR),
            width: size.0,
            height: size.1,
            takes_keyboard: kind.takes_keyboard(),
            fresh,
            away_from_dialog: kind == MenuKind::Toast,
        }
    }

    /// A click on a rail icon that has a menu: opens it, or closes it when
    /// it is the one open.
    pub fn click_menu(&mut self, kind: MenuKind, cx: &mut Context<Self>) {
        match self.menus.click(kind) {
            Change::Open(kind) => {
                self.cancel_prompt();
                self.close_window(cx);
                if kind == MenuKind::NewFolder && !self.vm.start_new_folder() {
                    self.menus.close();
                    self.changed(cx);
                    return;
                }
                self.open_window(kind, cx);
            }
            Change::Close => {
                self.shut_menu(cx);
                self.changed(cx);
            }
        }
        cx.notify();
    }

    fn open_window(&mut self, kind: MenuKind, cx: &mut Context<Self>) {
        let size = (kind.width(), kind.height(self.menu_rows(kind)));
        self.menu_size = Some(size);
        self.menu_serial += 1;
        let serial = self.menu_serial;
        self.active[2] = false;
        // The placement rule must be in force before the window maps.
        self.actions
            .place_menu(Some(self.request(kind, size, true)));
        let shared = cx.entity();
        // Opened after this update ends: the menu's view reads the shared
        // state as it is built.
        cx.defer(move |cx| {
            let options = menu_options(size, cx);
            let menu = shared.clone();
            let opened = cx.open_window(options, move |window, cx| {
                cx.new(|cx| MenuWindow::new(menu, kind, serial, window, cx))
            });
            shared.update(cx, |s, cx| match opened {
                // A menu closed or replaced before its window came up
                // takes the window with it.
                Ok(handle) if s.menu_serial == serial && s.menus.open() == Some(kind) => {
                    s.menu_window = Some(handle);
                }
                Ok(handle) => {
                    handle.update(cx, |_, w, _| w.remove_window()).ok();
                }
                Err(error) => {
                    eprintln!("ofolder: could not open a menu: {error}");
                    s.menus.close();
                    s.actions.place_menu(None);
                    cx.notify();
                }
            });
        });
    }

    fn close_window(&mut self, cx: &mut Context<Self>) {
        self.active[2] = false;
        self.menu_size = None;
        if let Some(handle) = self.menu_window.take() {
            cx.defer(move |cx| {
                handle.update(cx, |_, w, _| w.remove_window()).ok();
            });
        }
    }

    /// A prompt left open when its menu goes is abandoned.
    fn cancel_prompt(&mut self) {
        if self.vm.mode() != Mode::Browse {
            self.vm.cancel();
        }
    }

    /// Closes the open menu, if any. A message still showing then gets its
    /// own small window.
    pub fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menus.close().is_some() {
            self.shut_menu(cx);
        }
        self.changed(cx);
    }

    /// Takes down the window of a menu already marked closed.
    fn shut_menu(&mut self, cx: &mut Context<Self>) {
        self.cancel_prompt();
        self.close_window(cx);
        self.actions.place_menu(None);
    }

    /// After a row, a result or a prompt was used: the menu has done its job.
    pub fn after_choice(&mut self, cx: &mut Context<Self>) {
        if self.menus.open().is_some_and(|k| k != MenuKind::Toast) {
            self.close_menu(cx);
        }
    }

    /// Opens the message window when a message waits and nothing is open,
    /// and closes it when the message has gone; then redraws.
    pub fn changed(&mut self, cx: &mut Context<Self>) {
        let waiting = self.vm.message().is_some();
        if waiting && self.menus.show_toast() {
            self.open_window(MenuKind::Toast, cx);
        } else if !waiting && self.menus.open() == Some(MenuKind::Toast) {
            self.menus.close();
            self.shut_menu(cx);
        }
        cx.notify();
    }

    /// The menu window grows and shrinks with its content.
    pub fn fit_menu(&mut self) {
        let Some(kind) = self.menus.open() else {
            return;
        };
        let size = (kind.width(), kind.height(self.menu_rows(kind)));
        if self.menu_size != Some(size) {
            self.menu_size = Some(size);
            self.actions
                .place_menu(Some(self.request(kind, size, false)));
        }
    }

    /// Called when any window closes. A menu window that went without being
    /// asked to (closed by the compositor) leaves no menu open.
    pub fn menu_window_closed(&mut self, open: &[WindowId], cx: &mut Context<Self>) {
        if let Some(handle) = self.menu_window
            && !open.contains(&handle.window_id())
        {
            self.menu_window = None;
            self.menus.close();
            self.cancel_prompt();
            self.actions.place_menu(None);
            cx.notify();
        }
    }

    /// Notes which window holds the keyboard. When none of the strip's does
    /// for a moment (the user went back to the dialog or elsewhere), the open
    /// menu closes. The message window is left to time out instead.
    pub fn set_active(&mut self, which: Which, active: bool, cx: &mut Context<Self>) {
        match which {
            Which::Left => self.active[0] = active,
            Which::Right => self.active[1] = active,
            Which::Menu(serial) if serial == self.menu_serial => self.active[2] = active,
            Which::Menu(_) => return,
        }
        self.leave_check += 1;
        if self.active.iter().any(|a| *a)
            || !self.menus.open().is_some_and(|k| k != MenuKind::Toast)
        {
            return;
        }
        let check = self.leave_check;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LEAVE_GRACE).await;
            this.update(cx, |s, cx| {
                if s.leave_check == check && !s.active.iter().any(|a| *a) {
                    s.after_choice(cx);
                }
            })
            .ok();
        })
        .detach();
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
                        t.changed(cx);
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
                    t.changed(cx);
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
                    t.changed(cx);
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
                    t.changed(cx);
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
                    t.changed(cx);
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
        self.changed(cx);
    }

    pub fn start(&mut self, job: Option<Job>, cx: &mut Context<Self>) {
        if let Some(job) = job {
            self.run(job, cx);
        }
        self.changed(cx);
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
        if !(action.keeps_menu_open() || action == RowAction::Rename) {
            self.after_choice(cx);
        }
        self.changed(cx);
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

/// How a rail icon looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconLook {
    Enabled,
    /// Dimmed; the caller adds no click.
    Disabled,
    /// Its menu is open.
    Open,
    /// A switch that is on, such as the favorite toggle for a favorite.
    On,
}

/// A rail icon at its slot's position, with its tooltip. `icon` overrides
/// the button's own icon file (the favorite toggle shows a filled star).
pub fn rail_icon(
    button: rail::Button,
    y: f32,
    look: IconLook,
    tooltip: String,
    icon: Option<&'static str>,
) -> Stateful<gpui::Div> {
    let inset = (rail::RAIL_WIDTH - rail::BUTTON) / 2.;
    let (color, bg) = match look {
        IconLook::Enabled => (colors().text, None),
        IconLook::Disabled => (colors().muted, None),
        IconLook::Open => (colors().accent, Some(colors().chosen)),
        IconLook::On => (colors().accent, None),
    };
    let base = div()
        .id(button.id())
        .absolute()
        .top(px(y))
        .left(px(inset))
        .size(px(rail::BUTTON))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .when_some(bg, |d, bg| d.bg(rgb(bg)))
        // A press on an icon is not a press on the dead space around it,
        // which closes an open menu.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .tooltip(tip(tooltip))
        .child(
            gpui::svg()
                .path(icon.unwrap_or(button.icon()))
                .size(px(20.))
                .text_color(rgb(color)),
        );
    match look {
        IconLook::Disabled => base.opacity(0.45),
        IconLook::Enabled | IconLook::On => {
            base.cursor_pointer().hover(|s| s.bg(rgb(colors().hover)))
        }
        IconLook::Open => base.cursor_pointer(),
    }
}

/// A thin line across a rail between groups of icons.
pub fn rail_divider(y: f32) -> gpui::Div {
    div()
        .absolute()
        .top(px(y + rail::DIVIDER / 2.))
        .left(px(12.))
        .w(px(rail::RAIL_WIDTH - 24.))
        .h(px(1.))
        .bg(rgb(colors().raised))
}
