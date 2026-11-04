//! What the rails and their menu share: one view-model, one poll of the
//! dialog, one place that runs the jobs that touch the dialog, and the one
//! menu window that may be open. Each window ([`crate::left`],
//! [`crate::right`], [`crate::popup`]) only draws and forwards clicks.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gpui::{
    AnyView, App, ClipboardItem, Context, ElementId, Entity, MouseButton, Render, Stateful, Window,
    WindowHandle, WindowId, div, prelude::*, px, rgb,
};

use std::path::{Path, PathBuf};

use ofolder_core::{browse, messages, search};

use crate::colors::colors;
use crate::expand::{RowKey, Section};
use crate::hint::{self, Hint, Hints, Step, TipAnchor, TipWindow};
use crate::menu::RowAction;
use crate::model::{Effect, Job, Mode, ViewModel};
use crate::popup::MenuWindow;
use crate::preview::{self as panel, Content, PreviewWindow};
use crate::rail::{self, Button, Change, Layout, MenuKind, Menus, RailSide};
use crate::settings_window::{self, SettingsWindow};
use crate::sources::RecentsRead;
use crate::{
    MenuRequest, SettingsRequest, StripActions, TipRequest, menu_options, preview_options,
    settings_options, tip_options,
};
use ofolder_core::preview::{PreviewSettings, Settle, classify};
use ofolder_core::rails::RailLayout;
use ofolder_core::shortcuts::{Combo, Shortcuts};

/// Polls between two reads of the selection for a dialog whose answer is slow.
const SELECTION_EVERY: u32 = 3;
/// How often the dialog is asked for its folder: about five times a second.
const POLL: Duration = Duration::from_millis(200);
/// How often commands from shortcuts are picked up: soon enough to feel
/// immediate.
const LISTEN: Duration = Duration::from_millis(40);
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
    /// The short backend name, e.g. "flea", for the settings window.
    pub label: String,
    /// The settings file, which the settings window reads and writes.
    pub settings_path: PathBuf,
    pub home: PathBuf,
    /// The rail layout as the settings hold it, and as the rails draw it.
    pub rails: RailLayout,
    pub layout: Layout,
    /// The settings window, once it has opened; `settings_open` is set as
    /// soon as it is asked for.
    settings_window: Option<WindowHandle<SettingsWindow>>,
    settings_open: bool,
    /// Counts settings windows opened and closed, so a late one is taken down.
    settings_serial: u64,
    /// Which rail icon's tooltip is wanted or showing.
    hints: Hints,
    /// The tooltip's window, once it has opened.
    tip_window: Option<WindowHandle<TipWindow>>,
    /// Counts tooltips opened and closed, so a late window is taken down.
    tip_serial: u64,
    /// Shortcuts that could not be bound, by command id, with why.
    pub shortcut_problems: Vec<(String, String)>,
    /// The message log, when one is kept.
    pub messages_path: Option<PathBuf>,
    /// What the preview panel shows, once loaded; `None` while loading.
    pub preview_content: Option<Content>,
    preview_settings: PreviewSettings,
    /// Follows the selection, letting the panel change only once it holds.
    preview_settle: Settle,
    /// The panel's window, once it has opened; `preview_open` is set as soon
    /// as it is asked for.
    preview_window: Option<WindowHandle<PreviewWindow>>,
    preview_open: bool,
    /// Counts panel windows opened and closed, so a late one is taken down.
    preview_serial: u64,
    /// Counts loads, so one the selection moved on from is dropped.
    preview_generation: Arc<AtomicU64>,
    /// Hidden for this dialog by the `preview` shortcut.
    preview_hidden: bool,
}

impl Shared {
    /// What the strip's actions are, for work that must not run on the UI thread.
    pub fn actions(&self) -> Arc<dyn StripActions> {
        self.actions.clone()
    }

    pub fn new(
        vm: ViewModel,
        actions: Arc<dyn StripActions>,
        settings_path: PathBuf,
        home: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        let rails = vm.rails().clone();
        let preview_settings = vm.preview_settings();
        let layout = Layout::from_core(&rails);
        let mut shared = Shared {
            vm,
            actions,
            menus: Menus::default(),
            menu_window: None,
            menu_serial: 0,
            menu_size: None,
            active: [false; 3],
            leave_check: 0,
            label: String::new(),
            settings_path,
            home,
            rails,
            layout,
            settings_window: None,
            settings_open: false,
            settings_serial: 0,
            hints: Hints::default(),
            tip_window: None,
            tip_serial: 0,
            shortcut_problems: Vec::new(),
            messages_path: None,
            preview_content: None,
            preview_settings,
            preview_settle: Settle::default(),
            preview_window: None,
            preview_open: false,
            preview_serial: 0,
            preview_generation: Arc::new(AtomicU64::new(0)),
            preview_hidden: false,
        };
        shared.shortcut_problems = shared.actions.shortcut_problems();
        shared.start_polling(cx);
        shared.start_listening(cx);
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
            MenuKind::NewFolder => 0,
            MenuKind::Toast => self.vm.message().map_or(1, |m| rail::toast_lines(&m.text)),
        }
    }

    fn request(&self, kind: MenuKind, size: (i32, i32), fresh: bool) -> MenuRequest {
        MenuRequest {
            side: kind.side(),
            anchor_y: kind
                .button()
                .map(|b| rail::anchor_y(&self.layout, b))
                .unwrap_or(TOAST_ANCHOR),
            width: size.0,
            height: size.1,
            takes_keyboard: kind.takes_keyboard(),
            fresh,
            away_from_dialog: kind == MenuKind::Toast,
        }
    }

    // ---- commands: icon clicks and shortcuts ---------------------------------

    /// A rail icon was pressed, by click or by shortcut. Does what the icon
    /// does, unless the icon is switched off now (folder unknown, busy, no
    /// history).
    pub fn press(&mut self, button: Button, cx: &mut Context<Self>) {
        self.press_from(button, false, cx);
    }

    /// `from_shortcut`: the press came from a key binding, with no input
    /// event the clipboard can be set in answer to.
    fn press_from(&mut self, button: Button, from_shortcut: bool, cx: &mut Context<Self>) {
        let known = self.vm.folder().is_some();
        let busy = self.vm.busy();
        if let Some(kind) = button.menu() {
            let allowed = match button.side() {
                RailSide::Left => true,
                RailSide::Right => known && !(busy && button == Button::NewFolder),
            };
            if allowed || self.open_menu() == Some(kind) {
                self.click_menu(kind, cx);
            }
            return;
        }
        match button {
            Button::Back | Button::Forward | Button::Parent => {
                let can = match button {
                    Button::Back => self.vm.can_go_back(),
                    Button::Forward => self.vm.can_go_forward(),
                    _ => self.vm.can_go_up(),
                };
                if busy || !can {
                    if std::env::var_os("OFOLDER_DEBUG").is_some() {
                        eprintln!(
                            "ofolder strip: ignored {button:?} (busy: {busy}, possible: {can})"
                        );
                    }
                    return;
                }
                self.close_menu(cx);
                let job = match button {
                    Button::Back => self.vm.go_back(),
                    Button::Forward => self.vm.go_forward(),
                    _ => self.vm.go_up(),
                };
                self.start(job, cx);
            }
            _ if !known => {}
            _ => {
                self.close_menu(cx);
                let copied = match button {
                    Button::Favorite => {
                        self.vm.toggle_current_favorite();
                        None
                    }
                    Button::CopyPath => self.vm.copy_path(),
                    Button::FileManager => {
                        let job = self.vm.open_current(false);
                        self.start(job, cx);
                        None
                    }
                    _ => {
                        let job = self.vm.open_current(true);
                        self.start(job, cx);
                        None
                    }
                };
                if let Some(text) = copied {
                    if from_shortcut {
                        if let Err(e) = self.actions.copy_text(&text) {
                            self.vm
                                .show_error("copy-path", &format!("Could not copy: {e}"));
                        }
                    } else {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }
                self.changed(cx);
            }
        }
    }

    /// Runs a shortcut's command: an icon's, or `settings` for the gear.
    pub fn run_command(&mut self, id: &str, cx: &mut Context<Self>) {
        self.hide_tip(cx);
        if id == "settings" {
            self.toggle_settings(cx);
        } else if id == "preview" {
            self.toggle_preview(cx);
        } else if let Some(button) = Button::from_key(id) {
            self.press_from(button, true, cx);
        }
    }

    /// Picks up commands from keyboard shortcuts as they arrive.
    fn start_listening(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(LISTEN).await;
                let alive = this.update(cx, |s, cx| {
                    for id in s.actions.take_commands() {
                        if std::env::var_os("OFOLDER_DEBUG").is_some() {
                            eprintln!("ofolder strip: command {id}");
                        }
                        s.run_command(id, cx);
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// Binds `shortcuts` in place of the current ones and keeps what could
    /// not be bound, for the settings window.
    pub fn rebind(&mut self, shortcuts: &Shortcuts, cx: &mut Context<Self>) {
        self.shortcut_problems = self.actions.rebind(shortcuts);
        cx.notify();
    }

    /// What the user's own configuration does with `combo`, if it binds it.
    pub fn taken_by(&self, combo: &Combo) -> Option<String> {
        self.actions.taken_by(combo)
    }

    // ---- the settings window and the layout --------------------------------

    pub fn settings_open(&self) -> bool {
        self.settings_open
    }

    /// The gear: opens the settings window, or closes it when it is open.
    pub fn toggle_settings(&mut self, cx: &mut Context<Self>) {
        self.hide_tip(cx);
        if self.settings_open {
            self.close_settings(cx);
            return;
        }
        self.close_menu(cx);
        self.settings_open = true;
        self.settings_serial += 1;
        let serial = self.settings_serial;
        let size = settings_window::SIZE;
        // The placement rule must be in force before the window maps.
        self.actions.place_settings(Some(SettingsRequest {
            width: size.0,
            height: size.1,
        }));
        let shared = cx.entity();
        cx.defer(move |cx| {
            let options = settings_options(size, cx);
            let inner = shared.clone();
            let opened = cx.open_window(options, move |window, cx| {
                cx.new(|cx| SettingsWindow::new(inner, window, cx))
            });
            shared.update(cx, |s, cx| match opened {
                Ok(handle) if s.settings_serial == serial && s.settings_open => {
                    s.settings_window = Some(handle);
                }
                // Closed again before its window came up.
                Ok(handle) => {
                    // Not here: the window's closing is reported to this same state,
                    // which is being updated now.
                    cx.defer(move |cx| {
                        handle.update(cx, |_, w, _| w.remove_window()).ok();
                    });
                }
                Err(error) => {
                    eprintln!("ofolder: could not open the settings: {error}");
                    s.settings_open = false;
                    s.actions.place_settings(None);
                    cx.notify();
                }
            });
        });
        cx.notify();
    }

    pub fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.settings_serial += 1;
        if let Some(handle) = self.settings_window.take() {
            cx.defer(move |cx| {
                handle.update(cx, |_, w, _| w.remove_window()).ok();
            });
        }
        self.actions.place_settings(None);
        cx.notify();
    }

    /// Called when any window closes: a settings window closed by the
    /// compositor is forgotten.
    pub fn settings_window_closed(&mut self, open: &[WindowId], cx: &mut Context<Self>) {
        if self
            .settings_window
            .is_some_and(|h| !open.contains(&h.window_id()))
        {
            self.settings_window = None;
            self.settings_open = false;
            self.actions.place_settings(None);
            cx.notify();
        }
    }

    /// A new rail layout from the settings window: the rails redraw at once,
    /// and an open menu or tooltip, anchored to the old places, goes.
    pub fn set_rails(&mut self, rails: RailLayout, cx: &mut Context<Self>) {
        self.layout = Layout::from_core(&rails);
        self.rails = rails;
        self.hide_tip(cx);
        self.close_menu(cx);
        cx.notify();
    }

    // ---- tooltips ---------------------------------------------------------

    /// The pointer entered (`hovered`) or left a rail icon.
    pub fn hover_tip(&mut self, hint: Hint, hovered: bool, cx: &mut Context<Self>) {
        let step = if hovered {
            self.hints.enter(hint)
        } else {
            self.hints.leave(hint.key)
        };
        match step {
            Step::Wait(serial) => {
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(hint::DELAY_MS))
                        .await;
                    this.update(cx, |s, cx| {
                        if let Some(hint) = s.hints.due(serial).cloned() {
                            s.show_tip(hint, cx);
                        }
                    })
                    .ok();
                })
                .detach();
            }
            Step::ShowNow => {
                if let Some(hint) = self.hints.wanted().cloned() {
                    self.show_tip(hint, cx);
                }
            }
            Step::Hide => self.close_tip_window(cx),
            Step::Nothing => {}
        }
    }

    /// No tooltip, and none pending: after a click, when a menu opens, or
    /// when the keyboard leaves the rails.
    pub fn hide_tip(&mut self, cx: &mut Context<Self>) {
        self.hints.clear();
        self.close_tip_window(cx);
    }

    fn show_tip(&mut self, hint: Hint, cx: &mut Context<Self>) {
        self.close_tip_window(cx);
        let lines = hint::lines(&hint.text);
        let size = hint::size(&lines);
        // The placement rule must be in force before the window maps.
        self.actions.place_tip(Some(TipRequest {
            side: hint.side,
            anchor: hint.anchor,
            width: size.0,
            height: size.1,
        }));
        self.hints.shown();
        let serial = self.tip_serial;
        let shared = cx.entity();
        cx.defer(move |cx| {
            let options = tip_options(size, cx);
            let opened = cx.open_window(options, move |_, cx| cx.new(|_| TipWindow::new(lines)));
            shared.update(cx, |s, cx| match opened {
                Ok(handle) if s.tip_serial == serial => s.tip_window = Some(handle),
                // Taken down before its window came up.
                Ok(handle) => {
                    // Not here: the window's closing is reported to this same state,
                    // which is being updated now.
                    cx.defer(move |cx| {
                        handle.update(cx, |_, w, _| w.remove_window()).ok();
                    });
                }
                Err(error) => eprintln!("ofolder: could not open a tooltip: {error}"),
            });
        });
    }

    fn close_tip_window(&mut self, cx: &mut Context<Self>) {
        self.tip_serial += 1;
        if let Some(handle) = self.tip_window.take() {
            cx.defer(move |cx| {
                handle.update(cx, |_, w, _| w.remove_window()).ok();
            });
        }
        self.actions.place_tip(None);
    }

    /// Called when any window closes: a tooltip window the compositor closed
    /// is forgotten.
    pub fn tip_window_closed(&mut self, open: &[WindowId]) {
        if self
            .tip_window
            .is_some_and(|h| !open.contains(&h.window_id()))
        {
            self.tip_window = None;
            self.hints.clear();
        }
    }

    /// A click on a rail icon that has a menu: opens it, or closes it when
    /// it is the one open.
    pub fn click_menu(&mut self, kind: MenuKind, cx: &mut Context<Self>) {
        self.hide_tip(cx);
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
                    // Not here: the window's closing is reported to this same state,
                    // which is being updated now.
                    cx.defer(move |cx| {
                        handle.update(cx, |_, w, _| w.remove_window()).ok();
                    });
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

    // ---- the preview panel --------------------------------------------------

    /// Whether this poll reads the dialog's selection: every poll for Flea,
    /// whose answer costs about 20 ms and comes with the folder's, and every
    /// third for the others, whose answer costs more.
    fn selection_due(&self, tick: u32) -> bool {
        let wanted = self.preview_settings.enabled && !self.preview_hidden;
        wanted && (self.label == "flea" || tick.is_multiple_of(SELECTION_EVERY))
    }

    /// Shows, changes or hides the panel as the selection settles.
    fn follow_selection(&mut self, selection: Option<PathBuf>, cx: &mut Context<Self>) {
        let wanted = selection.filter(|p| classify(p).is_some());
        match self.preview_settle.observe(wanted, Instant::now()) {
            Some(Some(path)) => self.show_preview(path, cx),
            Some(None) => self.close_preview(cx),
            None => {}
        }
    }

    fn show_preview(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(kind) = classify(&path) else {
            return;
        };
        if !self.preview_open {
            let Some(size) = self.actions.preview_size() else {
                // No room under the dialog: try again when the selection changes.
                self.preview_settle.reset();
                return;
            };
            self.open_preview(size, cx);
        }
        let mine = self.preview_generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.preview_content = None;
        let generation = self.preview_generation.clone();
        let settings = self.preview_settings;
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move { panel::load(&path, kind, settings, &generation, mine) })
                .await;
            this.update(cx, |s, cx| {
                if let Some(content) = loaded
                    && s.preview_generation.load(Ordering::Relaxed) == mine
                {
                    s.preview_content = Some(content);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn open_preview(&mut self, size: (i32, i32), cx: &mut Context<Self>) {
        self.preview_open = true;
        self.preview_serial += 1;
        let serial = self.preview_serial;
        // The placement rule must be in force before the window maps.
        self.actions.place_preview(true);
        let shared = cx.entity();
        cx.defer(move |cx| {
            let options = preview_options(size, cx);
            let inner = shared.clone();
            let opened = cx.open_window(options, move |_, cx| {
                cx.new(|cx| PreviewWindow::new(inner, cx))
            });
            shared.update(cx, |s, cx| match opened {
                Ok(handle) if s.preview_serial == serial && s.preview_open => {
                    s.preview_window = Some(handle);
                }
                // Closed before its window came up.
                Ok(handle) => {
                    // Not here: the window's closing is reported to this same state,
                    // which is being updated now.
                    cx.defer(move |cx| {
                        handle.update(cx, |_, w, _| w.remove_window()).ok();
                    });
                }
                Err(error) => {
                    s.preview_open = false;
                    eprintln!("ofolder: could not open the preview: {error}");
                }
            });
        });
    }

    /// Takes the panel down and forgets what it showed.
    fn close_preview(&mut self, cx: &mut Context<Self>) {
        // Any load still running is no longer wanted.
        self.preview_generation.fetch_add(1, Ordering::Relaxed);
        self.preview_content = None;
        if !self.preview_open {
            return;
        }
        self.preview_open = false;
        self.preview_serial += 1;
        if let Some(handle) = self.preview_window.take() {
            cx.defer(move |cx| {
                handle.update(cx, |_, w, _| w.remove_window()).ok();
            });
        }
        self.actions.place_preview(false);
    }

    /// Called when any window closes: a panel the compositor closed is
    /// forgotten until the selection changes.
    pub fn preview_window_closed(&mut self, open: &[WindowId], cx: &mut Context<Self>) {
        if self
            .preview_window
            .is_some_and(|h| !open.contains(&h.window_id()))
        {
            self.preview_window = None;
            self.close_preview(cx);
        }
    }

    /// The `preview` shortcut: hides the panel for this dialog, or lets it
    /// show again.
    pub fn toggle_preview(&mut self, cx: &mut Context<Self>) {
        self.preview_hidden = !self.preview_hidden;
        self.preview_settle.reset();
        if self.preview_hidden {
            self.close_preview(cx);
        }
        cx.notify();
    }

    /// The settings window changed the preview's settings.
    pub fn set_preview_settings(&mut self, settings: PreviewSettings, cx: &mut Context<Self>) {
        self.preview_settings = settings;
        self.preview_settle.reset();
        self.close_preview(cx);
        cx.notify();
    }

    /// A click on the panel's caption copies the file's full path.
    pub fn copy_preview_path(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.preview_content.as_ref().map(|c| c.path.clone()) else {
            return;
        };
        match path.to_str() {
            Some(text) => match self.actions.copy_text(text) {
                Ok(()) => self.vm.show_info("preview", "Path copied"),
                Err(why) => self
                    .vm
                    .show_error("preview", &format!("Could not copy: {why}")),
            },
            None => self
                .vm
                .show_error("preview", "Cannot copy: the path is not valid text"),
        }
        self.changed(cx);
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
        // A tooltip belongs to a rail the user is pointing at.
        if matches!(which, Which::Left | Which::Right) && !active {
            self.hide_tip(cx);
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
                let Ok((paths, tip_side, shown, log, want_selection, preview_open)) =
                    this.update(cx, |t, _| {
                        (
                            t.vm.favorite_paths(),
                            t.hints.wanted().map(|h| h.side),
                            t.vm.take_shown(),
                            t.messages_path.clone(),
                            t.selection_due(tick),
                            t.preview_open,
                        )
                    })
                else {
                    return;
                };
                let actions = actions.clone();
                let sources = sources.clone();
                let read = cx
                    .background_executor()
                    .spawn(async move {
                        if let Some(log) = log {
                            write_log(&log, &shown);
                        }
                        let folder = actions.current_folder();
                        let alive = actions.dialog_alive();
                        let selection = want_selection.then(|| actions.selection());
                        let room = preview_open.then(|| actions.preview_size().is_some());
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
                        // A rail is not told when the pointer leaves it for
                        // good, so a tooltip asks where the pointer is.
                        let off_rail =
                            tip_side.filter(|side| actions.pointer_on_rail(*side) == Some(false));
                        Polled {
                            folder,
                            alive,
                            selection,
                            room,
                            seen,
                            recents,
                            places,
                            off_rail,
                        }
                    })
                    .await;
                let applied = this.update(cx, |t, cx| {
                    if !read.alive {
                        cx.quit();
                        return;
                    }
                    if let Some(side) = read.off_rail
                        && t.hints.wanted().is_some_and(|h| h.side == side)
                    {
                        t.hide_tip(cx);
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
                    if read.room == Some(false) {
                        // The dialog moved where nothing fits below it.
                        t.close_preview(cx);
                        t.preview_settle.reset();
                    }
                    if let Some(selection) = read.selection {
                        t.follow_selection(selection, cx);
                    }
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
                let debug = std::env::var_os("OFOLDER_DEBUG").is_some();
                if debug {
                    eprintln!("ofolder strip: going to {}", path.display());
                }
                let result = cx
                    .background_executor()
                    .spawn(async move { actions.go(&path) })
                    .await;
                if debug {
                    eprintln!("ofolder strip: go finished: {result:?}");
                }
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
    /// The rail whose tooltip is up, when the pointer is no longer on it.
    off_rail: Option<rail::RailSide>,
    /// The dialog's selection, when it was read this time.
    selection: Option<Option<PathBuf>>,
    /// Whether the open preview still fits under the dialog.
    room: Option<bool>,
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
    shared: &Entity<Shared>,
) -> Stateful<gpui::Div> {
    let hint = Hint {
        key: button.id(),
        side: button.side(),
        anchor: TipAnchor::Top((y + rail::BUTTON / 2.).round() as i32),
        text: tooltip,
    };
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
        .on_mouse_down(MouseButton::Left, press_hides_tip(shared))
        .on_hover(hover_shows_tip(shared, hint))
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

/// Shows `hint` while the pointer rests on the element it is attached to.
pub fn hover_shows_tip(
    shared: &Entity<Shared>,
    hint: Hint,
) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
    let shared = shared.clone();
    move |hovered, _, cx| {
        let hint = hint.clone();
        shared.update(cx, |s, cx| s.hover_tip(hint, *hovered, cx));
    }
}

/// A press on a rail icon: it is not a press on the rail's empty space,
/// and it takes the icon's tooltip down.
pub fn press_hides_tip(
    shared: &Entity<Shared>,
) -> impl Fn(&gpui::MouseDownEvent, &mut Window, &mut App) + 'static {
    let shared = shared.clone();
    move |_, _, cx| {
        cx.stop_propagation();
        shared.update(cx, |s, cx| s.hide_tip(cx));
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

/// Appends the messages shown to the log. A log that cannot be written is
/// not worth interrupting the user for; the message itself was on screen.
fn write_log(path: &Path, shown: &[crate::model::Shown]) {
    let entries: Vec<messages::Entry> = shown
        .iter()
        .map(|m| {
            let at =
                m.at.duration_since(std::time::UNIX_EPOCH)
                    .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
                    .unwrap_or(0);
            messages::Entry {
                at,
                offset: local_offset(at),
                kind: match m.kind {
                    crate::model::MessageKind::Info => "info",
                    crate::model::MessageKind::Error => "error",
                },
                context: m.context,
                text: m.text.clone(),
            }
        })
        .collect();
    if let Err(e) = messages::append(path, &entries) {
        eprintln!("ofolder: the message log could not be written: {e}");
    }
}

/// The local time zone's offset from UTC at `at`, in seconds.
pub fn local_offset(at: i64) -> i32 {
    // time_t is 64 bits on every target this builds for.
    let time: libc::time_t = at;
    // SAFETY: `tm` is plain old data (integers and a pointer), for which all
    // zero bits is a valid value.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `localtime_r` reads `time` and writes only into `tm`, both
    // live on this stack frame; a null return means it could not.
    let filled = unsafe { libc::localtime_r(&time, &mut tm) };
    if filled.is_null() {
        return 0;
    }
    i32::try_from(tm.tm_gmtoff).unwrap_or(0)
}
