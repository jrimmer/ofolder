//! The right panel, "do something here": the unknown-folder notice, copy
//! path, favorite, new folder, and the message area.

use gpui::{
    ClipboardItem, Context, Entity, Focusable, IntoElement, Render, Subscription, Window, div,
    prelude::*, px, rgb,
};

use crate::field::{FieldEvent, TextField};
use crate::model::{MessageKind, Mode};
use crate::view::{ACCENT, AMBER, BG, MUTED, PANEL, RED, Shared, TEXT, button, tip};

pub struct RightPanel {
    shared: Entity<Shared>,
    field: Entity<TextField>,
    label: String,
    _subscriptions: Vec<Subscription>,
}

impl RightPanel {
    pub fn new(shared: Entity<Shared>, label: String, cx: &mut Context<Self>) -> Self {
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
        RightPanel {
            shared,
            field,
            label,
            _subscriptions: vec![observe, handle],
        }
    }
}

impl Render for RightPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (known, busy, mode, favorite, full, message) = {
            let s = self.shared.read(cx);
            (
                s.vm.folder().is_some(),
                s.vm.busy(),
                s.vm.mode(),
                s.vm.current_is_favorite(),
                s.vm.folder_full(),
                s.vm.message().cloned(),
            )
        };

        let header = div()
            .flex_none()
            .flex()
            .justify_between()
            .items_center()
            .px_1()
            .child(div().text_size(px(12.)).child("ofolder"))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child(self.label.clone()),
            );

        let notice = (!known).then(|| {
            div()
                .flex_none()
                .px_2()
                .py_1()
                .rounded(px(4.))
                .bg(rgb(PANEL))
                .border_l_2()
                .border_color(rgb(AMBER))
                .text_size(px(12.))
                .text_color(rgb(AMBER))
                .child("Folder unknown: the dialog's folder cannot be read")
        });

        let copy = button("copy", "Copy path", known)
            .when(known, |b| {
                b.on_click(cx.listener(|this, _, _, cx| {
                    let text = this.shared.update(cx, |s, cx| {
                        cx.notify();
                        s.vm.copy_path()
                    });
                    if let Some(text) = text {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }))
            })
            .when_some(full, |b, full| b.tooltip(tip(full)));

        let favorite_button = button(
            "favorite",
            if favorite { "Unfavorite" } else { "Favorite" },
            known,
        )
        .when(known, |b| {
            b.on_click(cx.listener(|this, _, _, cx| {
                this.shared.update(cx, |s, cx| {
                    s.vm.toggle_current_favorite();
                    cx.notify();
                });
            }))
        });

        let new_folder =
            button("new-folder", "New folder", known && !busy).when(known && !busy, |b| {
                b.on_click(cx.listener(|this, _, window, cx| {
                    let started = this.shared.update(cx, |s, cx| {
                        cx.notify();
                        s.vm.start_new_folder()
                    });
                    if started {
                        this.field
                            .update(cx, |f, cx| f.reset("", "Folder name", cx));
                        window.focus(&this.field.focus_handle(cx));
                    }
                }))
            });

        let actions = div()
            .flex_none()
            .flex()
            .flex_wrap()
            .gap_1()
            .child(copy)
            .child(favorite_button)
            .child(new_folder);

        let message = message.map(|m| {
            let color = match m.kind {
                MessageKind::Info => ACCENT,
                MessageKind::Error => RED,
            };
            div()
                .id("message")
                .flex_none()
                .px_2()
                .py_1()
                .rounded(px(4.))
                .bg(rgb(PANEL))
                .border_l_2()
                .border_color(rgb(color))
                .text_size(px(12.))
                .text_color(rgb(color))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.shared.update(cx, |s, cx| {
                        s.vm.dismiss_message();
                        cx.notify();
                    });
                }))
                .child(m.text)
        });

        let prompt = (mode == Mode::NewFolder).then(|| {
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .rounded(px(6.))
                .bg(rgb(PANEL))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child("New folder name (a/b makes nested folders)"),
                )
                .child(self.field.clone())
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .child(button("prompt-ok", "Create", !busy).when(!busy, |b| {
                            b.on_click(cx.listener(|this, _, _, cx| {
                                let text = this.field.read(cx).text().to_owned();
                                this.shared.update(cx, |s, cx| s.submit(&text, cx));
                            }))
                        }))
                        .child(
                            button("prompt-cancel", "Cancel", true).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.shared.update(cx, |s, cx| {
                                        s.vm.cancel();
                                        cx.notify();
                                    });
                                },
                            )),
                        ),
                )
        });

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .overflow_hidden()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .text_size(px(13.))
            .child(header)
            .children(notice)
            .child(actions)
            .children(message)
            .children(prompt)
    }
}
