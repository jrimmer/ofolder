//! The left panel, "go somewhere": history, favorites, places and recents.

use gpui::{
    AnyElement, Context, Entity, Focusable, IntoElement, Render, Subscription, Window, div,
    prelude::*, px, rgb,
};

use crate::colors::colors;
use crate::expand::Section;
use crate::field::{FieldEvent, TextField};
use crate::model::Scope;
use crate::rows::render_lines;
use crate::view::{Shared, button};

/// Characters of a label that fit this panel at the top level.
const LABEL_CHARS: usize = 17;

pub struct LeftPanel {
    shared: Entity<Shared>,
    field: Entity<TextField>,
    _subscriptions: Vec<Subscription>,
}

impl LeftPanel {
    pub fn new(shared: Entity<Shared>, cx: &mut Context<Self>) -> Self {
        let field = cx.new(TextField::new);
        let observe = cx.observe(&shared, |_, _, cx| cx.notify());
        let handle = cx.subscribe(&field, |this, _, event: &FieldEvent, cx| match event {
            FieldEvent::Submit(text) => {
                let text = text.clone();
                this.shared.update(cx, |s, cx| s.submit(&text, cx));
            }
            FieldEvent::Cancel => this.shared.update(cx, |s, cx| {
                s.vm.cancel();
                cx.notify();
            }),
            FieldEvent::Changed(_) | FieldEvent::Up | FieldEvent::Down => {}
        });
        LeftPanel {
            shared,
            field,
            _subscriptions: vec![observe, handle],
        }
    }

    fn header(title: &str) -> impl IntoElement {
        div()
            .flex_none()
            .px_1()
            .pt_2()
            .text_size(px(11.))
            .text_color(rgb(colors().muted))
            .child(title.to_owned())
    }

    fn rename_prompt(&self, busy: bool, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .px_2()
            .pb_1()
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(colors().muted))
                    .child("Display name (blank restores the folder name)"),
            )
            .child(self.field.clone())
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(button("rename-ok", "Save", !busy).when(!busy, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| {
                            let text = this.field.read(cx).text().to_owned();
                            this.shared.update(cx, |s, cx| s.submit(&text, cx));
                        }))
                    }))
                    .child(
                        button("rename-cancel", "Cancel", true).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.shared.update(cx, |s, cx| {
                                    s.vm.cancel();
                                    cx.notify();
                                });
                            },
                        )),
                    ),
            )
            .into_any_element()
    }
}

impl Render for LeftPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(initial) = self.shared.update(cx, |s, _| s.vm.take_rename_seed()) {
            self.field
                .update(cx, |f, cx| f.reset(&initial, "Display name", cx));
            window.focus(&self.field.focus_handle(cx));
        }
        let (busy, back, forward, up, scope, can_app, recents_on, favs, places, recent, files) = {
            let s = self.shared.read(cx);
            (
                s.vm.busy(),
                s.vm.can_go_back(),
                s.vm.can_go_forward(),
                s.vm.can_go_up(),
                s.vm.scope(),
                s.vm.can_filter_by_app(),
                s.vm.recents_enabled(),
                s.vm.section_lines(Section::Favorites),
                s.vm.section_lines(Section::Places),
                s.vm.section_lines(Section::RecentFolders),
                s.vm.section_lines(Section::RecentFiles),
            )
        };
        let no_favorites = favs.is_empty();
        let prompt = Some(self.rename_prompt(busy, cx));

        let nav = |id: &'static str, label: &str, on: bool| button(id, label, on && !busy);
        let history = div()
            .flex_none()
            .flex()
            .gap_1()
            .child(nav("back", "Back", back).when(back && !busy, |b| {
                b.on_click(cx.listener(|this, _, _, cx| {
                    this.shared.update(cx, |s, cx| {
                        let job = s.vm.go_back();
                        s.start(job, cx);
                    });
                }))
            }))
            .child(
                nav("forward", "Forward", forward).when(forward && !busy, |b| {
                    b.on_click(cx.listener(|this, _, _, cx| {
                        this.shared.update(cx, |s, cx| {
                            let job = s.vm.go_forward();
                            s.start(job, cx);
                        });
                    }))
                }),
            )
            .child(nav("parent", "Parent", up).when(up && !busy, |b| {
                b.on_click(cx.listener(|this, _, _, cx| {
                    this.shared.update(cx, |s, cx| {
                        let job = s.vm.go_up();
                        s.start(job, cx);
                    });
                }))
            }));

        let scope_button = |id: &'static str, label: &str, this: Scope, enabled: bool| {
            let chosen = scope == this;
            let b = button(id, label, enabled);
            let b = if chosen {
                b.bg(rgb(colors().chosen)).text_color(rgb(colors().text))
            } else {
                b
            };
            b.when(enabled && !chosen, |b| {
                b.on_click(cx.listener(move |this_panel, _, _, cx| {
                    this_panel.shared.update(cx, |s, cx| {
                        s.vm.set_scope(this);
                        cx.notify();
                    });
                }))
            })
        };

        let recents_body = if recents_on {
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap_px()
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .pb_1()
                        .child(scope_button("scope-all", "All apps", Scope::AllApps, true))
                        .child(scope_button(
                            "scope-app",
                            "This app",
                            Scope::ThisApp,
                            can_app,
                        )),
                )
                .when(!can_app, |d| {
                    d.child(
                        div()
                            .px_1()
                            .text_size(px(11.))
                            .text_color(rgb(colors().muted))
                            .child("This application cannot be identified."),
                    )
                })
                .children(render_lines(recent, &self.shared, LABEL_CHARS, None))
                .child(Self::header("RECENT FILES"))
                .children(render_lines(files, &self.shared, LABEL_CHARS, None))
        } else {
            div()
                .px_1()
                .text_size(px(11.))
                .text_color(rgb(colors().muted))
                .child("Recent folders are turned off in the settings file.")
        };

        let body = div()
            .id("left-scroll")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_px()
            .child(Self::header("FAVORITES"))
            .children(render_lines(favs, &self.shared, LABEL_CHARS, prompt))
            .when(no_favorites, |d| {
                d.child(
                    div()
                        .px_2()
                        .text_size(px(12.))
                        .text_color(rgb(colors().muted))
                        .child("None yet. Use Favorite to save the current folder."),
                )
            })
            .child(Self::header("PLACES"))
            .children(render_lines(places, &self.shared, LABEL_CHARS, None))
            .child(Self::header("RECENT"))
            .child(recents_body);

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .overflow_hidden()
            .bg(rgb(colors().bg))
            .text_color(rgb(colors().text))
            .text_size(px(13.))
            .child(history)
            .child(div().flex_none().h(px(1.)).bg(rgb(colors().raised)))
            .child(body)
    }
}
