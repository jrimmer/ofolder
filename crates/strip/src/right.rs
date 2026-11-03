//! The right rail, "do something here": new folder and find open a menu;
//! the favorite toggle, copy path and the two launchers act at once. At the
//! foot, a mark says which dialog this is, or warns that its folder cannot
//! be read.

use gpui::{
    ClipboardItem, Context, Entity, IntoElement, Render, Subscription, Window, div, prelude::*, px,
    rgb, svg,
};

use crate::colors::colors;
use crate::rail::{self, Button, Entry, RailSide};
use crate::view::{IconLook, Shared, Which, rail_divider, rail_icon, tip};

pub struct RightRail {
    shared: Entity<Shared>,
    label: String,
    _subscriptions: Vec<Subscription>,
}

impl RightRail {
    pub fn new(
        shared: Entity<Shared>,
        label: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe = cx.observe(&shared, |_, _, cx| cx.notify());
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            let active = window.is_window_active();
            this.shared
                .update(cx, |s, cx| s.set_active(Which::Right, active, cx));
        });
        RightRail {
            shared,
            label,
            _subscriptions: vec![observe, activation],
        }
    }
}

impl Render for RightRail {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (known, busy, favorite, full, open) = {
            let s = self.shared.read(cx);
            (
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
            .bg(rgb(colors().bg))
            .text_color(rgb(colors().text))
            .text_size(px(13.));
        for slot in rail::slots(RailSide::Right) {
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
                rail_icon(button, slot.y, look, tooltip, None).when(
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
                rail_icon(button, slot.y, look, tooltip, icon).when(allowed, |b| {
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
        let (mark, mark_color, mark_tip) = if known {
            (
                "icons/info.svg",
                colors().muted,
                format!("ofolder on {}\n{}", self.label, full.unwrap_or_default()),
            )
        } else {
            (
                "icons/warning.svg",
                colors().amber,
                "Folder unknown: the dialog's folder cannot be read".to_owned(),
            )
        };
        rail.child(
            div()
                .id("rail-mark")
                .absolute()
                .bottom(px(8.))
                .left(px((rail::RAIL_WIDTH - rail::BUTTON) / 2.))
                .size(px(rail::BUTTON))
                .flex()
                .items_center()
                .justify_center()
                .tooltip(tip(mark_tip))
                .child(svg().path(mark).size(px(18.)).text_color(rgb(mark_color))),
        )
    }
}
