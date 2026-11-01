//! The left panel, "go somewhere": the favorites list.

use gpui::{
    Context, Entity, Focusable, IntoElement, Render, Subscription, Window, div, prelude::*, px, rgb,
};

use crate::field::{FieldEvent, TextField};
use crate::model::Mode;
use crate::paths::shorten_middle;
use crate::view::{ACCENT, AMBER, BG, HOVER, MUTED, PANEL, Shared, TEXT, button, tip};

pub struct LeftPanel {
    shared: Entity<Shared>,
    field: Entity<TextField>,
    _subscriptions: Vec<Subscription>,
}

/// A small folder drawn from two rectangles, so no icon font is needed.
fn folder_icon(color: u32) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(16.))
        .h(px(14.))
        .flex()
        .flex_col()
        .justify_end()
        .child(
            div()
                .w(px(7.))
                .h(px(2.))
                .rounded_tl(px(2.))
                .rounded_tr(px(2.))
                .bg(rgb(color)),
        )
        .child(div().w(px(14.)).h(px(9.)).rounded(px(2.)).bg(rgb(color)))
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
        });
        LeftPanel {
            shared,
            field,
            _subscriptions: vec![observe, handle],
        }
    }
}

impl Render for LeftPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (rows, mode, busy) = {
            let s = self.shared.read(cx);
            (s.vm.rows(), s.vm.mode(), s.vm.busy())
        };
        let count = rows.len();

        let list = rows.into_iter().map(|row| {
            let index = row.index;
            let managing = mode == Mode::Manage(index) || mode == Mode::Rename(index);
            let title_color = if row.missing { MUTED } else { TEXT };
            let main = div()
                .id(("favorite", index))
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .gap_2()
                .when(!row.missing, |d| {
                    d.cursor_pointer().hover(|s| s.bg(rgb(HOVER)))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.shared.update(cx, |s, cx| {
                        if let Some(job) = s.vm.click_favorite(index) {
                            s.run(job, cx);
                        }
                        cx.notify();
                    });
                }))
                .tooltip(tip(if row.missing {
                    format!("{} (folder is missing)", row.full_path)
                } else {
                    row.full_path.clone()
                }))
                .child(folder_icon(if row.missing { AMBER } else { ACCENT }))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_size(px(13.))
                                .text_color(rgb(title_color))
                                .child(
                                    // Cut by character count: the ellipsis style does
                                    // not apply to a flex item.
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .when(row.missing, |d| d.line_through())
                                        .child(shorten_middle(
                                            &row.label,
                                            if row.missing || row.is_current {
                                                11
                                            } else {
                                                17
                                            },
                                        )),
                                )
                                .when(row.missing, |d| {
                                    d.child(
                                        div()
                                            .flex_none()
                                            .text_size(px(10.))
                                            .text_color(rgb(AMBER))
                                            .child("MISSING"),
                                    )
                                })
                                .when(row.is_current, |d| {
                                    d.child(
                                        div()
                                            .flex_none()
                                            .text_size(px(10.))
                                            .text_color(rgb(ACCENT))
                                            .child("HERE"),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(MUTED))
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(row.subtitle.clone()),
                        ),
                );
            let edit_toggle = div()
                .id(("manage", index))
                .flex_none()
                .px_2()
                .py_1()
                .rounded(px(4.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(rgb(MUTED))
                .hover(|s| s.bg(rgb(HOVER)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.shared.update(cx, |s, cx| {
                        s.vm.toggle_manage(index);
                        cx.notify();
                    });
                }))
                .child("Edit");
            let controls = managing.then(|| {
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .px_2()
                    .pb_1()
                    .child(button(("up", index), "Up", index > 0).when(index > 0, |b| {
                        b.on_click(cx.listener(move |this, _, _, cx| {
                            this.shared.update(cx, |s, cx| {
                                s.vm.move_up(index);
                                cx.notify();
                            });
                        }))
                    }))
                    .child(button(("down", index), "Down", index + 1 < count).when(
                        index + 1 < count,
                        |b| {
                            b.on_click(cx.listener(move |this, _, _, cx| {
                                this.shared.update(cx, |s, cx| {
                                    s.vm.move_down(index);
                                    cx.notify();
                                });
                            }))
                        },
                    ))
                    .child(
                        button(("rename", index), "Rename", true).on_click(cx.listener(
                            move |this, _, window, cx| {
                                let initial = this.shared.update(cx, |s, cx| {
                                    cx.notify();
                                    s.vm.start_rename(index)
                                });
                                if let Some(initial) = initial {
                                    this.field
                                        .update(cx, |f, cx| f.reset(&initial, "Display name", cx));
                                    window.focus(&this.field.focus_handle(cx));
                                }
                            },
                        )),
                    )
                    .child(
                        button(("remove", index), "Remove", true).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.shared.update(cx, |s, cx| {
                                    s.vm.remove(index);
                                    cx.notify();
                                });
                            },
                        )),
                    )
            });
            let rename_prompt =
                matches!(mode, Mode::Rename(i) if i == index).then(|| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .px_2()
                        .pb_1()
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(MUTED))
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
                                .child(button("rename-cancel", "Cancel", true).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.shared.update(cx, |s, cx| {
                                            s.vm.cancel();
                                            cx.notify();
                                        });
                                    }),
                                )),
                        )
                });
            div()
                .flex_none()
                .rounded(px(5.))
                .when(managing, |d| d.bg(rgb(PANEL)))
                .child(div().flex().items_center().child(main).child(edit_toggle))
                .children(controls)
                .children(rename_prompt)
        });

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .overflow_hidden()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .text_size(px(13.))
            .child(
                div()
                    .flex_none()
                    .px_1()
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child("FAVORITES"),
            )
            .child(
                div()
                    .id("favorites-list")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_px()
                    .children(list)
                    .when(count == 0, |d| {
                        d.child(
                            div()
                                .px_2()
                                .text_size(px(12.))
                                .text_color(rgb(MUTED))
                                .child("None yet. Use Favorite to save the current folder."),
                        )
                    }),
            )
    }
}
