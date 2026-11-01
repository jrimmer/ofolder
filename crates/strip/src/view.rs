//! What the two panels share: one view-model, one poll of the dialog, one
//! place that runs the jobs that touch the dialog. Each panel
//! ([`crate::left`], [`crate::right`]) only draws and forwards clicks.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AnyView, App, Context, ElementId, Render, Stateful, Window, div, prelude::*, px, rgb};

use crate::StripActions;
use crate::model::{Job, Mode, ViewModel};

pub const BG: u32 = 0x1b_1d_23;
pub const PANEL: u32 = 0x24_27_2f;
pub const RAISED: u32 = 0x2f_33_3d;
pub const HOVER: u32 = 0x3a_3f_4b;
pub const TEXT: u32 = 0xe6_e6_e6;
pub const MUTED: u32 = 0x8a_90_9e;
pub const ACCENT: u32 = 0x6c_a0_ff;
pub const RED: u32 = 0xff_7b_72;
pub const AMBER: u32 = 0xe3_b3_41;

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

/// A hover tooltip with the given text.
pub fn tip(text: String) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| cx.new(|_| Tip(text.clone())).into()
}

/// A small button; a disabled one is dimmed and the caller adds no click.
pub fn button(id: impl Into<ElementId>, label: &str, enabled: bool) -> Stateful<gpui::Div> {
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

/// The state both panels show.
pub struct Shared {
    pub vm: ViewModel,
    actions: Arc<dyn StripActions>,
}

impl Shared {
    pub fn new(vm: ViewModel, actions: Arc<dyn StripActions>, cx: &mut Context<Self>) -> Self {
        let shared = Shared { vm, actions };
        shared.start_polling(cx);
        shared
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

    /// Runs a job that touches the dialog off the UI thread.
    pub fn run(&mut self, job: Job, cx: &mut Context<Self>) {
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

    /// Applies text typed into a prompt, for whichever prompt is open.
    pub fn submit(&mut self, text: &str, cx: &mut Context<Self>) {
        match self.vm.mode() {
            Mode::NewFolder => {
                if let Some(job) = self.vm.submit_new_folder(text) {
                    self.run(job, cx);
                }
            }
            Mode::Rename(_) => self.vm.submit_rename(text),
            _ => {}
        }
        cx.notify();
    }
}
