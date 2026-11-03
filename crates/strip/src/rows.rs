//! Drawing the lines of a folder list in a rail's menu; every click is
//! forwarded to the shared state, which decides what it means. Choosing a
//! folder or a file ends the menu's job, so it closes.

use gpui::{
    AnyElement, ClipboardItem, Entity, IntoElement, ParentElement, Styled, div, prelude::*, px,
    rgb, svg,
};

use crate::colors::colors;
use crate::expand::Section;
use crate::lines::{FolderLine, Icon, Line};
use crate::paths::shorten_middle;
use crate::view::{Shared, button, tip};

/// Pixels a subfolder is indented per level.
const INDENT: f32 = 12.;

/// Element names for one list; every element needs an id that no other list
/// uses.
struct Names {
    row: &'static str,
    arrow: &'static str,
    dots: &'static str,
    act: &'static str,
    more: &'static str,
}

fn names(section: Section) -> Names {
    match section {
        Section::Favorites => Names {
            row: "fav-row",
            arrow: "fav-arrow",
            dots: "fav-dots",
            act: "fav-act",
            more: "fav-more",
        },
        Section::Places => Names {
            row: "place-row",
            arrow: "place-arrow",
            dots: "place-dots",
            act: "place-act",
            more: "place-more",
        },
        Section::RecentFolders => Names {
            row: "recent-row",
            arrow: "recent-arrow",
            dots: "recent-dots",
            act: "recent-act",
            more: "recent-more",
        },
        Section::RecentFiles => Names {
            row: "file-row",
            arrow: "file-arrow",
            dots: "file-dots",
            act: "file-act",
            more: "file-more",
        },
        Section::Ranked => Names {
            row: "rank-row",
            arrow: "rank-arrow",
            dots: "rank-dots",
            act: "rank-act",
            more: "rank-more",
        },
        Section::Found => Names {
            row: "found-row",
            arrow: "found-arrow",
            dots: "found-dots",
            act: "found-act",
            more: "found-more",
        },
    }
}

/// A small folder drawn from rectangles, so no icon font is needed.
fn icon(kind: Icon, missing: bool) -> impl IntoElement {
    let color = if missing {
        colors().amber
    } else if kind == Icon::Drive {
        colors().drive
    } else {
        colors().accent
    };
    if kind == Icon::Drive {
        return div()
            .flex_none()
            .w(px(16.))
            .h(px(14.))
            .flex()
            .items_center()
            .child(div().w(px(15.)).h(px(7.)).rounded(px(2.)).bg(rgb(color)))
            .into_any_element();
    }
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
        .into_any_element()
}

fn file_icon() -> impl IntoElement {
    div()
        .flex_none()
        .w(px(16.))
        .h(px(14.))
        .flex()
        .justify_center()
        .child(
            div()
                .w(px(10.))
                .h(px(14.))
                .rounded(px(2.))
                .bg(rgb(colors().muted)),
        )
}

/// Turns lines into elements. `chars` is how many characters of a label fit
/// at the top level of this panel; `prompt` is placed where a rename prompt
/// line asks for it.
pub fn render_lines(
    lines: Vec<Line>,
    shared: &Entity<Shared>,
    chars: usize,
    mut prompt: Option<AnyElement>,
) -> Vec<AnyElement> {
    lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| match line {
            Line::Folder(f) => folder_row(i, f, shared, chars),
            Line::File {
                path,
                label,
                subtitle,
            } => {
                let n = names(Section::RecentFiles);
                let s = shared.clone();
                let target = path.clone();
                div()
                    .id((n.row, i))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(colors().hover)))
                    .tooltip(tip(path.display().to_string()))
                    .on_click(move |_, _, cx| {
                        s.update(cx, |s, cx| {
                            let job = s.vm.go_to_file_folder(&target);
                            s.start(job, cx);
                            s.after_choice(cx);
                        });
                    })
                    .child(file_icon())
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .whitespace_nowrap()
                                    .child(shorten_middle(&label, chars)),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(colors().muted))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(subtitle),
                            ),
                    )
                    .into_any_element()
            }
            Line::Loading(depth) => note(depth, "loading...", colors().muted),
            Line::Failed(depth, why) => note(depth, &why, colors().red),
            Line::Empty(depth) => note(depth, "no subfolders", colors().muted),
            Line::RenamePrompt(_) => prompt.take().unwrap_or_else(|| div().into_any_element()),
            Line::Menu { key, depth, items } => {
                let n = names(key.section);
                let mut row = div()
                    .flex_none()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .pb_1()
                    .pl(px(16. + depth as f32 * INDENT));
                for (k, (action, label)) in items.into_iter().enumerate() {
                    let s = shared.clone();
                    let path = key.path.clone();
                    row = row.child(button((n.act, i * 16 + k), &label, true).on_click(
                        move |_, _, cx| {
                            let copy = s.update(cx, |s, cx| s.menu_action(action, &path, cx));
                            if let Some(text) = copy {
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                            }
                        },
                    ));
                }
                row.into_any_element()
            }
            Line::More { section, hidden } => {
                let n = names(section);
                let s = shared.clone();
                let text = if hidden == 0 {
                    "Show fewer".to_owned()
                } else {
                    format!("Show {hidden} more")
                };
                div()
                    .id((n.more, i))
                    .flex_none()
                    .px_2()
                    .py_1()
                    .text_size(px(12.))
                    .text_color(rgb(colors().accent))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        s.update(cx, |s, cx| {
                            s.vm.toggle_show_all(section);
                            cx.notify();
                        });
                    })
                    .child(text)
                    .into_any_element()
            }
        })
        .collect()
}

fn note(depth: usize, text: &str, color: u32) -> AnyElement {
    div()
        .flex_none()
        .pl(px(34. + depth as f32 * INDENT))
        .pr_2()
        .text_size(px(11.))
        .text_color(rgb(color))
        .child(text.to_owned())
        .into_any_element()
}

fn folder_row(i: usize, f: FolderLine, shared: &Entity<Shared>, chars: usize) -> AnyElement {
    let n = names(f.key.section);
    let missing = f.missing;
    let title_color = if missing {
        colors().muted
    } else {
        colors().text
    };
    let limit = chars
        .saturating_sub(f.depth * 2)
        .saturating_sub(if missing || f.is_current { 7 } else { 0 })
        .max(6);

    let arrow = {
        let s = shared.clone();
        let key = f.key.clone();
        div()
            .id((n.arrow, i))
            .flex_none()
            .w(px(16.))
            .h(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(colors().hover)))
            .on_click(move |_, _, cx| {
                s.update(cx, |s, cx| s.expand(key.clone(), cx));
            })
            .child(
                svg()
                    .path(if f.open {
                        "icons/chevron-down.svg"
                    } else {
                        "icons/chevron-right.svg"
                    })
                    .size(px(12.))
                    .text_color(rgb(colors().muted)),
            )
    };

    let main = {
        let s = shared.clone();
        let path = f.key.path.clone();
        div()
            .id((n.row, i))
            .flex_1()
            .min_w(px(0.))
            .overflow_hidden()
            .px_1()
            .py_1()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(4.))
            .when(f.highlighted, |d| d.bg(rgb(colors().highlight)))
            .when(!missing, |d| {
                d.cursor_pointer().hover(|s| s.bg(rgb(colors().hover)))
            })
            .on_click(move |_, _, cx| {
                s.update(cx, |s, cx| {
                    let job = s.vm.go_to(&path);
                    let going = job.is_some();
                    s.start(job, cx);
                    if going {
                        s.after_choice(cx);
                    }
                });
            })
            .tooltip(tip(if missing {
                format!("{} (folder is missing)", f.full_path)
            } else {
                f.full_path.clone()
            }))
            .child(icon(f.icon, missing))
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
                            // Cut by character count: the ellipsis style does
                            // not apply to a flex item.
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .when(missing, |d| d.line_through())
                                    .child(shorten_middle(&f.label, limit)),
                            )
                            .when(missing, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .text_size(px(10.))
                                        .text_color(rgb(colors().amber))
                                        .child("MISSING"),
                                )
                            })
                            .when(f.is_current, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .text_size(px(10.))
                                        .text_color(rgb(colors().accent))
                                        .child("HERE"),
                                )
                            }),
                    )
                    .children(f.subtitle.clone().map(|sub| {
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(colors().muted))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(sub)
                    })),
            )
    };

    let dots = {
        let s = shared.clone();
        let key = f.key.clone();
        div()
            .id((n.dots, i))
            .flex_none()
            .px_1()
            .h(px(24.))
            .flex()
            .items_center()
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(colors().hover)))
            .tooltip(tip("More".to_owned()))
            .on_click(move |_, _, cx| {
                s.update(cx, |s, cx| {
                    s.vm.toggle_menu(key.clone());
                    cx.notify();
                });
            })
            .child(
                svg()
                    .path("icons/dots.svg")
                    .size(px(16.))
                    .text_color(rgb(colors().muted)),
            )
    };

    div()
        .flex_none()
        .rounded(px(5.))
        .when(f.menu_open, |d| d.bg(rgb(colors().panel)))
        .child(
            div()
                .flex()
                .items_center()
                .pl(px(f.depth as f32 * INDENT))
                .child(arrow)
                .child(main)
                .child(dots),
        )
        .into_any_element()
}
