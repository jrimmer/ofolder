//! A one-line text field for the strip's two questions (a folder name, a
//! display name). It edits at the end of the text, which is all a short
//! name needs, and reads typed characters from the key event.

use crate::colors::colors;
use gpui::{
    App, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, KeyDownEvent, MouseButton,
    MouseDownEvent, Window, div, prelude::*, px, rgb,
};

pub enum FieldEvent {
    Submit(String),
    Cancel,
    /// The text changed; carries the new text.
    Changed(String),
    Up,
    Down,
}

pub struct TextField {
    text: String,
    placeholder: &'static str,
    focus: FocusHandle,
    /// The text, with the cursor, is wider than the box, as of the last frame. Then the end of it
    /// is shown, so what is being typed stays in view; otherwise the text starts at the left edge.
    overflowing: bool,
}

/// Room the box's padding takes from the text, left and right together.
const PADDING_PX: f32 = 16.0;

impl EventEmitter<FieldEvent> for TextField {}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Removes the last word and the spaces before it.
fn delete_word(text: &mut String) {
    while text.ends_with(char::is_whitespace) {
        text.pop();
    }
    while text.chars().last().is_some_and(|c| !c.is_whitespace()) {
        text.pop();
    }
}

impl TextField {
    pub fn new(cx: &mut Context<Self>) -> TextField {
        TextField {
            text: String::new(),
            placeholder: "",
            focus: cx.focus_handle(),
            overflowing: false,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn reset(&mut self, text: &str, placeholder: &'static str, cx: &mut Context<Self>) {
        self.text = text.to_owned();
        self.placeholder = placeholder;
        cx.notify();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = &ks.modifiers;
        let ctrl = m.control || m.platform;
        let before_text = self.text.clone();
        match ks.key.as_str() {
            "up" => cx.emit(FieldEvent::Up),
            "down" => cx.emit(FieldEvent::Down),
            "enter" => cx.emit(FieldEvent::Submit(self.text.clone())),
            "escape" => cx.emit(FieldEvent::Cancel),
            "backspace" if ctrl => delete_word(&mut self.text),
            "backspace" => {
                self.text.pop();
            }
            "u" if ctrl => self.text.clear(),
            "v" if ctrl => {
                if let Some(pasted) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    self.text
                        .push_str(pasted.lines().next().unwrap_or_default());
                }
            }
            "c" | "x" if ctrl => {
                cx.write_to_clipboard(ClipboardItem::new_string(self.text.clone()));
                if ks.key == "x" {
                    self.text.clear();
                }
            }
            _ => match &ks.key_char {
                Some(typed) if !ctrl && !m.alt && !typed.chars().any(char::is_control) => {
                    self.text.push_str(typed);
                }
                _ => return,
            },
        }
        if before_text != self.text {
            cx.emit(FieldEvent::Changed(self.text.clone()));
        }
        cx.notify();
        cx.stop_propagation();
    }

    fn on_mouse_down(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        cx.notify();
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let shown = if self.text.is_empty() {
            div()
                .flex_none()
                .whitespace_nowrap()
                .text_color(rgb(colors().placeholder))
                .child(self.placeholder)
        } else {
            div()
                .flex_none()
                .whitespace_nowrap()
                .child(self.text.clone())
        };
        let field = cx.entity();
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .w_full()
            .h(px(26.))
            .px_2()
            .relative()
            .flex()
            .items_center()
            // Text starts at the left edge. Once it is too long for the box it is pushed to the end,
            // so the part being typed shows.
            .when(self.overflowing, |d| d.justify_end())
            .overflow_hidden()
            // The text is the first child, the cursor (when there is one) the next and the probe,
            // which is as big as the box, the last.
            .on_children_prepainted(move |bounds, _, cx| {
                let (Some(text), Some(probe)) = (bounds.first(), bounds.last()) else {
                    return;
                };
                let cursor = if bounds.len() > 2 { 1.0 } else { 0.0 };
                let overflowing =
                    f32::from(text.size.width) + cursor > f32::from(probe.size.width) - PADDING_PX;
                if field.read(cx).overflowing != overflowing {
                    field.update(cx, |f, cx| {
                        f.overflowing = overflowing;
                        cx.notify();
                    });
                }
            })
            .rounded(px(4.))
            .bg(rgb(colors().field_bg))
            .border_1()
            .border_color(if focused {
                rgb(colors().accent)
            } else {
                rgb(colors().field_border)
            })
            .child(shown)
            .when(focused, |d| {
                d.child(
                    div()
                        .flex_none()
                        .w(px(1.))
                        .h(px(14.))
                        .bg(rgb(colors().accent)),
                )
            })
            .child(div().absolute().top_0().left_0().size_full())
    }
}

#[cfg(test)]
mod tests {
    use super::delete_word;

    #[test]
    fn deleting_a_word_removes_it_and_the_spaces_before_it() {
        let mut t = "my folder  ".to_owned();
        delete_word(&mut t);
        assert_eq!(t, "my ");
        delete_word(&mut t);
        assert_eq!(t, "");
    }
}
