//! The strip's window contents. It reads the [`ViewModel`] and calls its
//! methods; the model decides what each click means.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AnyView, App, ClipboardItem, Context, ElementId, Entity, Focusable, IntoElement, Render,
    Stateful, Subscription, Window, div, prelude::*, px, rgb,
};

use crate::StripActions;
use crate::field::{FieldEvent, TextField};
use crate::model::{Job, MessageKind, Mode, ViewModel};

const BG: u32 = 0x1b_1d_23;
const PANEL: u32 = 0x24_27_2f;
const RAISED: u32 = 0x2f_33_3d;
const HOVER: u32 = 0x3a_3f_4b;
const TEXT: u32 = 0xe6_e6_e6;
const MUTED: u32 = 0x8a_90_9e;
const ACCENT: u32 = 0x6c_a0_ff;
const RED: u32 = 0xff_7b_72;
const AMBER: u32 = 0xe3_b3_41;

/// How often the dialog is asked for its folder: about five times a second.
const POLL: Duration = Duration::from_millis(200);
/// Favorites are checked for existence every this many polls, because a
/// stat on a slow mount can take a while.
const AVAILABILITY_EVERY: u32 = 10;

struct Tip(String);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(600.))
            .px_2()
            .py_1()
            .rounded(px(4.))
            .bg(rgb(0x0f_10_14))
            .border_1()
            .border_color(rgb(HOVER))
            .text_size(px(12.))
            .text_color(rgb(TEXT))
            .child(self.0.clone())
    }
}

fn tip(text: String) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| cx.new(|_| Tip(text.clone())).into()
}

pub struct StripView {
    vm: ViewModel,
    actions: Arc<dyn StripActions>,
    field: Entity<TextField>,
    _subscription: Subscription,
}

impl StripView {
    pub fn new(vm: ViewModel, actions: Arc<dyn StripActions>, cx: &mut Context<Self>) -> Self {
        let field = cx.new(TextField::new);
        let subscription = cx.subscribe(&field, |this, _, event: &FieldEvent, cx| {
            match event {
                FieldEvent::Submit(text) => this.submit(text, cx),
                FieldEvent::Cancel => this.vm.cancel(),
            }
            cx.notify();
        });
        let view = StripView {
            vm,
            actions,
            field,
            _subscription: subscription,
        };
        view.start_polling(cx);
        view
    }

    /// Asks the dialog for its folder on a background thread and applies the
    /// answer here. Ends the process when the dialog is gone.
    fn start_polling(&self, cx: &mut Context<Self>) {
        let actions = self.actions.clone();
        cx.spawn(async move |this, cx| {
            let mut tick: u32 = 0;
            loop {
                let check = tick.is_multiple_of(AVAILABILITY_EVERY);
                tick = tick.wrapping_add(1);
                let Ok(paths) = this.update(cx, |t, _| t.vm.favorite_paths()) else {
                    return;
                };
                let actions = actions.clone();
                let (folder, alive, seen) = cx
                    .background_executor()
                    .spawn(async move {
                        let folder = actions.current_folder();
                        let alive = actions.dialog_alive();
                        let seen = check.then(|| {
                            paths
                                .into_iter()
                                .map(|p| {
                                    let there = p.is_dir();
                                    (p, there)
                                })
                                .collect::<Vec<_>>()
                        });
                        (folder, alive, seen)
                    })
                    .await;
                let applied = this.update(cx, |t, cx| {
                    if !alive {
                        cx.quit();
                        return;
                    }
                    let mut changed = t.vm.set_folder(folder);
                    if let Some(seen) = seen {
                        changed |= t.vm.set_availability(seen);
                    }
                    changed |= t.vm.expire_message(Instant::now());
                    if changed {
                        cx.notify();
                    }
                });
                if applied.is_err() {
                    return;
                }
                cx.background_executor().timer(POLL).await;
            }
        })
        .detach();
    }

    fn run(&mut self, job: Job, cx: &mut Context<Self>) {
        let actions = self.actions.clone();
        cx.spawn(async move |this, cx| match job {
            Job::Go(path) => {
                let result = cx
                    .background_executor()
                    .spawn(async move { actions.go(&path) })
                    .await;
                this.update(cx, |t, cx| {
                    t.vm.finish_go(result);
                    cx.notify();
                })
                .ok();
            }
            Job::NewFolder(name) => {
                let result = cx
                    .background_executor()
                    .spawn(async move { actions.new_folder(&name) })
                    .await;
                this.update(cx, |t, cx| {
                    t.vm.finish_new_folder(result);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn submit(&mut self, text: &str, cx: &mut Context<Self>) {
        match self.vm.mode() {
            Mode::NewFolder => {
                if let Some(job) = self.vm.submit_new_folder(text) {
                    self.run(job, cx);
                }
            }
            Mode::Rename(_) => self.vm.submit_rename(text),
            _ => {}
        }
    }

    fn open_field(
        &mut self,
        text: &str,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.field
            .update(cx, |f, cx| f.reset(text, placeholder, cx));
        window.focus(&self.field.read(cx).focus_handle(cx));
    }

    fn button(&self, id: impl Into<ElementId>, label: &str, enabled: bool) -> Stateful<gpui::Div> {
        let base = div()
            .id(id)
            .px_2()
            .py(px(3.))
            .rounded(px(4.))
            .text_size(px(12.))
            .whitespace_nowrap()
            .bg(rgb(RAISED))
            .child(label.to_owned());
        if enabled {
            base.cursor_pointer()
                .hover(|s| s.bg(rgb(HOVER)))
                .text_color(rgb(TEXT))
        } else {
            base.text_color(rgb(MUTED)).opacity(0.5)
        }
    }
}

impl Render for StripView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let known = self.vm.folder().is_some();
        let busy = self.vm.busy();
        let mode = self.vm.mode();

        let folder_text = if known {
            div().text_color(rgb(TEXT)).child(self.vm.folder_display())
        } else {
            div().text_color(rgb(MUTED)).italic().child("unknown")
        };
        let folder_box = div()
            .id("folder")
            .flex_none()
            .p_2()
            .rounded(px(6.))
            .bg(rgb(PANEL))
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child("FOLDER"),
            )
            .child(
                folder_text
                    .text_size(px(13.))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis(),
            )
            .when_some(self.vm.folder_full(), |d, full| d.tooltip(tip(full)));

        let favorite_label = if self.vm.current_is_favorite() {
            "Unfavorite"
        } else {
            "Favorite"
        };
        let actions_row = div()
            .flex_none()
            .flex()
            .flex_wrap()
            .gap_1()
            .child(self.button("copy", "Copy path", known).when(known, |b| {
                b.on_click(cx.listener(|this, _, _, cx| {
                    if let Some(text) = this.vm.copy_path() {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                    cx.notify();
                }))
            }))
            .child(
                self.button("favorite", favorite_label, known)
                    .when(known, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| {
                            this.vm.toggle_current_favorite();
                            cx.notify();
                        }))
                    }),
            )
            .child(
                self.button("new-folder", "New folder", known && !busy)
                    .when(known && !busy, |b| {
                        b.on_click(cx.listener(|this, _, window, cx| {
                            if this.vm.start_new_folder() {
                                this.open_field("", "Folder name", window, cx);
                            }
                            cx.notify();
                        }))
                    }),
            );

        let message = self.vm.message().map(|m| {
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
                    this.vm.dismiss_message();
                    cx.notify();
                }))
                .child(m.text.clone())
        });

        let prompt = match mode {
            Mode::NewFolder | Mode::Rename(_) => {
                let (title, ok) = if mode == Mode::NewFolder {
                    ("New folder name (a/b makes nested folders)", "Create")
                } else {
                    ("Display name (blank restores the folder name)", "Save")
                };
                Some(
                    div()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_2()
                        .rounded(px(6.))
                        .bg(rgb(PANEL))
                        .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child(title))
                        .child(self.field.clone())
                        .child(
                            div()
                                .flex()
                                .gap_1()
                                .child(self.button("prompt-ok", ok, !busy).when(!busy, |b| {
                                    b.on_click(cx.listener(|this, _, _, cx| {
                                        let text = this.field.read(cx).text().to_owned();
                                        this.submit(&text, cx);
                                        cx.notify();
                                    }))
                                }))
                                .child(self.button("prompt-cancel", "Cancel", true).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.vm.cancel();
                                        cx.notify();
                                    }),
                                )),
                        ),
                )
            }
            _ => None,
        };

        let rows = self.vm.rows();
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
                .when(!row.missing, |d| {
                    d.cursor_pointer().hover(|s| s.bg(rgb(HOVER)))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(job) = this.vm.click_favorite(index) {
                        this.run(job, cx);
                    }
                    cx.notify();
                }))
                .tooltip(tip(if row.missing {
                    format!("{} (folder is missing)", row.full_path)
                } else {
                    row.full_path.clone()
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .text_size(px(13.))
                        .text_color(rgb(title_color))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_ellipsis()
                                .when(row.missing, |d| d.line_through())
                                .child(row.label.clone()),
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
                    this.vm.toggle_manage(index);
                    cx.notify();
                }))
                .child("Edit");
            let controls =
                managing.then(|| {
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .px_2()
                        .pb_1()
                        .child(
                            self.button(("up", index), "Up", index > 0)
                                .when(index > 0, |b| {
                                    b.on_click(cx.listener(move |this, _, _, cx| {
                                        this.vm.move_up(index);
                                        cx.notify();
                                    }))
                                }),
                        )
                        .child(
                            self.button(("down", index), "Down", index + 1 < count)
                                .when(index + 1 < count, |b| {
                                    b.on_click(cx.listener(move |this, _, _, cx| {
                                        this.vm.move_down(index);
                                        cx.notify();
                                    }))
                                }),
                        )
                        .child(self.button(("rename", index), "Rename", true).on_click(
                            cx.listener(move |this, _, window, cx| {
                                if let Some(initial) = this.vm.start_rename(index) {
                                    this.open_field(&initial, "Display name", window, cx);
                                }
                                cx.notify();
                            }),
                        ))
                        .child(self.button(("remove", index), "Remove", true).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.vm.remove(index);
                                cx.notify();
                            }),
                        ))
                });
            div()
                .flex_none()
                .rounded(px(5.))
                .when(managing, |d| d.bg(rgb(PANEL)))
                .child(div().flex().items_center().child(main).child(edit_toggle))
                .children(controls)
        });

        let favorites = div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex_none()
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
            );

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
            .child(
                div()
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
                            .child(self.vm.label().to_owned()),
                    ),
            )
            .child(folder_box)
            .child(actions_row)
            .children(message)
            .children(prompt)
            .child(favorites)
    }
}
