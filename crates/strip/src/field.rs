//! A one-line text field for the strip's two questions (a folder name, a
//! display name). It edits at the end of the text, which is all a short
//! name needs, and reads typed characters from the key event.

use gpui::{
    App, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Window, div,
    prelude::*, px, rgb,
};

pub enum FieldEvent {
    Submit(String),
    Cancel,
}

pub struct TextField {
    text: String,
    placeholder: &'static str,
    focus: FocusHandle,
}

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
        match ks.key.as_str() {
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
        cx.notify();
        cx.stop_propagation();
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let shown = if self.text.is_empty() {
            div()
                .flex_none()
                .whitespace_nowrap()
                .text_color(rgb(0x6c_72_80))
                .child(self.placeholder)
        } else {
            div()
                .flex_none()
                .whitespace_nowrap()
                .child(self.text.clone())
        };
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .w_full()
            .h(px(26.))
            .px_2()
            .flex()
            .items_center()
            // Justified to the end so a long name shows the part being typed.
            .justify_end()
            .overflow_hidden()
            .rounded(px(4.))
            .bg(rgb(0x14_16_1b))
            .border_1()
            .border_color(if focused {
                rgb(0x6c_a0_ff)
            } else {
                rgb(0x3a_3e_48)
            })
            .child(shown)
            .when(focused, |d| {
                d.child(div().flex_none().w(px(1.)).h(px(14.)).bg(rgb(0x6c_a0_ff)))
            })
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
