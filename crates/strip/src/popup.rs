//! A rail's menu: the favorites, places and recent lists, the new-folder
//! prompt and find, each in its own small window beside the rail. The same
//! window, with no icon behind it, shows a message while no menu is open.

use gpui::{
    AnyElement, Context, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render,
    Subscription, Window, div, prelude::*, px, rgb,
};

use crate::colors::colors;
use crate::expand::Section;
use crate::field::{FieldEvent, TextField};
use crate::model::{MessageKind, Mode, Scope};
use crate::rail::MenuKind;
use crate::rows::render_lines;
use crate::view::{Shared, Which, button};

/// Characters of a label that fit a menu at the top level.
const LABEL_CHARS: usize = 26;

pub struct MenuWindow {
    shared: Entity<Shared>,
    kind: MenuKind,
    /// A new folder's name, or a favorite's new display name.
    field: Entity<TextField>,
    find: Entity<TextField>,
    /// Holds the keyboard when no field does, for Escape.
    root: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl MenuWindow {
    pub fn new(
        shared: Entity<Shared>,
        kind: MenuKind,
        serial: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe = cx.observe(&shared, |_, _, cx| cx.notify());
        let activation = cx.observe_window_activation(window, move |this, window, cx| {
            let active = window.is_window_active();
            this.shared
                .update(cx, |s, cx| s.set_active(Which::Menu(serial), active, cx));
        });
        let field = cx.new(TextField::new);
        let field_events = cx.subscribe(&field, |this, _, event: &FieldEvent, cx| match event {
            FieldEvent::Submit(text) => {
                let text = text.clone();
                this.shared.update(cx, |s, cx| {
                    s.submit(&text, cx);
                    // A name that was refused keeps the prompt open to fix it.
                    if s.vm.mode() == Mode::Browse && this.kind == MenuKind::NewFolder {
                        s.after_choice(cx);
                    }
                });
            }
            FieldEvent::Cancel => this.shared.update(cx, |s, cx| {
                s.vm.cancel();
                if this.kind == MenuKind::NewFolder {
                    s.close_menu(cx);
                }
                cx.notify();
            }),
            FieldEvent::Changed(_) | FieldEvent::Up | FieldEvent::Down => {}
        });
        let find = cx.new(TextField::new);
        let query = shared.read(cx).vm.find_query().to_owned();
        find.update(cx, |f, cx| f.reset(&query, "Find a folder", cx));
        let find_events = cx.subscribe(&find, |this, _, event: &FieldEvent, cx| {
            let was_empty = this.shared.read(cx).vm.find_query().trim().is_empty();
            this.shared.update(cx, |s, cx| {
                match event {
                    FieldEvent::Changed(text) => s.vm.find_set_query(text),
                    FieldEvent::Submit(_) => {
                        let job = s.vm.find_enter();
                        let going = job.is_some();
                        s.start(job, cx);
                        if going {
                            s.after_choice(cx);
                        }
                    }
                    // Escape empties the query first, then closes the menu.
                    FieldEvent::Cancel if was_empty => s.close_menu(cx),
                    FieldEvent::Cancel => s.vm.find_escape(),
                    FieldEvent::Up => s.vm.find_move(-1),
                    FieldEvent::Down => s.vm.find_move(1),
                }
                cx.notify();
            });
            if matches!(event, FieldEvent::Cancel) {
                let query = this.shared.read(cx).vm.find_query().to_owned();
                this.find
                    .update(cx, |f, cx| f.reset(&query, "Find a folder", cx));
            }
        });
        let root = cx.focus_handle();
        match kind {
            MenuKind::NewFolder => {
                field.update(cx, |f, cx| f.reset("", "Folder name", cx));
                window.focus(&field.focus_handle(cx));
            }
            MenuKind::Find => window.focus(&find.focus_handle(cx)),
            _ => window.focus(&root),
        }
        MenuWindow {
            shared,
            kind,
            field,
            find,
            root,
            _subscriptions: vec![observe, activation, field_events, find_events],
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            self.shared.update(cx, |s, cx| s.close_menu(cx));
        }
    }

    fn title(text: &str) -> impl IntoElement {
        div()
            .flex_none()
            .px_2()
            .pt_1()
            .text_size(px(11.))
            .text_color(rgb(colors().muted))
            .child(text.to_owned())
    }

    fn quiet(text: &str) -> AnyElement {
        div()
            .px_2()
            .py_1()
            .text_size(px(12.))
            .text_color(rgb(colors().muted))
            .child(text.to_owned())
            .into_any_element()
    }

    fn prompt(
        &self,
        intro: &str,
        ok: &'static str,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                    .child(intro.to_owned()),
            )
            .child(self.field.clone())
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(button("prompt-ok", ok, !busy).when(!busy, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| {
                            let text = this.field.read(cx).text().to_owned();
                            let kind = this.kind;
                            this.shared.update(cx, |s, cx| {
                                s.submit(&text, cx);
                                if s.vm.mode() == Mode::Browse && kind == MenuKind::NewFolder {
                                    s.after_choice(cx);
                                }
                            });
                        }))
                    }))
                    .child(
                        button("prompt-cancel", "Cancel", true).on_click(cx.listener(
                            |this, _, _, cx| {
                                let kind = this.kind;
                                this.shared.update(cx, |s, cx| {
                                    s.vm.cancel();
                                    if kind == MenuKind::NewFolder {
                                        s.close_menu(cx);
                                    }
                                    cx.notify();
                                });
                            },
                        )),
                    ),
            )
            .into_any_element()
    }

    fn scope_switch(&self, scope: Scope, can_app: bool, cx: &mut Context<Self>) -> AnyElement {
        let choice = |id: &'static str, label: &str, this: Scope, enabled: bool| {
            let chosen = scope == this;
            let b = button(id, label, enabled);
            let b = if chosen {
                b.bg(rgb(colors().chosen)).text_color(rgb(colors().text))
            } else {
                b
            };
            b.when(enabled && !chosen, |b| {
                b.on_click(cx.listener(move |menu, _, _, cx| {
                    menu.shared.update(cx, |s, cx| {
                        s.vm.set_scope(this);
                        cx.notify();
                    });
                }))
            })
        };
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_1()
            .px_1()
            .pb_1()
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(choice("scope-all", "All apps", Scope::AllApps, true))
                    .child(choice("scope-app", "This app", Scope::ThisApp, can_app)),
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
            .into_any_element()
    }

    fn message(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let m = self.shared.read(cx).vm.message().cloned()?;
        let color = match m.kind {
            MessageKind::Info => colors().accent,
            MessageKind::Error => colors().red,
        };
        Some(
            div()
                .id("message")
                .flex_none()
                .mx_1()
                .px_2()
                .py_1()
                .rounded(px(4.))
                .bg(rgb(colors().panel))
                .border_l_2()
                .border_color(rgb(color))
                .text_size(px(12.))
                .text_color(rgb(color))
                .cursor_pointer()
                // Resting the pointer on a message keeps it; a click dismisses it.
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    let hovered = *hovered;
                    this.shared.update(cx, |s, _| s.vm.hold_message(hovered));
                }))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.shared.update(cx, |s, cx| {
                        s.vm.dismiss_message();
                        s.changed(cx);
                    });
                }))
                .child(m.text)
                .into_any_element(),
        )
    }

    fn body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (&'static str, Vec<AnyElement>) {
        let shared = self.shared.clone();
        match self.kind {
            MenuKind::Favorites => {
                if let Some(initial) = shared.update(cx, |s, _| s.vm.take_rename_seed()) {
                    self.field
                        .update(cx, |f, cx| f.reset(&initial, "Display name", cx));
                    window.focus(&self.field.focus_handle(cx));
                }
                let (lines, busy) = {
                    let s = shared.read(cx);
                    (s.vm.section_lines(Section::Favorites), s.vm.busy())
                };
                let empty = lines.is_empty();
                let prompt = self.prompt(
                    "Display name (blank restores the folder name)",
                    "Save",
                    busy,
                    cx,
                );
                let mut out = render_lines(lines, &shared, LABEL_CHARS, Some(prompt));
                if empty {
                    out.push(Self::quiet("No favorites yet"));
                }
                ("FAVORITES", out)
            }
            MenuKind::Places => {
                let lines = shared.read(cx).vm.section_lines(Section::Places);
                ("PLACES", render_lines(lines, &shared, LABEL_CHARS, None))
            }
            MenuKind::RecentFolders | MenuKind::RecentFiles => {
                let files = self.kind == MenuKind::RecentFiles;
                let (on, scope, can_app, lines) = {
                    let s = shared.read(cx);
                    (
                        s.vm.recents_enabled(),
                        s.vm.scope(),
                        s.vm.can_filter_by_app(),
                        s.vm.section_lines(if files {
                            Section::RecentFiles
                        } else {
                            Section::RecentFolders
                        }),
                    )
                };
                let title = if files {
                    "RECENT FILES"
                } else {
                    "RECENT FOLDERS"
                };
                if !on {
                    return (
                        title,
                        vec![Self::quiet("Recents are turned off in the settings file")],
                    );
                }
                let empty = lines.is_empty();
                let mut out = vec![self.scope_switch(scope, can_app, cx)];
                out.extend(render_lines(lines, &shared, LABEL_CHARS, None));
                if empty {
                    out.push(Self::quiet("Nothing yet"));
                }
                (title, out)
            }
            MenuKind::NewFolder => {
                let busy = shared.read(cx).vm.busy();
                let prompt = self.prompt("Name (a/b makes nested folders)", "Create", busy, cx);
                ("NEW FOLDER", vec![prompt])
            }
            MenuKind::Find => {
                let (known, ranked, found, running, status, has_query) = {
                    let s = shared.read(cx);
                    (
                        s.vm.folder().is_some(),
                        s.vm.section_lines(Section::Ranked),
                        s.vm.section_lines(Section::Found),
                        s.vm.search_running(),
                        s.vm.search_status(),
                        !s.vm.find_query().trim().is_empty(),
                    )
                };
                let search = if running {
                    button("search-stop", "Stop", true).on_click(cx.listener(|this, _, _, cx| {
                        this.shared.update(cx, |s, cx| s.stop_search(cx));
                    }))
                } else {
                    let can = known && has_query;
                    button("search-go", "Search below this folder", can).when(can, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| {
                            this.shared.update(cx, |s, cx| s.search(cx));
                        }))
                    })
                };
                let mut out = vec![div().px_1().child(self.find.clone()).into_any_element()];
                out.extend(render_lines(ranked, &shared, LABEL_CHARS, None));
                out.push(div().flex().px_1().child(search).into_any_element());
                if let Some(text) = status {
                    out.push(Self::quiet(&text));
                }
                out.extend(render_lines(found, &shared, LABEL_CHARS, None));
                ("FIND", out)
            }
            MenuKind::Toast => ("", Vec::new()),
        }
    }
}

impl Render for MenuWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.shared.update(cx, |s, _| s.fit_menu());
        let message = self.message(cx);
        let frame = div()
            .track_focus(&self.root)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .flex()
            .flex_col()
            .gap_1()
            .p_1()
            .overflow_hidden()
            .bg(rgb(colors().bg))
            .border_1()
            .border_color(rgb(colors().field_border))
            .text_color(rgb(colors().text))
            .text_size(px(13.));
        if self.kind == MenuKind::Toast {
            return frame.justify_center().children(message);
        }
        let (title, body) = self.body(window, cx);
        frame.child(Self::title(title)).children(message).child(
            div()
                .id("menu-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_px()
                .children(body),
        )
    }
}
