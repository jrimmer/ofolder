//! The settings window the gear opens: a list of panes on the left with
//! what this is in its corner, and the chosen pane on the right. Every
//! change is saved at once; the rail layout also redraws the open rails.

use gpui::{
    AnyElement, Context, ElementId, Entity, FocusHandle, FontWeight, IntoElement, KeyDownEvent,
    Render, Stateful, Subscription, Window, div, prelude::*, px, rgb, svg,
};

use ofolder_core::rails::{Item, RailLayout};
use ofolder_core::settings::{self, MAX_RECENTS_LIMIT, Settings};
use ofolder_core::settings_write::{self, Edit};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    General,
    Rails,
}

impl Pane {
    const ALL: [Pane; 2] = [Pane::General, Pane::Rails];

    fn title(self) -> &'static str {
        match self {
            Pane::General => "General",
            Pane::Rails => "Rails",
        }
    }
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
        let subscriptions = vec![
            cx.subscribe(&file_manager, command_events("file_manager")),
            cx.subscribe(&terminal, command_events("terminal")),
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
            _subscriptions: subscriptions,
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.shared.update(cx, |s, cx| s.close_settings(cx));
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        let ctrl = ks.modifiers.control || ks.modifiers.platform;
        if ks.key == "escape" || (ctrl && ks.key == "w") {
            cx.stop_propagation();
            self.close(cx);
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
                    .child(last)
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
        let mut out = vec![
            Self::hint(
                "Changes show on the rails at once. An icon stays on its own rail: \
                 the left one goes somewhere, the right one does something here.",
            ),
            Self::heading("Left rail"),
        ];
        out.extend(self.rail_rows(RailSide::Left, cx));
        out.push(Self::heading("Right rail"));
        out.extend(self.rail_rows(RailSide::Right, cx));
        out.push(
            div()
                .pt_3()
                .flex()
                .child(
                    button("rails-reset", "Reset to default layout", !default).when(
                        !default,
                        |b| {
                            b.on_click(cx.listener(|this, _, _, cx| {
                                this.change_rails(
                                    |l| {
                                        *l = RailLayout::default();
                                        true
                                    },
                                    cx,
                                );
                            }))
                        },
                    ),
                )
                .into_any_element(),
        );
        out
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
        };
        let error = self.error.clone();
        div()
            .track_focus(&self.root)
            .on_key_down(cx.listener(Self::on_key))
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
    fn both_panes_have_titles_and_general_comes_first() {
        assert_eq!(Pane::ALL[0], Pane::General);
        assert!(Pane::ALL.iter().all(|p| !p.title().is_empty()));
    }
}
