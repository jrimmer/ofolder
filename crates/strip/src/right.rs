//! The right panel, "do something here": the unknown-folder notice, copy
//! path, favorite, new folder, launchers, find, and the message area.

use gpui::{
    ClipboardItem, Context, Entity, Focusable, IntoElement, Render, Subscription, Window, div,
    prelude::*, px, rgb,
};

use crate::colors::colors;
use crate::expand::Section;
use crate::field::{FieldEvent, TextField};
use crate::model::{MessageKind, Mode};
use crate::rows::render_lines;
use crate::view::{Shared, button, tip};

/// Characters of a label that fit this panel at the top level.
const LABEL_CHARS: usize = 21;

pub struct RightPanel {
    shared: Entity<Shared>,
    field: Entity<TextField>,
    find: Entity<TextField>,
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
            FieldEvent::Changed(_) | FieldEvent::Up | FieldEvent::Down => {}
        });
        let find = cx.new(TextField::new);
        find.update(cx, |f, cx| f.reset("", "Find a folder", cx));
        let find_handle = cx.subscribe(&find, |this, _, event: &FieldEvent, cx| {
            this.shared.update(cx, |s, cx| {
                match event {
                    FieldEvent::Changed(text) => s.vm.find_set_query(text),
                    FieldEvent::Submit(_) => {
                        let job = s.vm.find_enter();
                        s.start(job, cx);
                    }
                    FieldEvent::Cancel => s.vm.find_escape(),
                    FieldEvent::Up => s.vm.find_move(-1),
                    FieldEvent::Down => s.vm.find_move(1),
                }
                cx.notify();
            });
            // Escape empties the box the user sees as well as the query.
            if matches!(event, FieldEvent::Cancel) {
                let query = this.shared.read(cx).vm.find_query().to_owned();
                this.find
                    .update(cx, |f, cx| f.reset(&query, "Find a folder", cx));
            }
        });
        RightPanel {
            shared,
            field,
            find,
            label,
            _subscriptions: vec![observe, handle, find_handle],
        }
    }
}

impl Render for RightPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (known, busy, mode, favorite, full, message, ranked, found, running, status, has_query) = {
            let s = self.shared.read(cx);
            (
                s.vm.folder().is_some(),
                s.vm.busy(),
                s.vm.mode(),
                s.vm.current_is_favorite(),
                s.vm.folder_full(),
                s.vm.message().cloned(),
                s.vm.section_lines(Section::Ranked),
                s.vm.section_lines(Section::Found),
                s.vm.search_running(),
                s.vm.search_status(),
                !s.vm.find_query().trim().is_empty(),
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
                    .text_color(rgb(colors().muted))
                    .child(self.label.clone()),
            );

        let notice = (!known).then(|| {
            div()
                .flex_none()
                .px_2()
                .py_1()
                .rounded(px(4.))
                .bg(rgb(colors().panel))
                .border_l_2()
                .border_color(rgb(colors().amber))
                .text_size(px(12.))
                .text_color(rgb(colors().amber))
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

        let launcher = |id: &'static str, label: &str, terminal: bool| {
            button(id, label, known).when(known, |b| {
                b.on_click(cx.listener(move |this, _, _, cx| {
                    this.shared.update(cx, |s, cx| {
                        let job = s.vm.open_current(terminal);
                        s.start(job, cx);
                    });
                }))
            })
        };
        let actions = div()
            .flex_none()
            .flex()
            .flex_wrap()
            .gap_1()
            .child(copy)
            .child(favorite_button)
            .child(new_folder)
            .child(launcher("open-fm", "File manager", false))
            .child(launcher("open-term", "Terminal", true));

        let search_button = if running {
            button("search-stop", "Stop", true).on_click(cx.listener(|this, _, _, cx| {
                this.shared.update(cx, |s, cx| s.stop_search(cx));
            }))
        } else {
            button("search-go", "Search below this folder", known && has_query).when(
                known && has_query,
                |b| {
                    b.on_click(cx.listener(|this, _, _, cx| {
                        this.shared.update(cx, |s, cx| s.search(cx));
                    }))
                },
            )
        };
        let find_box = div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .px_1()
                    .pt_1()
                    .text_size(px(11.))
                    .text_color(rgb(colors().muted))
                    .child("FIND"),
            )
            .child(self.find.clone())
            .children(render_lines(ranked, &self.shared, LABEL_CHARS, None))
            .child(div().flex().child(search_button))
            .children(status.map(|text| {
                div()
                    .px_1()
                    .text_size(px(11.))
                    .text_color(rgb(colors().muted))
                    .child(text)
            }))
            .children(render_lines(found, &self.shared, LABEL_CHARS, None));

        let message = message.map(|m| {
            let color = match m.kind {
                MessageKind::Info => colors().accent,
                MessageKind::Error => colors().red,
            };
            div()
                .id("message")
                .flex_none()
                .px_2()
                .py_1()
                .rounded(px(4.))
                .bg(rgb(colors().panel))
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
                .bg(rgb(colors().panel))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(colors().muted))
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
            .bg(rgb(colors().bg))
            .text_color(rgb(colors().text))
            .text_size(px(13.))
            .child(header)
            .child(
                div()
                    .id("right-scroll")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(notice)
                    .child(actions)
                    .children(message)
                    .children(prompt)
                    .child(find_box),
            )
    }
}
