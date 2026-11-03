//! The left rail, "go somewhere": back, forward and parent act at once; the
//! favorites, places and recent lists open in a menu beside the rail.

use gpui::{
    Context, Entity, IntoElement, MouseButton, Render, Subscription, Window, div, prelude::*, px,
    rgb,
};

use crate::colors::colors;
use crate::rail::{self, Button, Entry, RailSide};
use crate::view::{IconLook, Shared, Which, rail_divider, rail_icon};

pub struct LeftRail {
    shared: Entity<Shared>,
    _subscriptions: Vec<Subscription>,
}

impl LeftRail {
    pub fn new(shared: Entity<Shared>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&shared, |_, _, cx| cx.notify());
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            let active = window.is_window_active();
            this.shared
                .update(cx, |s, cx| s.set_active(Which::Left, active, cx));
        });
        LeftRail {
            shared,
            _subscriptions: vec![observe, activation],
        }
    }
}

impl Render for LeftRail {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (entries, busy, back, forward, up, open) = {
            let s = self.shared.read(cx);
            (
                s.layout.entries(RailSide::Left).to_vec(),
                s.vm.busy(),
                s.vm.can_go_back(),
                s.vm.can_go_forward(),
                s.vm.can_go_up(),
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
            let tooltip = button.tooltip(false).to_owned();
            let element = if let Some(kind) = button.menu() {
                let look = if open == Some(kind) {
                    IconLook::Open
                } else {
                    IconLook::Enabled
                };
                rail_icon(button, slot.y, look, tooltip, None, &self.shared).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.shared.update(cx, |s, cx| s.click_menu(kind, cx));
                    },
                ))
            } else {
                let allowed = !busy
                    && match button {
                        Button::Back => back,
                        Button::Forward => forward,
                        _ => up,
                    };
                let look = if allowed {
                    IconLook::Enabled
                } else {
                    IconLook::Disabled
                };
                rail_icon(button, slot.y, look, tooltip, None, &self.shared).when(allowed, |b| {
                    b.on_click(cx.listener(move |this, _, _, cx| {
                        this.shared.update(cx, |s, cx| s.press(button, cx));
                    }))
                })
            };
            rail = rail.child(element);
        }
        rail
    }
}
