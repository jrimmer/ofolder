//! The right rail, "do something here": new folder and find open a menu;
//! the favorite toggle, copy path and the two launchers act at once. At the
//! foot, the gear opens the settings window; an amber dot on it says the
//! dialog's folder cannot be read.

use gpui::{
    ClipboardItem, Context, Entity, IntoElement, MouseButton, Render, Subscription, Window, div,
    prelude::*, px, rgb, svg,
};

use crate::colors::colors;
use crate::hint::{Hint, TipAnchor};
use crate::rail::{self, Button, Entry, RailSide};
use crate::view::{
    IconLook, Shared, Which, hover_shows_tip, press_hides_tip, rail_divider, rail_icon,
};

/// The gear's distance from the rail's foot.
const GEAR_BOTTOM: f32 = 8.0;

pub struct RightRail {
    shared: Entity<Shared>,
    _subscriptions: Vec<Subscription>,
}

impl RightRail {
    pub fn new(shared: Entity<Shared>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&shared, |_, _, cx| cx.notify());
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            let active = window.is_window_active();
            this.shared
                .update(cx, |s, cx| s.set_active(Which::Right, active, cx));
        });
        RightRail {
            shared,
            _subscriptions: vec![observe, activation],
        }
    }
}

impl Render for RightRail {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (entries, known, busy, favorite, full, open) = {
            let s = self.shared.read(cx);
            (
                s.layout.entries(RailSide::Right).to_vec(),
                s.vm.folder().is_some(),
                s.vm.busy(),
                s.vm.current_is_favorite(),
                s.vm.folder_full(),
                s.open_menu(),
            )
        };
        let mut rail = div()
            .size_full()
            .relative()
            .overflow_hidden()
            // A press on the rail's empty space takes down an open menu.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.shared.update(cx, |s, cx| s.close_menu(cx));
                }),
            )
            .bg(rgb(colors().bg))
            .text_color(rgb(colors().text))
            .text_size(px(13.));
        for slot in rail::slots(&entries) {
            let button = match slot.entry {
                Entry::Divider => {
                    rail = rail.child(rail_divider(slot.y));
                    continue;
                }
                Entry::Button(b) => b,
            };
            let allowed = known && !(busy && button == Button::NewFolder);
            let mut tooltip = button.tooltip(favorite).to_owned();
            if button == Button::CopyPath
                && let Some(full) = &full
            {
                tooltip = format!("{tooltip}\n{full}");
            }
            if !known {
                tooltip = format!("{tooltip} (the folder is unknown)");
            }
            let element = if let Some(kind) = button.menu() {
                let look = match (open == Some(kind), allowed) {
                    (true, _) => IconLook::Open,
                    (false, true) => IconLook::Enabled,
                    (false, false) => IconLook::Disabled,
                };
                rail_icon(button, slot.y, look, tooltip, None, &self.shared).when(
                    allowed || open == Some(kind),
                    |b| {
                        b.on_click(cx.listener(move |this, _, _, cx| {
                            this.shared.update(cx, |s, cx| s.click_menu(kind, cx));
                        }))
                    },
                )
            } else {
                let on = button == Button::Favorite && favorite;
                let look = match (allowed, on) {
                    (false, _) => IconLook::Disabled,
                    (true, true) => IconLook::On,
                    (true, false) => IconLook::Enabled,
                };
                let icon = on.then_some("icons/star-filled.svg");
                rail_icon(button, slot.y, look, tooltip, icon, &self.shared).when(allowed, |b| {
                    b.on_click(cx.listener(move |this, _, _, cx| {
                        let copied = this.shared.update(cx, |s, cx| {
                            s.close_menu(cx);
                            let copied = match button {
                                Button::Favorite => {
                                    s.vm.toggle_current_favorite();
                                    None
                                }
                                Button::CopyPath => s.vm.copy_path(),
                                Button::FileManager => {
                                    let job = s.vm.open_current(false);
                                    s.start(job, cx);
                                    None
                                }
                                _ => {
                                    let job = s.vm.open_current(true);
                                    s.start(job, cx);
                                    None
                                }
                            };
                            s.changed(cx);
                            copied
                        });
                        if let Some(text) = copied {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }
                    }))
                })
            };
            rail = rail.child(element);
        }
        let settings_open = self.shared.read(cx).settings_open();
        let gear_tip = if known {
            "Settings".to_owned()
        } else {
            "Settings\nFolder unknown: the dialog's folder cannot be read".to_owned()
        };
        rail.child(
            div()
                .id("rail-gear")
                .absolute()
                .bottom(px(GEAR_BOTTOM))
                .left(px((rail::RAIL_WIDTH - rail::BUTTON) / 2.))
                .size(px(rail::BUTTON))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(7.))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, press_hides_tip(&self.shared))
                .when(settings_open, |d| d.bg(rgb(colors().chosen)))
                .hover(|s| s.bg(rgb(colors().hover)))
                .on_hover(hover_shows_tip(
                    &self.shared,
                    Hint {
                        key: "rail-gear",
                        side: RailSide::Right,
                        anchor: TipAnchor::Bottom((GEAR_BOTTOM + rail::BUTTON / 2.).round() as i32),
                        text: gear_tip,
                    },
                ))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.shared.update(cx, |s, cx| s.toggle_settings(cx));
                }))
                .child(svg().path("icons/gear.svg").size(px(20.)).text_color(rgb(
                    if settings_open {
                        colors().accent
                    } else {
                        colors().muted
                    },
                )))
                // The folder cannot be read: a small amber dot on the gear,
                // explained by its tooltip.
                .when(!known, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top(px(5.))
                            .right(px(5.))
                            .size(px(7.))
                            .rounded(px(4.))
                            .bg(rgb(colors().amber)),
                    )
                }),
        )
    }
}
