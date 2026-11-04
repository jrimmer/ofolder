//! The settings window the gear opens: a list of panes on the left with
//! what this is in its corner, and the chosen pane on the right. Every
//! change is saved at once; the rail layout also redraws the open rails.

use gpui::{
    AnyElement, Context, ElementId, Entity, FocusHandle, FontWeight, IntoElement, KeyDownEvent,
    Render, Stateful, Subscription, Window, div, prelude::*, px, rgb, svg,
};

use ofolder_core::messages;
use ofolder_core::preview::{MAX_MB_CHOICES, PreviewSettings};
use ofolder_core::rails::{Item, RailLayout};
use ofolder_core::settings::{self, MAX_RECENTS_LIMIT, Settings};
use ofolder_core::settings_write::{self, Edit};
use ofolder_core::shortcuts::{
    COMMANDS, Combo, ComboError, Group, Held, Shortcuts, from_keystroke,
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

/// Whether a log line records an error: its kind follows the date and time.
pub fn is_error_line(line: &str) -> bool {
    line.split_whitespace().nth(2) == Some("error")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    General,
    Rails,
    Preview,
    Messages,
    Shortcuts,
}

impl Pane {
    const ALL: [Pane; 5] = [
        Pane::General,
        Pane::Rails,
        Pane::Preview,
        Pane::Messages,
        Pane::Shortcuts,
    ];

    fn title(self) -> &'static str {
        match self {
            Pane::General => "General",
            Pane::Rails => "Rails",
            Pane::Preview => "Preview",
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
    _subscriptions: Vec<Subscription>,
}

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
        let root = cx.focus_handle();
        window.focus(&root);
        SettingsWindow {
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
            _subscriptions: subscriptions,
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.shared.update(cx, |s, cx| s.close_settings(cx));
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
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
            Self::heading("Keeping ofolder working"),
            self.guard_toggle(s.guard, cx),
            Self::hint(
                "On: if another program takes ofolder out of the file-chooser chain, it is put back \
                 and you get a notification; if the ofolder program is deleted, file dialogs in every \
                 application keep opening with the next backend instead of failing.",
            ),
            Self::hint(self.guard_note.clone().unwrap_or_default()),
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
                "Saved in {file}. Start folders for each application, folders never \
                 recorded as recent and applications left alone are edited there by hand; \
                 its comments are kept. `ofolder pause` turns ofolder off for every dialog."
            )),
        ]
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
        let mut out = vec![Self::hint(
            "What the rails said, newest first. An error stays on screen while the pointer \
             rests on it; a click dismisses it.",
        )];
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
        let panes = Pane::ALL.into_iter().enumerate().map(|(i, pane)| {
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
        });
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
            // What this is, in the corner.
            .child(
                div()
                    .p(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(div().font_weight(FontWeight::BOLD).child("ofolder"))
                    .child(muted(format!("Version {}", env!("CARGO_PKG_VERSION"))))
                    .when(!label.is_empty(), |d| {
                        d.child(muted(format!("Beside the {label} dialog")))
                    })
                    .child(muted("© 2026 Jason Rimmer".to_owned()))
                    .child(muted("MIT License".to_owned())),
            )
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.pane {
            Pane::General => self.general(cx),
            Pane::Rails => self.rails(cx),
            Pane::Preview => self.preview(cx),
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
    fn every_pane_has_a_title_general_comes_first_and_shortcuts_last() {
        assert_eq!(Pane::ALL.last(), Some(&Pane::Shortcuts));
    }

    #[test]
    fn both_panes_have_titles_and_general_comes_first() {
        assert_eq!(Pane::ALL[0], Pane::General);
        assert!(Pane::ALL.iter().all(|p| !p.title().is_empty()));
    }
}
