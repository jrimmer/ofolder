//! A rail icon's tooltip. A rail is too narrow to hold one, so each shows in
//! its own small window beside the rail, placed by whoever docks the strip.
//! This module holds the pure parts (when a tooltip shows, how its text is
//! broken into lines and how big its window is) and the window's view.

use gpui::{Context, IntoElement, Render, Window, div, prelude::*, px, rgb};

use crate::colors::colors;
use crate::rail::RailSide;

/// How long the pointer rests on an icon before its tooltip shows.
pub const DELAY_MS: u64 = 400;

/// The longest line, in characters, before it is broken.
const MAX_CHARS: usize = 60;
/// A generous width of one character at the tooltip's text size, so the
/// window is never narrower than its text.
const CHAR_W: f32 = 7.2;
const LINE_H: f32 = 17.0;
/// Padding inside the window, each side.
const PAD_X: f32 = 8.0;
const PAD_Y: f32 = 5.0;
const MIN_W: f32 = 40.0;

/// Where a tooltip's icon is on its rail: the icon's centre, from the
/// rail's top or, for the mark at the rail's foot, from its bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipAnchor {
    Top(i32),
    Bottom(i32),
}

/// A tooltip one icon asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    /// The icon's element id; one tooltip per icon.
    pub key: &'static str,
    pub side: RailSide,
    pub anchor: TipAnchor,
    pub text: String,
}

/// What the caller should do after the pointer moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Wait the delay, then call [`Hints::due`] with this serial.
    Wait(u64),
    /// A tooltip is already up: replace it with the new one now.
    ShowNow,
    /// Take the shown tooltip down.
    Hide,
    Nothing,
}

/// Which tooltip is wanted and which is showing. The pointer resting on an
/// icon wants its tooltip after a delay; moving straight to another icon
/// while one shows replaces it at once.
#[derive(Debug, Default)]
pub struct Hints {
    serial: u64,
    wanted: Option<Hint>,
    showing: bool,
}

impl Hints {
    pub fn enter(&mut self, hint: Hint) -> Step {
        if self.wanted.as_ref() == Some(&hint) {
            return Step::Nothing;
        }
        self.serial += 1;
        self.wanted = Some(hint);
        if self.showing {
            Step::ShowNow
        } else {
            Step::Wait(self.serial)
        }
    }

    /// The pointer left the icon `key`. Leaving one icon after entering the
    /// next (the order is not guaranteed) changes nothing.
    pub fn leave(&mut self, key: &str) -> Step {
        if self.wanted.as_ref().is_none_or(|h| h.key != key) {
            return Step::Nothing;
        }
        self.serial += 1;
        self.wanted = None;
        if std::mem::take(&mut self.showing) {
            Step::Hide
        } else {
            Step::Nothing
        }
    }

    /// The delay for `serial` ran out: the tooltip to show, if the pointer
    /// is still where it was.
    pub fn due(&self, serial: u64) -> Option<&Hint> {
        (serial == self.serial)
            .then_some(self.wanted.as_ref())
            .flatten()
    }

    /// The wanted tooltip is now on screen.
    pub fn shown(&mut self) {
        self.showing = self.wanted.is_some();
    }

    pub fn wanted(&self) -> Option<&Hint> {
        self.wanted.as_ref()
    }

    /// A click, a menu or the keyboard going elsewhere: no tooltip, and none
    /// pending. True when one was showing.
    pub fn clear(&mut self) -> bool {
        self.serial += 1;
        self.wanted = None;
        std::mem::take(&mut self.showing)
    }
}

/// The text broken into lines no longer than the limit: at a `/` or a
/// space where there is one, else anywhere.
pub fn lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest: Vec<char> = line.chars().collect();
        while rest.len() > MAX_CHARS {
            let cut = rest[..MAX_CHARS]
                .iter()
                .rposition(|c| *c == '/' || *c == ' ')
                .filter(|i| *i > 0)
                .map_or(MAX_CHARS, |i| i + 1);
            out.push(rest[..cut].iter().collect::<String>().trim_end().to_owned());
            rest.drain(..cut);
        }
        out.push(rest.into_iter().collect());
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// The window's size for these lines, in logical pixels.
pub fn size(lines: &[String]) -> (i32, i32) {
    let widest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let w = (widest as f32 * CHAR_W + 2. * PAD_X).max(MIN_W);
    let h = lines.len() as f32 * LINE_H + 2. * PAD_Y;
    (w.ceil() as i32, h.ceil() as i32)
}

/// The tooltip's window.
pub struct TipWindow {
    lines: Vec<String>,
}

impl TipWindow {
    pub fn new(lines: Vec<String>) -> Self {
        TipWindow { lines }
    }
}

impl Render for TipWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_center()
            .px(px(PAD_X - 1.))
            .bg(rgb(colors().tooltip_bg))
            .border_1()
            .border_color(rgb(colors().hover))
            .text_size(px(12.))
            .line_height(px(LINE_H))
            .text_color(rgb(colors().text))
            .children(
                self.lines
                    .iter()
                    .map(|l| div().whitespace_nowrap().child(l.clone())),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(key: &'static str) -> Hint {
        Hint {
            key,
            side: RailSide::Right,
            anchor: TipAnchor::Top(30),
            text: key.to_owned(),
        }
    }

    #[test]
    fn resting_on_an_icon_shows_its_tooltip_after_the_delay() {
        let mut h = Hints::default();
        let Step::Wait(serial) = h.enter(hint("a")) else {
            panic!("expected a wait");
        };
        assert_eq!(h.due(serial).map(|t| t.key), Some("a"));
    }

    #[test]
    fn leaving_before_the_delay_runs_out_shows_nothing() {
        let mut h = Hints::default();
        let Step::Wait(serial) = h.enter(hint("a")) else {
            panic!("expected a wait");
        };
        assert_eq!(h.leave("a"), Step::Nothing);
        assert!(h.due(serial).is_none());
    }

    #[test]
    fn leaving_a_shown_tooltip_takes_it_down() {
        let mut h = Hints::default();
        h.enter(hint("a"));
        h.shown();
        assert_eq!(h.leave("a"), Step::Hide);
    }

    #[test]
    fn moving_to_the_next_icon_while_a_tooltip_shows_replaces_it_at_once() {
        let mut h = Hints::default();
        h.enter(hint("a"));
        h.shown();
        assert_eq!(h.enter(hint("b")), Step::ShowNow);
        // The late "left a" must not take b's tooltip down.
        assert_eq!(h.leave("a"), Step::Nothing);
        assert_eq!(h.wanted().map(|t| t.key), Some("b"));
    }

    #[test]
    fn an_older_delay_is_ignored_once_the_pointer_moved_on() {
        let mut h = Hints::default();
        let Step::Wait(first) = h.enter(hint("a")) else {
            panic!("expected a wait");
        };
        h.enter(hint("b"));
        assert!(h.due(first).is_none());
    }

    #[test]
    fn a_click_clears_the_tooltip_and_any_pending_one() {
        let mut h = Hints::default();
        let Step::Wait(serial) = h.enter(hint("a")) else {
            panic!("expected a wait");
        };
        assert!(!h.clear());
        assert!(h.due(serial).is_none());
        h.enter(hint("b"));
        h.shown();
        assert!(h.clear());
    }

    #[test]
    fn a_long_path_is_broken_after_a_slash_and_no_line_is_too_long() {
        let path = "/home/someone/Projects/a-rather-long-folder-name/and-another-one/deeper/still";
        let out = lines(&format!("Copy this folder's path\n{path}"));
        assert_eq!(out[0], "Copy this folder's path");
        assert!(
            out.iter().all(|l| l.chars().count() <= MAX_CHARS),
            "{out:?}"
        );
        assert!(out[1].ends_with('/'), "{out:?}");
        assert_eq!(out[1..].concat(), path);
    }

    #[test]
    fn a_line_with_nowhere_to_break_is_cut_at_the_limit() {
        let word = "x".repeat(MAX_CHARS + 5);
        let out = lines(&word);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].len(), MAX_CHARS);
    }

    #[test]
    fn the_window_grows_with_the_widest_line_and_the_line_count() {
        let one = size(&lines("Back"));
        let two = size(&lines("Copy this folder's path\n/home/someone"));
        assert!(two.0 > one.0 && two.1 > one.1);
        assert!(one.0 >= MIN_W as i32);
    }
}
