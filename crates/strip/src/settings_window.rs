//! The settings window the gear opens: a list of panes on the left with
//! what this is in its corner, and the chosen pane on the right. Every
//! change is saved at once; the rail layout also redraws the open rails.

use gpui::{
    AnyElement, Context, ElementId, Entity, FocusHandle, FontWeight, IntoElement, KeyDownEvent,
    Render, Stateful, Subscription, Window, div, prelude::*, px, rgb, svg,
};

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ofolder_core::known;
use ofolder_core::messages;
use ofolder_core::preview::{MAX_MB_CHOICES, PreviewSettings};
use ofolder_core::rails::{Item, RailLayout};
use ofolder_core::settings::{
    self, MAX_RECENTS_LIMIT, Settings, check_app_key, check_folder, collapse_home,
    default_recents_excluded,
};
use ofolder_core::settings_write::{self, Edit};
use ofolder_core::shortcuts::{
    COMMANDS, Combo, ComboError, Group, Held, Shortcuts, from_keystroke,
};
use ofolder_core::update::{
    Event, State, Tail, display_version, modify_state, offered_version, pill_text, read_state,
    running_version,
};

use crate::colors::colors;
use crate::field::{FieldEvent, TextField};
use crate::paths::abbreviate;
use crate::rail::{Button, RailSide};
use crate::view::{Shared, button};

/// The window's size in logical pixels.
pub const SIZE: (i32, i32) = (780, 560);
const SIDEBAR: f32 = 180.;
/// The width of a row's label, before its control.
const LABEL: f32 = 160.;
/// Messages listed in the Messages pane.
const RECENT_MESSAGES: usize = 30;
/// The ages offered for "forget recents older than", in days; zero is never.
pub const RECENTS_AGE_CHOICES: [(u32, &str); 4] = [
    (30, "30 days"),
    (90, "90 days"),
    (365, "1 year"),
    (0, "never"),
];

/// How long the copy button says "Copied".
const COPIED_FOR: Duration = Duration::from_millis(2500);

/// What a bug report gets: the versions, then the messages as they were logged, oldest first.
pub fn report_text(ofolder_version: &str, beside: &str, lines: &[String]) -> String {
    let mut text = format!("ofolder {ofolder_version}\n");
    if !beside.is_empty() {
        text.push_str(beside);
        text.push('\n');
    }
    text.push('\n');
    if lines.is_empty() {
        text.push_str("No messages yet.\n");
    }
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    text
}

/// Whether a log line records an error: its kind follows the date and time.
pub fn is_error_line(line: &str) -> bool {
    line.split_whitespace().nth(2) == Some("error")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    General,
    Rails,
    Preview,
    Apps,
    Messages,
    Shortcuts,
}

impl Pane {
    /// Every pane, in the order the sidebar lists them.
    const ALL: [Pane; 6] = [
        Pane::General,
        Pane::Rails,
        Pane::Preview,
        Pane::Apps,
        Pane::Shortcuts,
        Pane::Messages,
    ];
    /// Messages is not a setting: it sits in the sidebar's corner, over the version it goes with.
    fn in_corner(self) -> bool {
        self == Pane::Messages
    }

    fn title(self) -> &'static str {
        match self {
            Pane::General => "General",
            Pane::Rails => "Rails",
            Pane::Preview => "Preview",
            Pane::Apps => "Apps and privacy",
            Pane::Messages => "Messages",
            Pane::Shortcuts => "Shortcuts",
        }
    }
}

/// A combination recorded for one command that another command already has:
/// the user says whether to swap the two or take it from the other.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Conflict {
    id: &'static str,
    combo: Combo,
    other: &'static str,
}

/// Whether a command is listed for the text typed in the filter: its title
/// or its keys contain it, ignoring case.
pub fn matches_filter(title: &str, keys: Option<&Combo>, filter: &str) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty()
        || title.to_lowercase().contains(&filter)
        || keys.is_some_and(|k| k.label().to_lowercase().contains(&filter))
}

/// The number of recent items kept after a press of minus or plus: steps of
/// five, so the usual values are a few presses apart, within the limits the
/// settings file accepts.
pub fn step_recents(value: usize, up: bool) -> usize {
    let next = if up {
        (value / 5 + 1) * 5
    } else if value <= 5 {
        value.saturating_sub(1)
    } else {
        (value - 1) / 5 * 5
    };
    next.clamp(1, MAX_RECENTS_LIMIT)
}

pub struct SettingsWindow {
    shared: Entity<Shared>,
    pane: Pane,
    root: FocusHandle,
    /// The settings as last read from the file.
    settings: Settings,
    file_manager: Entity<TextField>,
    terminal: Entity<TextField>,
    /// Why the last change could not be saved.
    error: Option<String>,
    filter: Entity<TextField>,
    filter_text: String,
    /// The command whose keys are being recorded.
    recording: Option<&'static str>,
    /// Why the last recorded keys were not taken.
    notice: Option<String>,
    conflict: Option<Conflict>,
    /// The "turn the guard off?" question is showing.
    confirm_guard_off: bool,
    /// What the last change of the guard did, to show under its switch.
    guard_note: Option<String>,
    /// The "let omarchy update update ofolder too?" question is showing.
    confirm_update_system: bool,
    /// What the last change of the system registration did.
    update_note: Option<String>,
    /// The What's new list under the update pill is open.
    update_open: bool,
    /// The install that is running, followed through its progress file.
    update_job: Option<UpdateJob>,
    /// Why an install could not be started.
    update_error: Option<String>,
    /// The result of the last copy of the messages, shown on the copy button for a moment.
    copied: Option<Result<(), String>>,
    /// The "forget all recents?" question is showing.
    confirm_forget: bool,
    /// What the last forgetting did, to show under its button.
    forget_note: Option<String>,
    /// The fields of the Apps and privacy pane, each with the reason its last Add was refused.
    app_field: Entity<TextField>,
    app_note: Option<String>,
    recents_field: Entity<TextField>,
    recents_note: Option<String>,
    start_app_field: Entity<TextField>,
    start_folder_field: Entity<TextField>,
    start_note: Option<String>,
    /// The "record this folder in the recents again?" question is showing, for this folder.
    confirm_unexclude: Option<PathBuf>,
    _subscriptions: Vec<Subscription>,
}

/// Which Add of the Apps and privacy pane a field belongs to.
#[derive(Clone, Copy)]
enum ListKind {
    Apps,
    Recents,
    Starts,
}

const APP_PLACEHOLDER: &str = "Application key, such as class:kiosk";
const FOLDER_PLACEHOLDER: &str = "Folder, such as ~/Private or /mnt/secret";
const START_APP_PLACEHOLDER: &str = "Application key";
const START_FOLDER_PLACEHOLDER: &str = "Starts in, such as ~/Downloads";

impl SettingsWindow {
    pub fn new(shared: Entity<Shared>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (path, home) = {
            let s = shared.read(cx);
            (s.settings_path.clone(), s.home.clone())
        };
        let settings = settings::load(&path, &home).settings;
        let file_manager = cx.new(TextField::new);
        let terminal = cx.new(TextField::new);
        file_manager.update(cx, |f, cx| {
            f.reset(
                settings.file_manager.as_deref().unwrap_or(""),
                "Detected from the desktop",
                cx,
            )
        });
        terminal.update(cx, |f, cx| {
            f.reset(
                settings.terminal.as_deref().unwrap_or(""),
                "Detected from $TERMINAL",
                cx,
            )
        });
        let command_events = |key: &'static str| {
            move |this: &mut SettingsWindow,
                  _: Entity<TextField>,
                  event: &FieldEvent,
                  cx: &mut Context<SettingsWindow>| match event {
                FieldEvent::Changed(text) => {
                    let command = Some(text.trim().to_owned()).filter(|t| !t.is_empty());
                    this.save(vec![Edit::Command(key, command)], cx);
                }
                FieldEvent::Cancel => this.close(cx),
                FieldEvent::Submit(_) | FieldEvent::Up | FieldEvent::Down => {}
            }
        };
        let filter = cx.new(TextField::new);
        filter.update(cx, |f, cx| f.reset("", "Filter shortcuts", cx));
        let subscriptions = vec![
            cx.subscribe(&file_manager, command_events("file_manager")),
            cx.subscribe(&terminal, command_events("terminal")),
            cx.subscribe(&filter, |this, _, event: &FieldEvent, cx| match event {
                FieldEvent::Changed(text) => {
                    this.filter_text = text.clone();
                    cx.notify();
                }
                FieldEvent::Cancel => this.close(cx),
                FieldEvent::Submit(_) | FieldEvent::Up | FieldEvent::Down => {}
            }),
            cx.observe(&shared, |_, _, cx| cx.notify()),
        ];
        let list_field = |cx: &mut Context<Self>, placeholder: &'static str, kind: ListKind| {
            let field = cx.new(TextField::new);
            field.update(cx, |f, cx| f.reset("", placeholder, cx));
            let subscription = cx.subscribe(&field, move |this, _, event: &FieldEvent, cx| {
                match event {
                    FieldEvent::Submit(_) => this.add_listed(kind, cx),
                    FieldEvent::Changed(_) => {
                        // The refusal was about the old text.
                        match kind {
                            ListKind::Apps => this.app_note = None,
                            ListKind::Recents => this.recents_note = None,
                            ListKind::Starts => this.start_note = None,
                        }
                        cx.notify();
                    }
                    FieldEvent::Cancel => this.close(cx),
                    FieldEvent::Up | FieldEvent::Down => {}
                }
            });
            (field, subscription)
        };
        let (app_field, s1) = list_field(cx, APP_PLACEHOLDER, ListKind::Apps);
        let (recents_field, s2) = list_field(cx, FOLDER_PLACEHOLDER, ListKind::Recents);
        let (start_app_field, s3) = list_field(cx, START_APP_PLACEHOLDER, ListKind::Starts);
        let (start_folder_field, s4) = list_field(cx, START_FOLDER_PLACEHOLDER, ListKind::Starts);
        let mut subscriptions = subscriptions;
        subscriptions.extend([s1, s2, s3, s4]);
        let root = cx.focus_handle();
        window.focus(&root);
        SettingsWindow {
            app_field,
            app_note: None,
            recents_field,
            recents_note: None,
            start_app_field,
            start_folder_field,
            start_note: None,
            confirm_unexclude: None,
            shared,
            pane: Pane::General,
            root,
            settings,
            file_manager,
            terminal,
            error: None,
            filter,
            filter_text: String::new(),
            recording: None,
            notice: None,
            conflict: None,
            confirm_guard_off: false,
            guard_note: None,
            confirm_forget: false,
            confirm_update_system: false,
            update_note: None,
            update_open: false,
            update_job: None,
            update_error: None,
            copied: None,
            forget_note: None,
            _subscriptions: subscriptions,
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.shared.update(cx, |s, cx| s.close_settings(cx));
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        if self.confirm_update_system {
            // The question has the window: Escape keeps the registration off, nothing else acts.
            cx.stop_propagation();
            if ks.key == "escape" {
                self.confirm_update_system = false;
                cx.notify();
            }
            return;
        }
        if self.confirm_unexclude.is_some() {
            // The question has the window: Escape keeps the folder excluded, nothing else acts.
            cx.stop_propagation();
            if ks.key == "escape" {
                self.confirm_unexclude = None;
                cx.notify();
            }
            return;
        }
        if self.confirm_forget {
            // The question has the window: Escape keeps the recents, nothing else acts.
            cx.stop_propagation();
            if ks.key == "escape" {
                self.confirm_forget = false;
                cx.notify();
            }
            return;
        }
        if self.confirm_guard_off {
            // The question has the window: Escape keeps the guard on, nothing else acts.
            cx.stop_propagation();
            if ks.key == "escape" {
                self.confirm_guard_off = false;
                cx.notify();
            }
            return;
        }
        if let Some(id) = self.recording {
            // While recording, every key belongs to the recording.
            cx.stop_propagation();
            if ks.key == "escape" {
                self.recording = None;
                cx.notify();
                return;
            }
            let held = Held {
                ctrl: ks.modifiers.control,
                alt: ks.modifiers.alt,
                shift: ks.modifiers.shift,
                super_key: ks.modifiers.platform,
            };
            match from_keystroke(held, &ks.key) {
                // Still waiting for the key that goes with the modifiers.
                Err(ComboError::ModifierOnly) => {}
                Err(e) => {
                    self.recording = None;
                    self.notice = Some(capitalised(&e.to_string()));
                    cx.notify();
                }
                Ok(combo) => self.recorded(id, combo, cx),
            }
            return;
        }
        let ctrl = ks.modifiers.control || ks.modifiers.platform;
        if ks.key == "escape" || (ctrl && ks.key == "w") {
            cx.stop_propagation();
            self.close(cx);
        }
    }

    /// Keys were recorded for `id`: refused when the user's own
    /// configuration uses them, asked about when another command has them,
    /// otherwise saved and bound.
    fn recorded(&mut self, id: &'static str, combo: Combo, cx: &mut Context<Self>) {
        self.recording = None;
        self.notice = None;
        self.conflict = None;
        let current = self.settings.shortcuts.clone();
        if current.get(id) == Some(&combo) {
            cx.notify();
            return;
        }
        if let Some(holder) = self.shared.read(cx).taken_by(&combo) {
            self.notice = Some(format!(
                "{} is already {holder}, so ofolder leaves it alone. Choose other keys.",
                combo.label()
            ));
            cx.notify();
            return;
        }
        if let Some(other) = current.holder(&combo) {
            self.conflict = Some(Conflict { id, combo, other });
            cx.notify();
            return;
        }
        let mut next = current;
        next.set(id, Some(combo));
        self.save_shortcuts(next, cx);
    }

    /// Saves `shortcuts`, then binds what the file now says.
    fn save_shortcuts(&mut self, shortcuts: Shortcuts, cx: &mut Context<Self>) {
        self.save(vec![Edit::Shortcuts(shortcuts)], cx);
        if self.error.is_none() {
            let saved = self.settings.shortcuts.clone();
            self.shared.update(cx, |s, cx| s.rebind(&saved, cx));
        }
    }

    /// Writes `edits` to the settings file and reads it back, so the window
    /// shows what the file now says.
    fn save(&mut self, edits: Vec<Edit>, cx: &mut Context<Self>) {
        let (path, home) = {
            let s = self.shared.read(cx);
            (s.settings_path.clone(), s.home.clone())
        };
        match settings_write::save(&path, &edits) {
            Ok(()) => {
                self.settings = settings::load(&path, &home).settings;
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    /// Applies `change` to the rail layout: the open rails redraw at once,
    /// and the layout is saved.
    fn change_rails(
        &mut self,
        change: impl FnOnce(&mut RailLayout) -> bool,
        cx: &mut Context<Self>,
    ) {
        let mut layout = self.shared.read(cx).rails.clone();
        if !change(&mut layout) {
            return;
        }
        self.save(vec![Edit::Rails(layout.clone())], cx);
        self.shared.update(cx, |s, cx| s.set_rails(layout, cx));
    }

    // ---- pieces -----------------------------------------------------------

    fn hint(text: impl Into<String>) -> AnyElement {
        div()
            .text_size(px(12.))
            .text_color(rgb(colors().muted))
            .child(text.into())
            .into_any_element()
    }

    fn heading(text: &str) -> AnyElement {
        div()
            .pt_3()
            .font_weight(FontWeight::BOLD)
            .child(text.to_owned())
            .into_any_element()
    }

    /// A label and its control on one line.
    fn row(label: &str, control: impl IntoElement) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap_3()
            .min_h(px(30.))
            .child(div().w(px(LABEL)).flex_none().child(label.to_owned()))
            .child(control)
            .into_any_element()
    }

    fn switch(on: bool) -> impl IntoElement {
        div()
            .relative()
            .flex_none()
            .w(px(34.))
            .h(px(18.))
            .rounded(px(9.))
            .bg(rgb(if on { colors().accent } else { colors().raised }))
            .child(
                div()
                    .absolute()
                    .top(px(2.))
                    .left(px(if on { 18. } else { 2. }))
                    .size(px(14.))
                    .rounded(px(7.))
                    .bg(rgb(if on { colors().bg } else { colors().muted })),
            )
    }

    /// A switch for one of the preview's settings, saved under `[preview]`
    /// and applied to the open dialog at once.
    fn preview_toggle(
        &self,
        id: &'static str,
        label: &str,
        on: bool,
        flip: fn(&mut PreviewSettings),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap_3()
            .min_h(px(30.))
            .cursor_pointer()
            .child(Self::switch(on))
            .child(label.to_owned())
            .on_click(cx.listener(move |this, _, _, cx| {
                let mut preview = this.settings.preview;
                flip(&mut preview);
                this.save_preview(preview, cx);
            }))
            .into_any_element()
    }

    fn save_preview(&mut self, preview: PreviewSettings, cx: &mut Context<Self>) {
        self.save(vec![Edit::Preview(preview)], cx);
        if self.error.is_none() {
            let saved = self.settings.preview;
            self.shared
                .update(cx, |s, cx| s.set_preview_settings(saved, cx));
        }
    }

    /// The Preview pane: whether the panel under the dialog shows, video
    /// stills, and the largest file it reads.
    fn preview(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = self.settings.preview;
        let sizes = div().flex().gap_1().children(MAX_MB_CHOICES.map(|mb| {
            let chosen = p.max_mb == mb;
            let b = button(("preview-mb", mb as usize), &format!("{mb} MB"), true);
            let b = if chosen {
                b.bg(rgb(colors().chosen)).text_color(rgb(colors().text))
            } else {
                b
            };
            b.when(!chosen, |b| {
                b.on_click(cx.listener(move |this, _, _, cx| {
                    let mut preview = this.settings.preview;
                    preview.max_mb = mb;
                    this.save_preview(preview, cx);
                }))
            })
        }));
        let ffmpeg = std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| dir.join("ffmpeg").is_file())
        });
        vec![
            self.preview_toggle(
                "preview-enabled",
                "Show a preview under the dialog when an image or video is selected",
                p.enabled,
                |p| p.enabled = !p.enabled,
                cx,
            ),
            Self::hint(
                "Images (png, jpg, gif, webp, bmp, tiff, svg) and videos (mp4, mkv, webm, mov, \
                 avi, m4v). Anything else, or no selection, hides it. It needs at least 140 \
                 pixels free below the dialog. A click on its caption copies the file's path.",
            ),
            self.preview_toggle(
                "preview-video",
                "Show a still from a selected video",
                p.video_thumbnails,
                |p| p.video_thumbnails = !p.video_thumbnails,
                cx,
            ),
            Self::hint(if ffmpeg {
                "Stills are made with ffmpeg, which is installed."
            } else {
                "Stills need ffmpeg, which was not found; videos show their details only."
            }),
            Self::row("Largest file", sizes),
            Self::hint("Larger files are described, not opened."),
        ]
    }

    /// Saves the guard setting and puts the guard in place or takes it away.
    fn set_guard(&mut self, on: bool, cx: &mut Context<Self>) {
        self.confirm_guard_off = false;
        self.save(vec![Edit::Bool("guard", on)], cx);
        let actions = self.shared.read(cx).actions();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { actions.set_guard(on) })
                .await;
            this.update(cx, |t, cx| {
                t.guard_note = Some(match result {
                    Ok(_) if on => "The guard is on.".to_owned(),
                    Ok(_) => "The guard is off.".to_owned(),
                    Err(e) => {
                        format!("The setting was saved, but the guard could not be changed: {e}")
                    }
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The guard's switch. Turning it on is at once; turning it off asks first.
    fn guard_toggle(&self, on: bool, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("guard")
            .flex()
            .items_center()
            .gap_3()
            .min_h(px(30.))
            .cursor_pointer()
            .child(Self::switch(on))
            .child("Keep ofolder working, and keep file dialogs working without it")
            .on_click(cx.listener(move |this, _, _, cx| {
                if on {
                    this.confirm_guard_off = true;
                    cx.notify();
                } else {
                    this.set_guard(true, cx);
                }
            }))
            .into_any_element()
    }

    /// Records the user's choice about the update that is offered (`change`) in the update state
    /// file, and redraws the pill from what it now says.
    fn choose_update(&mut self, change: impl FnOnce(&mut State), cx: &mut Context<Self>) {
        let Some(path) = self.shared.read(cx).update_state_path.clone() else {
            return;
        };
        if let Err(e) = modify_state(&path, change) {
            self.update_note = Some(format!("The choice could not be saved: {e}"));
        }
        self.update_open = false;
        cx.notify();
    }

    /// Starts praeco's install of `version` in the background, then follows its progress.
    fn begin_install(&mut self, version: String, terminal: bool, cx: &mut Context<Self>) {
        self.update_job = None;
        self.update_error = None;
        let actions = self.shared.read(cx).actions();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { actions.start_update(&version, terminal) })
                .await;
            this.update(cx, |t, cx| {
                match result {
                    Ok(progress) => {
                        t.update_job = Some(UpdateJob::new(progress));
                        t.watch_update(cx);
                    }
                    Err(e) => t.update_error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Reads the progress file every quarter second until the install ends.
    fn watch_update(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let keep_going = this.update(cx, |t, cx| t.poll_update(cx)).unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    /// Takes what praeco wrote since the last look. Returns whether the install is still running.
    fn poll_update(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(job) = self.update_job.as_mut() else {
            return false;
        };
        if job.finished {
            return false;
        }
        let bytes = read_from(&job.progress, job.tail.offset());
        if bytes.is_empty() && job.tail.offset() == 0 && job.started.elapsed() > UPDATER_START {
            job.failed = Some("The updater did not start.".to_owned());
            job.finished = true;
        }
        for event in job.tail.feed(&bytes) {
            match event {
                Event::Working { text, fraction } => {
                    job.text = text;
                    job.fraction = fraction;
                }
                Event::NeedsTerminal { message } => {
                    job.text = message;
                    job.needs_terminal = true;
                    job.finished = true;
                }
                Event::Done { restarted } => {
                    job.text = if restarted {
                        "Restarting ofolder".to_owned()
                    } else {
                        "Updated. Open ofolder to use the new version.".to_owned()
                    };
                    job.finished = true;
                }
                Event::Failed { message, .. } => {
                    job.failed = Some(message);
                    job.finished = true;
                }
                Event::Ignored => {}
            }
        }
        cx.notify();
        !job.finished
    }

    /// Turns the system registration on or off, after the user chose so, and says what happened.
    fn set_registration(&mut self, on: bool, cx: &mut Context<Self>) {
        self.confirm_update_system = false;
        self.save(vec![Edit::Bool("update_with_system", on)], cx);
        let actions = self.shared.read(cx).actions();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { actions.set_update_registration(on) })
                .await;
            this.update(cx, |t, cx| {
                t.update_note = Some(match result {
                    Ok(message) => message,
                    Err(e) => {
                        format!("The setting was saved, but omarchy update was not changed: {e}")
                    }
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The Updates block of the General pane: the check switch; when a newer release is offered, the
    /// pill, What's new with Install, Later and Skip, and the install's progress; and the switch that
    /// lets omarchy update update ofolder too.
    fn update_section(&self, s: &Settings, cx: &mut Context<Self>) -> AnyElement {
        let mut rows: Vec<AnyElement> = vec![
            Self::heading("Updates"),
            self.toggle(
                "updates",
                "Check for a newer release of ofolder",
                s.updates,
                cx,
            ),
            Self::hint(
                "A check runs a minute after the portal starts, then once a week. Nothing is \
                 installed without your choice.",
            ),
        ];
        let (state_path, available) = {
            let shared = self.shared.read(cx);
            (
                shared.update_state_path.clone(),
                shared.actions().updates_available(),
            )
        };
        if !available {
            rows.push(Self::hint(
                "praeco, which checks for and installs releases, was not found beside ofolder, so \
                 nothing is checked or installed.",
            ));
        } else {
            let state = state_path.as_deref().map(read_state).unwrap_or_default();
            let running = running_version(
                std::env::var("OFOLDER_PRETEND_VERSION").ok().as_deref(),
                env!("CARGO_PKG_VERSION"),
            );
            rows.push(Self::hint(checked_text(&state, &running)));
            if let Some(latest) = offered_version(&state, &running) {
                rows.push(self.update_pill(latest, cx));
                if self.update_open {
                    rows.extend(self.whats_new(&state, latest, cx));
                }
            }
            if let Some(job) = &self.update_job {
                rows.push(self.job_lines(job, cx));
            }
            if let Some(error) = &self.update_error {
                rows.push(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(colors().red))
                        .child(error.clone())
                        .into_any_element(),
                );
            }
            rows.push(self.system_switch(s.update_with_system, cx));
            if let Some(note) = self.update_note.clone() {
                rows.push(Self::hint(note));
            }
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .children(rows)
            .into_any_element()
    }

    /// The chip that says a newer release is out; a click opens or closes What's new.
    fn update_pill(&self, latest: &str, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("update-pill")
            .mt_1()
            .child(
                button(
                    "update-pill-button",
                    &format!("{}  ·  What's new", pill_text(latest)),
                    true,
                )
                .text_color(rgb(colors().accent))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.update_open = !this.update_open;
                    cx.notify();
                })),
            )
            .into_any_element()
    }

    /// The releases newer than the running one, newest first, and the three choices.
    fn whats_new(&self, state: &State, latest: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        for release in &state.notes {
            out.push(
                div()
                    .pt_2()
                    .font_weight(FontWeight::BOLD)
                    .child(format!(
                        "{}  ·  {}",
                        display_version(&release.version),
                        release.published_at.get(..10).unwrap_or("")
                    ))
                    .into_any_element(),
            );
            out.push(Self::hint(short_notes(&release.notes)));
        }
        let version = latest.to_owned();
        let mut choices = div().flex().gap_2().pt_1();
        if state.can_install {
            choices = choices.child(button("update-install", "Install", true).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.begin_install(version.clone(), false, cx);
                }),
            ));
        } else {
            out.push(Self::hint(format!(
                "praeco cannot install this kind of install. The release is at {}.",
                state.notes.first().map_or("", |n| n.url.as_str())
            )));
        }
        choices = choices
            .child(
                button("update-later", "Later", true).on_click(cx.listener(|this, _, _, cx| {
                    this.choose_update(|s| s.later_checked_at = s.checked_at, cx);
                })),
            )
            .child(
                button("update-skip", "Skip this version", true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.choose_update(|s| s.skipped = s.latest.clone(), cx);
                    },
                )),
            );
        out.push(choices.into_any_element());
        out
    }

    /// The install in words, with the one choice praeco can need: a terminal, or a retry.
    fn job_lines(&self, job: &UpdateJob, cx: &mut Context<Self>) -> AnyElement {
        let mut lines = div()
            .flex()
            .flex_col()
            .gap_1()
            .pt_1()
            .child(Self::hint(job.text.clone()));
        if let Some(message) = &job.failed {
            lines = lines.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(colors().red))
                    .child(message.clone()),
            );
        }
        if job.needs_terminal {
            lines = lines.child(
                button("update-terminal", "Install in a terminal window", true).on_click(
                    cx.listener(|this, _, _, cx| {
                        if let Some(version) = this.update_version(cx) {
                            this.begin_install(version, true, cx);
                        }
                    }),
                ),
            );
        }
        if job.failed.is_some() {
            lines = lines.child(
                button("update-retry", "Try again", true).on_click(cx.listener(
                    |this, _, _, cx| {
                        if let Some(version) = this.update_version(cx) {
                            this.begin_install(version, false, cx);
                        }
                    },
                )),
            );
        }
        lines.into_any_element()
    }

    /// The version an install is for: the one the state file offers.
    fn update_version(&self, cx: &mut Context<Self>) -> Option<String> {
        let path = self.shared.read(cx).update_state_path.clone()?;
        let running = running_version(
            std::env::var("OFOLDER_PRETEND_VERSION").ok().as_deref(),
            env!("CARGO_PKG_VERSION"),
        );
        offered_version(&read_state(&path), &running).map(str::to_owned)
    }

    /// The switch that lets omarchy update update ofolder too. Turning it on asks first.
    fn system_switch(&self, on: bool, cx: &mut Context<Self>) -> AnyElement {
        let row = div()
            .id("update-system")
            .flex()
            .items_center()
            .gap_3()
            .min_h(px(30.))
            .cursor_pointer()
            .child(Self::switch(on))
            .child("Let omarchy update also update ofolder")
            .on_click(cx.listener(move |this, _, _, cx| {
                if on {
                    this.set_registration(false, cx);
                } else {
                    this.confirm_update_system = true;
                    cx.notify();
                }
            }));
        div()
            .flex()
            .flex_col()
            .child(row)
            .child(Self::hint(
                "When on, omarchy update updates ofolder with praeco, as it updates the rest of \
                 the system. Turning it off removes that registration again.",
            ))
            .into_any_element()
    }

    /// The question shown over the window before omarchy update is allowed to update ofolder.
    fn update_system_question(&self, cx: &mut Context<Self>) -> AnyElement {
        let line = |text: &'static str| {
            div()
                .flex()
                .gap_2()
                .child(div().flex_none().child("•"))
                .child(div().flex_1().min_w(px(0.)).child(text))
        };
        div()
            .id("update-system-question")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(0x0000_00b3))
            .child(
                div()
                    .w(px(440.))
                    .p(px(18.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .rounded(px(8.))
                    .bg(rgb(colors().panel))
                    .border_1()
                    .border_color(rgb(colors().accent))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::BOLD)
                            .child("Let omarchy update update ofolder?"),
                    )
                    .child(line(
                        "Each time omarchy update runs, it also looks for a newer ofolder and installs it, \
                         with praeco, the way ofolder was installed.",
                    ))
                    .child(line(
                        "Turning this off again removes that registration (praeco init --remove).",
                    ))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .pt_2()
                            .child(button("update-system-keep", "Keep it off", true).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.confirm_update_system = false;
                                    cx.notify();
                                }),
                            ))
                            .child(button("update-system-on", "Turn it on", true).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.set_registration(true, cx);
                                }),
                            )),
                    ),
            )
            .into_any_element()
    }

    /// The question shown over the window before the guard is turned off.
    fn guard_question(&self, cx: &mut Context<Self>) -> AnyElement {
        let line = |text: &'static str| {
            div()
                .flex()
                .gap_2()
                .child(div().flex_none().child("•"))
                .child(div().flex_1().min_w(px(0.)).child(text))
        };
        div()
            .id("guard-question")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(0x0000_00b3))
            .child(
                div()
                    .w(px(440.))
                    .p(px(18.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .rounded(px(8.))
                    .bg(rgb(colors().panel))
                    .border_1()
                    .border_color(rgb(colors().red))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::BOLD)
                            .child("Turn the guard off?"),
                    )
                    .child(div().child("Without the guard:"))
                    .child(line(
                        "If another program takes ofolder out of the file-chooser chain (a Flea \
                         update has done it), ofolder stops working without a word until you run \
                         `ofolder repair`.",
                    ))
                    .child(line(
                        "If the ofolder program is deleted or broken, every Open and Save dialog \
                         in every application stops opening until it is put back.",
                    ))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .pt_2()
                            .child(
                                button("guard-keep", "Keep it on", true).on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.confirm_guard_off = false;
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                button("guard-off", "Turn it off", true)
                                    .text_color(rgb(colors().red))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_guard(false, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// Empties the recents in the background, since it writes a file, and
    /// says what it did under the button.
    fn forget_recents(&mut self, cx: &mut Context<Self>) {
        self.confirm_forget = false;
        let actions = self.shared.read(cx).actions();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { actions.forget_recents() })
                .await;
            this.update(cx, |t, cx| {
                t.forget_note = Some(match result {
                    Ok(text) if text.is_empty() => "Forgotten.".to_owned(),
                    Ok(text) => format!("{}.", capitalised(&text)),
                    Err(e) => format!("The recents could not be forgotten: {e}"),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The question shown over the window before every recent is forgotten.
    fn forget_question(&self, cx: &mut Context<Self>) -> AnyElement {
        let line = |text: &'static str| {
            div()
                .flex()
                .gap_2()
                .child(div().flex_none().child("•"))
                .child(div().flex_1().min_w(px(0.)).child(text))
        };
        div()
            .id("forget-question")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(0x0000_00b3))
            .child(
                div()
                    .w(px(440.))
                    .p(px(18.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .rounded(px(8.))
                    .bg(rgb(colors().panel))
                    .border_1()
                    .border_color(rgb(colors().red))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::BOLD)
                            .child("Forget all recents?"),
                    )
                    .child(line(
                        "Every recent folder and file is removed from ofolder's list, in every \
                         application.",
                    ))
                    .child(line(
                        "Favorites are kept, and no folder or file on disk is touched.",
                    ))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .pt_2()
                            .child(button("recents-forget-keep", "Keep them", true).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.confirm_forget = false;
                                    cx.notify();
                                }),
                            ))
                            .child(
                                button("recents-forget-yes", "Forget them", true)
                                    .text_color(rgb(colors().red))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.forget_recents(cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// The age choices for "forget recents older than", the chosen one marked.
    /// A choice is saved at once and applies to the next read or record.
    fn recents_age_chips(&self, cx: &mut Context<Self>) -> AnyElement {
        let kept = self.settings.recents_days;
        div()
            .flex()
            .gap_1()
            .children(RECENTS_AGE_CHOICES.map(|(days, label)| {
                let chosen = kept == days;
                let b = button(("recents-age", days as usize), label, true);
                let b = if chosen {
                    b.bg(rgb(colors().chosen)).text_color(rgb(colors().text))
                } else {
                    b
                };
                b.when(!chosen, |b| {
                    b.on_click(cx.listener(move |this, _, _, cx| {
                        this.save(vec![Edit::Int("recents_days", i64::from(days))], cx);
                    }))
                })
            }))
            .into_any_element()
    }

    /// A switch with its label; a click anywhere on the line flips it.
    fn toggle(
        &self,
        id: &'static str,
        label: &str,
        on: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap_3()
            .min_h(px(30.))
            .cursor_pointer()
            .child(Self::switch(on))
            .child(label.to_owned())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.save(vec![Edit::Bool(id, !on)], cx);
            }))
            .into_any_element()
    }

    fn icon_button(
        id: impl Into<ElementId>,
        icon: &'static str,
        enabled: bool,
    ) -> Stateful<gpui::Div> {
        let base = div()
            .id(id)
            .flex_none()
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .child(svg().path(icon).size(px(14.)).text_color(rgb(if enabled {
                colors().text
            } else {
                colors().muted
            })));
        if enabled {
            base.cursor_pointer().hover(|s| s.bg(rgb(colors().hover)))
        } else {
            base.opacity(0.4)
        }
    }

    // ---- panes ------------------------------------------------------------

    fn general(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let s = self.settings.clone();
        let file = {
            let shared = self.shared.read(cx);
            abbreviate(&shared.settings_path, &shared.home)
        };
        let recents = s.recents_enabled;
        let max = s.recents_max;
        let stepper = div()
            .flex()
            .items_center()
            .gap_2()
            .when(!recents, |d| d.opacity(0.5))
            .child(
                button("recents-less", "−", recents && max > 1).when(recents && max > 1, |b| {
                    b.on_click(cx.listener(move |this, _, _, cx| {
                        let n = step_recents(max, false) as i64;
                        this.save(vec![Edit::Int("recents_max", n)], cx);
                    }))
                }),
            )
            .child(
                div()
                    .w(px(36.))
                    .flex()
                    .justify_center()
                    .child(max.to_string()),
            )
            .child(
                button("recents-more", "+", recents && max < MAX_RECENTS_LIMIT).when(
                    recents && max < MAX_RECENTS_LIMIT,
                    |b| {
                        b.on_click(cx.listener(move |this, _, _, cx| {
                            let n = step_recents(max, true) as i64;
                            this.save(vec![Edit::Int("recents_max", n)], cx);
                        }))
                    },
                ),
            );
        vec![
            self.toggle(
                "keep_dialog_after_file_manager",
                "Keep the dialog open after opening the file manager",
                s.keep_dialog_after_file_manager,
                cx,
            ),
            self.toggle(
                "recents_enabled",
                "Remember recent folders and files",
                recents,
                cx,
            ),
            Self::row("Recent items kept", stepper),
            Self::hint("Recent folders change with the next dialog."),
            Self::row("Forget recents older than", self.recents_age_chips(cx)),
            Self::hint(
                "Older entries are dropped when the lists are next read or a dialog is recorded. \
                 Folders such as ~/.ssh and the password-manager folders are never recorded \
                 unless the settings file says otherwise.",
            ),
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    button("recents-forget", "Forget all recents…", true)
                        .text_color(rgb(colors().red))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_forget = true;
                            cx.notify();
                        })),
                )
                .into_any_element(),
            Self::hint(self.forget_note.clone().unwrap_or_default()),
            Self::heading("Keeping ofolder working"),
            self.guard_toggle(s.guard, cx),
            Self::hint(
                "On: if another program takes ofolder out of the file-chooser chain, it is put back \
                 and you get a notification; if the ofolder program is deleted, file dialogs in every \
                 application keep opening with the next backend instead of failing.",
            ),
            Self::hint(self.guard_note.clone().unwrap_or_default()),
            self.update_section(&s, cx),
            Self::heading("Programs"),
            Self::row(
                "File manager",
                div().w(px(300.)).child(self.file_manager.clone()),
            ),
            Self::row("Terminal", div().w(px(300.)).child(self.terminal.clone())),
            Self::hint(
                "Leave a command blank to use the desktop's own. The folder is added at the end.",
            ),
            Self::heading("The settings file"),
            Self::hint(format!(
                "Saved in {file}. Start folders, folders never recorded as recent and \
                 applications left alone are set in the Apps and privacy pane; everything else is \
                 edited there by hand. Its comments are kept. `ofolder pause` turns ofolder off \
                 for every dialog."
            )),
        ]
    }

    // ---- apps and privacy -------------------------------------------------

    fn home(&self, cx: &Context<Self>) -> PathBuf {
        self.shared.read(cx).home.clone()
    }

    fn clear_field(field: &Entity<TextField>, placeholder: &'static str, cx: &mut Context<Self>) {
        field.update(cx, |f, cx| f.reset("", placeholder, cx));
    }

    /// Adds what is typed to the list the field belongs to, or says why not.
    fn add_listed(&mut self, kind: ListKind, cx: &mut Context<Self>) {
        let home = self.home(cx);
        match kind {
            ListKind::Apps => {
                let typed = self.app_field.read(cx).text().to_owned();
                match check_app_key(&typed, &self.settings.excluded_apps) {
                    Ok(key) => {
                        let mut apps = self.settings.excluded_apps.clone();
                        apps.push(key);
                        self.save(vec![Edit::ExcludedApps(apps)], cx);
                        if self.error.is_none() {
                            self.app_note = None;
                            Self::clear_field(&self.app_field, APP_PLACEHOLDER, cx);
                        }
                    }
                    Err(why) => self.app_note = Some(why),
                }
            }
            ListKind::Recents => {
                let typed = self.recents_field.read(cx).text().to_owned();
                match check_folder(&typed, &home, &self.settings.recents_excluded) {
                    Ok(folder) => {
                        let mut list = self.settings.recents_excluded.clone();
                        list.push(folder);
                        self.write_recents(&list, cx);
                        if self.error.is_none() {
                            self.recents_note = None;
                            Self::clear_field(&self.recents_field, FOLDER_PLACEHOLDER, cx);
                        }
                    }
                    Err(why) => self.recents_note = Some(why),
                }
            }
            ListKind::Starts => {
                let typed_app = self.start_app_field.read(cx).text().to_owned();
                let typed_folder = self.start_folder_field.read(cx).text().to_owned();
                let known: Vec<String> = self.settings.start_folders.keys().cloned().collect();
                let checked = check_app_key(&typed_app, &known)
                    .map_err(|why| {
                        if why.contains("already") {
                            format!("{why} Remove it first to give it another folder.")
                        } else {
                            why
                        }
                    })
                    .and_then(|app| check_folder(&typed_folder, &home, &[]).map(|f| (app, f)));
                match checked {
                    Ok((app, folder)) => {
                        let written = collapse_home(&folder, &home);
                        self.save(vec![Edit::StartFolder(app, Some(written))], cx);
                        if self.error.is_none() {
                            self.start_note = None;
                            Self::clear_field(&self.start_app_field, START_APP_PLACEHOLDER, cx);
                            Self::clear_field(
                                &self.start_folder_field,
                                START_FOLDER_PLACEHOLDER,
                                cx,
                            );
                        }
                    }
                    Err(why) => self.start_note = Some(why),
                }
            }
        }
        cx.notify();
    }

    /// Saves `list` as the folders never recorded as recent. The default list
    /// removes the key, so a later version's default applies.
    fn write_recents(&mut self, list: &[PathBuf], cx: &mut Context<Self>) {
        let home = self.home(cx);
        let defaults = default_recents_excluded(&home);
        let is_default = list.len() == defaults.len() && defaults.iter().all(|d| list.contains(d));
        let edit = if is_default {
            None
        } else {
            Some(list.iter().map(|p| collapse_home(p, &home)).collect())
        };
        self.save(vec![Edit::RecentsExcluded(edit)], cx);
    }

    /// Takes `folder` off the never-recorded list. A folder of the default list
    /// is a privacy decision, so it asks first.
    fn remove_recents_folder(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        let home = self.home(cx);
        if default_recents_excluded(&home).contains(&folder) {
            self.confirm_unexclude = Some(folder);
            cx.notify();
            return;
        }
        self.unexclude(&folder, cx);
    }

    fn unexclude(&mut self, folder: &Path, cx: &mut Context<Self>) {
        self.confirm_unexclude = None;
        let list: Vec<PathBuf> = self
            .settings
            .recents_excluded
            .iter()
            .filter(|p| p.as_path() != folder)
            .cloned()
            .collect();
        self.write_recents(&list, cx);
    }

    /// One listed entry with a button that removes it.
    fn list_row(
        &self,
        id: (&'static str, usize),
        label: String,
        remove: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap_2()
            .min_h(px(28.))
            .px_2()
            .rounded(px(4.))
            .bg(rgb(colors().panel))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(label),
            )
            .child(
                Self::icon_button(id, "icons/close.svg", true)
                    .on_click(cx.listener(move |this, _, _, cx| remove(this, cx))),
            )
            .into_any_element()
    }

    /// A line of fields with the Add button after them, and under it the reason the last Add was refused.
    fn add_line(
        id: &'static str,
        fields: Vec<AnyElement>,
        note: &Option<String>,
        add: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().flex().items_center().gap_2().children(fields).child(
                button(id, "Add", true).on_click(cx.listener(move |this, _, _, cx| add(this, cx))),
            ))
            .when_some(note.clone(), |d, why| {
                d.child(div().text_color(rgb(colors().amber)).child(why))
            })
            .into_any_element()
    }

    /// The Apps and privacy pane: applications left alone, folders never
    /// recorded as recent, and the folder a dialog starts in for an application.
    fn apps(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let home = self.home(cx);
        let mut out = vec![Self::heading("Applications left alone")];
        out.push(Self::hint(
            "ofolder shows no rails beside the dialogs of these applications. An application \
             is named by its key: its portal id, or class:name for the class of its window \
             (`hyprctl clients` lists them).",
        ));
        let apps = self.settings.excluded_apps.clone();
        if apps.is_empty() {
            out.push(Self::hint("No application is left alone."));
        }
        for (i, app) in apps.iter().enumerate() {
            let key = app.clone();
            out.push(self.list_row(
                ("excluded-app-remove", i),
                app.clone(),
                move |this, cx| {
                    let rest: Vec<String> = this
                        .settings
                        .excluded_apps
                        .iter()
                        .filter(|a| **a != key)
                        .cloned()
                        .collect();
                    this.save(vec![Edit::ExcludedApps(rest)], cx);
                },
                cx,
            ));
        }
        out.push(Self::add_line(
            "excluded-app-add",
            vec![
                div()
                    .w(px(300.))
                    .child(self.app_field.clone())
                    .into_any_element(),
            ],
            &self.app_note,
            |this, cx| this.add_listed(ListKind::Apps, cx),
            cx,
        ));

        out.push(Self::heading("Folders never recorded as recent"));
        out.push(Self::hint(
            "A folder here, and anything inside it, is not added to the recent folders or \
             files, in any application. The list starts with the folders that hold keys, \
             passwords and browser profiles.",
        ));
        let folders = self.settings.recents_excluded.clone();
        if folders.is_empty() {
            out.push(Self::hint("Every folder can be recorded as recent."));
        }
        for (i, folder) in folders.iter().enumerate() {
            let path = folder.clone();
            out.push(self.list_row(
                ("recents-excluded-remove", i),
                abbreviate(folder, &home),
                move |this, cx| this.remove_recents_folder(path.clone(), cx),
                cx,
            ));
        }
        out.push(Self::add_line(
            "recents-excluded-add",
            vec![
                div()
                    .w(px(300.))
                    .child(self.recents_field.clone())
                    .into_any_element(),
            ],
            &self.recents_note,
            |this, cx| this.add_listed(ListKind::Recents, cx),
            cx,
        ));
        let defaults = default_recents_excluded(&home);
        let is_default =
            folders.len() == defaults.len() && defaults.iter().all(|d| folders.contains(d));
        out.push(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    button(
                        "recents-excluded-restore",
                        "Restore the default list",
                        !is_default,
                    )
                    .when(!is_default, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| {
                            this.recents_note = None;
                            this.save(vec![Edit::RecentsExcluded(None)], cx);
                        }))
                    }),
                )
                .into_any_element(),
        );
        out.push(Self::hint(
            "Once the list is changed it is kept as it is, so folders a later version adds to \
             the default are not added to it; restoring the default list follows them again.",
        ));

        out.push(Self::heading("Start folders"));
        out.push(Self::hint(
            "The folder a dialog opens in for an application, when the application names none \
             of its own. It is used only when the folder exists.",
        ));
        let starts: Vec<(String, PathBuf)> = self
            .settings
            .start_folders
            .iter()
            .map(|(a, f)| (a.clone(), f.clone()))
            .collect();
        if starts.is_empty() {
            out.push(Self::hint("No application has a start folder."));
        }
        for (i, (app, folder)) in starts.into_iter().enumerate() {
            let label = format!("{app}  →  {}", abbreviate(&folder, &home));
            out.push(self.list_row(
                ("start-folder-remove", i),
                label,
                move |this, cx| {
                    this.save(vec![Edit::StartFolder(app.clone(), None)], cx);
                },
                cx,
            ));
        }
        out.push(Self::add_line(
            "start-folder-add",
            vec![
                div()
                    .w(px(170.))
                    .child(self.start_app_field.clone())
                    .into_any_element(),
                div()
                    .w(px(230.))
                    .child(self.start_folder_field.clone())
                    .into_any_element(),
            ],
            &self.start_note,
            |this, cx| this.add_listed(ListKind::Starts, cx),
            cx,
        ));
        out
    }

    /// The question shown over the window before a folder of the default list is taken off it.
    fn unexclude_question(&self, folder: &Path, cx: &mut Context<Self>) -> AnyElement {
        let shown = abbreviate(folder, &self.home(cx));
        let line = |text: String| {
            div()
                .flex()
                .gap_2()
                .child(div().flex_none().child("•"))
                .child(div().flex_1().min_w(px(0.)).child(text))
        };
        div()
            .id("recents-excluded-question")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(0x0000_00b3))
            .child(
                div()
                    .w(px(440.))
                    .p(px(18.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .rounded(px(8.))
                    .bg(rgb(colors().panel))
                    .border_1()
                    .border_color(rgb(colors().red))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::BOLD)
                            .child(format!("Record {shown} in the recents?")),
                    )
                    .child(line(format!(
                        "{shown} is on the default list of folders that hold private things. If \
                         you remove it, the folders and files you open or save there will appear \
                         in the recents of every application."
                    )))
                    .child(line(
                        "Anyone who can see your screen can read them there. Nothing on disk is \
                         touched, and Restore the default list puts it back."
                            .to_owned(),
                    ))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .pt_2()
                            .child(
                                button("recents-excluded-keep", "Keep it excluded", true).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.confirm_unexclude = None;
                                        cx.notify();
                                    }),
                                ),
                            )
                            .child({
                                let folder = folder.to_path_buf();
                                button("recents-excluded-remove-yes", "Remove it", true)
                                    .text_color(rgb(colors().red))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.unexclude(&folder, cx);
                                    }))
                            }),
                    ),
            )
            .into_any_element()
    }

    /// The last messages the strips showed, newest first, so one that went
    /// by too fast can be read here.
    fn recent_messages(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let (path, home) = {
            let shared = self.shared.read(cx);
            (shared.messages_path.clone(), shared.home.clone())
        };
        let Some(path) = path else {
            return Vec::new();
        };
        let lines = messages::read_last(&path, RECENT_MESSAGES);
        let copy_label = match &self.copied {
            Some(Ok(())) => "Copied",
            _ => "Copy for a bug report",
        };
        let mut out = vec![
            Self::hint(
                "What the rails said, newest first. An error stays on screen while the pointer \
                 rests on it; a click dismisses it.",
            ),
            div()
                .id("copy-messages")
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .px_2()
                .py(px(3.))
                .rounded(px(4.))
                .text_size(px(12.))
                .whitespace_nowrap()
                .bg(rgb(colors().raised))
                .text_color(rgb(colors().text))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(colors().hover)))
                .child(
                    svg()
                        .path("icons/copy.svg")
                        .size(px(14.))
                        .text_color(rgb(colors().text)),
                )
                .child(copy_label)
                .on_click(cx.listener(|this, _, _, cx| this.copy_messages(cx)))
                .into_any_element(),
        ];
        if let Some(Err(why)) = &self.copied {
            out.push(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(colors().red))
                    .child(format!("Could not copy: {why}"))
                    .into_any_element(),
            );
        }
        if lines.is_empty() {
            out.push(Self::hint("No messages yet."));
        }
        for (i, line) in lines.into_iter().enumerate() {
            let color = if is_error_line(&line) {
                colors().red
            } else {
                colors().muted
            };
            out.push(
                div()
                    .id(("message-line", i))
                    .text_size(px(12.))
                    .text_color(rgb(color))
                    .child(line)
                    .into_any_element(),
            );
        }
        out.push(Self::hint(format!(
            "The last {} are kept in {}.",
            messages::KEEP,
            abbreviate(&path, &home)
        )));
        out
    }

    /// Puts the versions and the kept messages, oldest first, on the clipboard, ready to paste
    /// into a bug report. The button says "Copied" for a moment; a failure is said beside it.
    fn copy_messages(&mut self, cx: &mut Context<Self>) {
        let (path, actions, label, backend_version) = {
            let shared = self.shared.read(cx);
            (
                shared.messages_path.clone(),
                shared.actions(),
                shared.label.clone(),
                shared.backend_version.clone(),
            )
        };
        let mut lines = path
            .map(|p| messages::read_last(&p, messages::KEEP))
            .unwrap_or_default();
        lines.reverse();
        let text = report_text(
            env!("CARGO_PKG_VERSION"),
            &known::describe(&label, backend_version.as_deref()),
            &lines,
        );
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { actions.copy_text(&text) })
                .await;
            this.update(cx, |t, cx| {
                t.copied = Some(result);
                cx.notify();
            })
            .ok();
            cx.background_executor().timer(COPIED_FOR).await;
            this.update(cx, |t, cx| {
                if matches!(t.copied, Some(Ok(()))) {
                    t.copied = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn rail_rows(&mut self, side: RailSide, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let core = side.core();
        let layout = self.shared.read(cx).rails.clone();
        let names = match side {
            RailSide::Left => ["left-up", "left-down", "left-line", "left-hide"],
            RailSide::Right => ["right-up", "right-down", "right-line", "right-hide"],
        };
        let mut out = Vec::new();
        for (i, item) in layout.items(core).iter().enumerate() {
            let (up, down) = (layout.can_move_up(core, i), layout.can_move_down(core, i));
            let what: AnyElement = match item {
                Item::Icon(key) => {
                    let button = Button::from_key(key);
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .flex_1()
                        .when_some(button, |d, b| {
                            d.child(
                                svg()
                                    .path(b.icon())
                                    .size(px(16.))
                                    .text_color(rgb(colors().text)),
                            )
                            .child(b.name())
                        })
                        .into_any_element()
                }
                Item::Divider => div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_1()
                    .child(div().w(px(16.)).h(px(1.)).bg(rgb(colors().muted)))
                    .child(div().text_color(rgb(colors().muted)).child("Divider line"))
                    .into_any_element(),
            };
            let last = match item {
                Item::Icon(_) => {
                    let can = layout.can_hide(core, i);
                    button((names[3], i), "Hide", can).when(can, |b| {
                        b.on_click(cx.listener(move |this, _, _, cx| {
                            this.change_rails(|l| l.hide(core, i), cx);
                        }))
                    })
                }
                Item::Divider => button((names[3], i), "Remove", true).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.change_rails(|l| l.remove_divider(core, i), cx);
                    },
                )),
            };
            out.push(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(32.))
                    .px_2()
                    .rounded(px(6.))
                    .bg(rgb(colors().panel))
                    .child(what)
                    .child(
                        Self::icon_button((names[0], i), "icons/chevron-up.svg", up).when(
                            up,
                            |b| {
                                b.on_click(cx.listener(move |this, _, _, cx| {
                                    this.change_rails(|l| l.move_up(core, i), cx);
                                }))
                            },
                        ),
                    )
                    .child(
                        Self::icon_button((names[1], i), "icons/chevron-down.svg", down).when(
                            down,
                            |b| {
                                b.on_click(cx.listener(move |this, _, _, cx| {
                                    this.change_rails(|l| l.move_down(core, i), cx);
                                }))
                            },
                        ),
                    )
                    .child(
                        button((names[2], i), "Add divider", true).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.change_rails(|l| l.add_divider_after(core, i), cx);
                            },
                        )),
                    )
                    // A fixed width keeps the arrows lined up whether the
                    // last button says Hide or Remove.
                    .child(div().w(px(68.)).flex().justify_end().child(last))
                    .into_any_element(),
            );
        }
        let hidden = layout.hidden(core);
        if !hidden.is_empty() {
            out.push(Self::hint("Hidden"));
            let show_id = match side {
                RailSide::Left => "left-show",
                RailSide::Right => "right-show",
            };
            for (i, key) in hidden.into_iter().enumerate() {
                let Some(b) = Button::from_key(key) else {
                    continue;
                };
                out.push(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .h(px(32.))
                        .px_2()
                        .rounded(px(6.))
                        .child(
                            svg()
                                .path(b.icon())
                                .size(px(16.))
                                .text_color(rgb(colors().muted)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_color(rgb(colors().muted))
                                .child(b.name()),
                        )
                        .child(button((show_id, i), "Show", true).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.change_rails(|l| l.show(core, key), cx);
                            },
                        )))
                        .into_any_element(),
                );
            }
        }
        out
    }

    fn rails(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let default = self.shared.read(cx).rails.is_default();
        // Reset sits at the top, where it is seen without scrolling.
        let reset =
            button("rails-reset", "Reset to default layout", !default).when(!default, |b| {
                b.on_click(cx.listener(|this, _, _, cx| {
                    this.change_rails(
                        |l| {
                            *l = RailLayout::default();
                            true
                        },
                        cx,
                    );
                }))
            });
        let mut out = vec![
            div()
                .flex()
                .items_start()
                .gap_3()
                .child(div().flex_1().min_w(px(0.)).child(Self::hint(
                    "Changes show on the rails at once. An icon stays on its own rail: \
                     the left one goes somewhere, the right one does something here.",
                )))
                .child(div().flex_none().child(reset))
                .into_any_element(),
            Self::heading("Left rail"),
        ];
        out.extend(self.rail_rows(RailSide::Left, cx));
        out.push(Self::heading("Right rail"));
        out.extend(self.rail_rows(RailSide::Right, cx));
        out
    }

    fn shortcuts(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let keys = self.settings.shortcuts.clone();
        let problems = self.shared.read(cx).shortcut_problems.clone();
        let reset = button("shortcuts-reset", "Reset to defaults", !keys.is_empty()).when(
            !keys.is_empty(),
            |b| {
                b.on_click(cx.listener(|this, _, _, cx| {
                    this.conflict = None;
                    this.notice = None;
                    this.save_shortcuts(Shortcuts::default(), cx);
                }))
            },
        );
        let mut out = vec![
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(div().w(px(260.)).child(self.filter.clone()))
                .child(div().flex_1())
                .child(reset)
                .into_any_element(),
            Self::hint(
                "Shortcuts work while a dialog is open: ofolder adds them to Hyprland for that \
                 time only. A combination your own Hyprland configuration already uses is \
                 refused. Click a shortcut, then press the keys; Escape cancels.",
            ),
        ];
        if let Some(notice) = &self.notice {
            out.push(
                div()
                    .text_color(rgb(colors().amber))
                    .child(notice.clone())
                    .into_any_element(),
            );
        }
        if let Some(c) = self.conflict.clone() {
            out.push(self.conflict_prompt(c, cx));
        }
        for group in [Group::GoSomewhere, Group::DoSomethingHere] {
            let defs: Vec<_> = COMMANDS
                .iter()
                .enumerate()
                .filter(|(_, d)| d.group == group)
                .filter(|(_, d)| matches_filter(d.title, keys.get(d.id), &self.filter_text))
                .collect();
            if defs.is_empty() {
                continue;
            }
            out.push(Self::heading(group.title()));
            for (i, def) in defs {
                let id = def.id;
                let combo = keys.get(id).cloned();
                let recording = self.recording == Some(id);
                let (text, color) = match (&combo, recording) {
                    (_, true) => ("Press keys…".to_owned(), colors().accent),
                    (Some(c), false) => (c.label(), colors().text),
                    (None, false) => ("Click to set".to_owned(), colors().muted),
                };
                let key_button = div()
                    .id(("shortcut-key", i))
                    .w(px(170.))
                    .h(px(26.))
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .border_1()
                    .border_color(rgb(if recording {
                        colors().accent
                    } else {
                        colors().field_border
                    }))
                    .bg(rgb(colors().panel))
                    .text_color(rgb(color))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(colors().hover)))
                    // A press here starts a recording; a press anywhere else ends one.
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.notice = None;
                        this.conflict = None;
                        this.recording = if this.recording == Some(id) {
                            None
                        } else {
                            Some(id)
                        };
                        window.focus(&this.root);
                        cx.notify();
                    }))
                    .child(text);
                let clear = combo.is_some().then(|| {
                    Self::icon_button(("shortcut-clear", i), "icons/close.svg", true)
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.recording = None;
                            this.conflict = None;
                            let mut next = this.settings.shortcuts.clone();
                            next.set(id, None);
                            this.save_shortcuts(next, cx);
                        }))
                });
                let problem = problems
                    .iter()
                    .find(|(pid, _)| pid == id)
                    .map(|(_, why)| capitalised(why));
                out.push(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .min_h(px(32.))
                                .child(div().flex_1().child(def.title))
                                .child(key_button)
                                .child(div().w(px(24.)).children(clear)),
                        )
                        .when_some(problem, |d, why| {
                            d.child(
                                div()
                                    .pb_1()
                                    .text_size(px(12.))
                                    .text_color(rgb(colors().amber))
                                    .child(why),
                            )
                        })
                        .into_any_element(),
                );
            }
        }
        out
    }

    fn conflict_prompt(&self, c: Conflict, cx: &mut Context<Self>) -> AnyElement {
        let title = |id: &str| {
            COMMANDS
                .iter()
                .find(|d| d.id == id)
                .map_or(id.to_owned(), |d| d.title.to_owned())
        };
        let (other_title, mine) = (title(c.other), title(c.id));
        let swap = c.clone();
        let take = c.clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .rounded(px(6.))
            .bg(rgb(colors().panel))
            .child(format!(
                "{} is the shortcut for {other_title}. Give it to {mine}?",
                c.combo.label()
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        button("conflict-swap", "Swap the two", true).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.conflict = None;
                                let mut next = this.settings.shortcuts.clone();
                                next.swap(swap.id, swap.combo.clone());
                                this.save_shortcuts(next, cx);
                            },
                        )),
                    )
                    .child(
                        button("conflict-take", &format!("Clear {other_title}"), true).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.conflict = None;
                                let mut next = this.settings.shortcuts.clone();
                                next.take(take.id, take.combo.clone());
                                this.save_shortcuts(next, cx);
                            }),
                        ),
                    )
                    .child(
                        button("conflict-cancel", "Cancel", true).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.conflict = None;
                                cx.notify();
                            },
                        )),
                    ),
            )
            .into_any_element()
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self.shared.read(cx).label.clone();
        let row = |i: usize, pane: Pane, cx: &mut Context<Self>| {
            let chosen = pane == self.pane;
            div()
                .id(("settings-pane", i))
                .h(px(32.))
                .px_3()
                .flex()
                .items_center()
                .rounded(px(6.))
                .cursor_pointer()
                .when(chosen, |d| d.bg(rgb(colors().hover)))
                .when(!chosen, |d| d.hover(|s| s.bg(rgb(colors().panel))))
                .child(pane.title())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.pane = pane;
                    cx.notify();
                }))
        };
        let panes: Vec<_> = Pane::ALL
            .into_iter()
            .enumerate()
            .filter(|(_, pane)| !pane.in_corner())
            .map(|(i, pane)| row(i, pane, cx))
            .collect();
        let corner: Vec<_> = Pane::ALL
            .into_iter()
            .enumerate()
            .filter(|(_, pane)| pane.in_corner())
            .map(|(i, pane)| row(i, pane, cx))
            .collect();
        let (version, backend_version) = {
            let shared = self.shared.read(cx);
            (env!("CARGO_PKG_VERSION"), shared.backend_version.clone())
        };
        let beside = known::describe(&label, backend_version.as_deref());
        let muted = |text: String| {
            div()
                .text_size(px(11.))
                .text_color(rgb(colors().muted))
                .child(text)
        };
        div()
            .w(px(SIDEBAR))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .justify_between()
            .bg(rgb(colors().sidebar))
            .child(
                div()
                    .pt(px(14.))
                    .px_2()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .children(panes),
            )
            // What this is, in the corner, with the messages that go with a bug report above it.
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(div().px_2().flex().flex_col().gap(px(2.)).children(corner))
                    .child(
                        div()
                            .p(px(14.))
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(div().font_weight(FontWeight::BOLD).child("ofolder"))
                            .child(muted(format!("Version {version}")))
                            .when(!beside.is_empty(), |d| d.child(muted(beside.clone())))
                            .child(muted("© 2026 Jason Rimmer".to_owned()))
                            .child(muted("MIT License".to_owned())),
                    ),
            )
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.pane {
            Pane::General => self.general(cx),
            Pane::Rails => self.rails(cx),
            Pane::Preview => self.preview(cx),
            Pane::Apps => self.apps(cx),
            Pane::Messages => self.recent_messages(cx),
            Pane::Shortcuts => self.shortcuts(cx),
        };
        let error = self.error.clone();
        div()
            .track_focus(&self.root)
            .on_key_down(cx.listener(Self::on_key))
            // A press anywhere else ends a recording.
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.recording.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .size_full()
            .relative()
            .flex()
            .bg(rgb(colors().bg))
            .text_color(rgb(colors().text))
            .text_size(px(13.))
            .child(self.sidebar(cx))
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .px(px(24.))
                            .py(px(20.))
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .pb_2()
                                    .text_size(px(19.))
                                    .font_weight(FontWeight::BOLD)
                                    .child(self.pane.title()),
                            )
                            .when_some(error, |d, e| {
                                d.child(div().text_color(rgb(colors().red)).child(e))
                            })
                            .children(body),
                    ),
            )
            .when(self.confirm_guard_off, |d| d.child(self.guard_question(cx)))
            .when(self.confirm_forget, |d| d.child(self.forget_question(cx)))
            .when_some(self.confirm_unexclude.clone(), |d, folder| {
                d.child(self.unexclude_question(&folder, cx))
            })
            .when(self.confirm_update_system, |d| {
                d.child(self.update_system_question(cx))
            })
            // Omarchy windows have no title bar, so the window closes itself.
            .child(
                div()
                    .id("settings-close")
                    .absolute()
                    .top(px(10.))
                    .right(px(10.))
                    .size(px(26.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(colors().hover)))
                    .child(
                        svg()
                            .path("icons/close.svg")
                            .size(px(14.))
                            .text_color(rgb(colors().muted)),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
            )
    }
}

/// How long praeco may take to create its progress file before the install is called failed.
const UPDATER_START: Duration = Duration::from_secs(20);
/// The most of a release's notes the window shows; the rest is on its page.
const NOTES_CHARS: usize = 1200;

/// An install praeco is doing, followed through the progress file it writes.
struct UpdateJob {
    progress: PathBuf,
    tail: Tail,
    started: Instant,
    text: String,
    fraction: Option<f64>,
    /// praeco needs a terminal window to ask for the password.
    needs_terminal: bool,
    failed: Option<String>,
    finished: bool,
}

impl UpdateJob {
    fn new(progress: PathBuf) -> Self {
        UpdateJob {
            progress,
            tail: Tail::default(),
            started: Instant::now(),
            text: "Getting ready".to_owned(),
            fraction: None,
            needs_terminal: false,
            failed: None,
            finished: false,
        }
    }
}

/// The bytes of `path` from `offset` on; empty while the file does not exist yet.
fn read_from(path: &Path, offset: u64) -> Vec<u8> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(offset)).is_ok() {
        file.read_to_end(&mut bytes).ok();
    }
    bytes
}

/// The line under the Updates heading that says when the last check was and what it found.
fn checked_text(state: &State, running: &str) -> String {
    let Some(_) = state.checked_at else {
        return "No check has been made yet.".to_owned();
    };
    let latest = state.latest.as_deref().unwrap_or(running);
    if !state.update_available {
        return format!("ofolder {running} is the latest release.");
    }
    if state.skipped.as_deref() == Some(latest) {
        return format!("You skipped {latest}; the release after it is offered.");
    }
    format!("ofolder {running}; {latest} is the latest release.")
}

/// The start of a release's notes, cut at a character boundary.
fn short_notes(notes: &str) -> String {
    let mut text: String = notes.chars().take(NOTES_CHARS).collect();
    if notes.chars().count() > NOTES_CHARS {
        text.push_str("...");
    }
    text
}

/// The text with its first letter capitalised, for a message line.
fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_items_step_by_fives_within_the_limits() {
        assert_eq!(step_recents(20, true), 25);
        assert_eq!(step_recents(20, false), 15);
        assert_eq!(step_recents(22, true), 25);
        assert_eq!(step_recents(22, false), 20);
        assert_eq!(step_recents(5, false), 4);
        assert_eq!(step_recents(1, false), 1);
        assert_eq!(step_recents(MAX_RECENTS_LIMIT, true), MAX_RECENTS_LIMIT);
    }

    #[test]
    fn the_age_choices_are_in_range_and_include_the_default() {
        assert!(
            RECENTS_AGE_CHOICES
                .iter()
                .all(|(days, _)| *days <= settings::MAX_RECENTS_DAYS)
        );
        assert!(
            RECENTS_AGE_CHOICES
                .iter()
                .any(|(days, _)| *days == settings::DEFAULT_RECENTS_DAYS)
        );
        assert_eq!(RECENTS_AGE_CHOICES.last(), Some(&(0, "never")));
    }

    #[test]
    fn the_filter_matches_a_title_or_the_keys_ignoring_case() {
        let keys = Combo::parse("SUPER+ALT+F9").expect("valid");
        assert!(matches_filter("Places and drives", None, ""));
        assert!(matches_filter("Places and drives", None, "  DRIVES "));
        assert!(matches_filter("Places and drives", Some(&keys), "alt+f9"));
        assert!(!matches_filter(
            "Places and drives",
            Some(&keys),
            "terminal"
        ));
        assert!(!matches_filter("Places and drives", None, "f9"));
    }

    #[test]
    fn an_error_line_in_the_log_is_told_from_a_note() {
        assert!(is_error_line(
            "2026-11-03 01:22:46 error go: did not arrive"
        ));
        assert!(!is_error_line(
            "2026-11-03 01:22:46 info copy-path: Path copied"
        ));
        assert!(!is_error_line("garbage"));
    }

    #[test]
    fn a_message_starts_with_a_capital() {
        assert_eq!(capitalised("add Super or Alt"), "Add Super or Alt");
        assert_eq!(capitalised(""), "");
    }

    #[test]
    fn messages_sit_in_the_corner_over_the_version_and_shortcuts_close_the_list() {
        assert_eq!(Pane::ALL.last(), Some(&Pane::Messages));
        let listed: Vec<_> = Pane::ALL.iter().filter(|p| !p.in_corner()).collect();
        assert_eq!(listed.last(), Some(&&Pane::Shortcuts));
        assert_eq!(Pane::ALL.iter().filter(|p| p.in_corner()).count(), 1);
    }

    #[test]
    fn a_bug_report_has_the_versions_then_the_messages_oldest_first() {
        let lines = vec![
            "2026-11-07 10:00:00 error go: refused".to_owned(),
            "2026-11-07 10:00:05 error open: no terminal".to_owned(),
        ];
        assert_eq!(
            report_text("1.0.1", "With Flea v0.4.0", &lines),
            "ofolder 1.0.1\nWith Flea v0.4.0\n\n\
             2026-11-07 10:00:00 error go: refused\n\
             2026-11-07 10:00:05 error open: no terminal\n"
        );
        assert_eq!(
            report_text("1.0.1", "", &[]),
            "ofolder 1.0.1\n\nNo messages yet.\n"
        );
    }

    #[test]
    fn both_panes_have_titles_and_general_comes_first() {
        assert_eq!(Pane::ALL[0], Pane::General);
        assert!(Pane::ALL.iter().all(|p| !p.title().is_empty()));
    }

    #[test]
    fn the_check_line_says_what_was_found_or_that_nothing_was_checked() {
        let fresh = State::default();
        assert_eq!(checked_text(&fresh, "0.1.0"), "No check has been made yet.");
        let found = State {
            checked_at: Some(5),
            latest: Some("0.2.0".to_owned()),
            update_available: true,
            ..State::default()
        };
        assert_eq!(
            checked_text(&found, "0.1.0"),
            "ofolder 0.1.0; 0.2.0 is the latest release."
        );
        let skipped = State {
            skipped: Some("0.2.0".to_owned()),
            ..found.clone()
        };
        assert_eq!(
            checked_text(&skipped, "0.1.0"),
            "You skipped 0.2.0; the release after it is offered."
        );
        let current = State {
            update_available: false,
            ..found
        };
        assert_eq!(
            checked_text(&current, "0.2.0"),
            "ofolder 0.2.0 is the latest release."
        );
    }

    #[test]
    fn long_release_notes_are_cut_at_a_character_not_a_byte() {
        let notes = "é".repeat(NOTES_CHARS + 5);
        let shown = short_notes(&notes);
        assert_eq!(shown.chars().count(), NOTES_CHARS + 3);
        assert!(shown.ends_with("..."));
        assert_eq!(short_notes("Fixed"), "Fixed");
    }
}
