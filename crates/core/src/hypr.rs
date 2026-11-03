//! The parts of Hyprland's `hyprctl -j` output the strip reads.

use serde::Deserialize;

use crate::geometry::Rect;

#[derive(Debug, Clone, Deserialize)]
pub struct Workspace {
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Client {
    pub address: String,
    pub class: String,
    pub title: String,
    pub pid: u32,
    pub at: [i32; 2],
    pub size: [i32; 2],
    pub floating: bool,
    pub workspace: Workspace,
    pub monitor: i32,
}

impl Client {
    pub fn rect(&self) -> Rect {
        Rect {
            x: self.at[0],
            y: self.at[1],
            w: self.size[0],
            h: self.size[1],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Monitor {
    pub id: i32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f64,
    /// Space taken by bars: left, top, right, bottom.
    pub reserved: [i32; 4],
}

impl Monitor {
    /// The area windows can use, in the logical pixels window geometry is
    /// reported in: the panel size divided by the scale, less reserved space.
    pub fn usable_bounds(&self) -> Rect {
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        // Truncation is the intent: a fractional logical pixel cannot be placed.
        #[allow(clippy::cast_possible_truncation)]
        let (w, h) = (
            (f64::from(self.width) / scale) as i32,
            (f64::from(self.height) / scale) as i32,
        );
        let [left, top, right, bottom] = self.reserved;
        Rect {
            x: self.x + left,
            y: self.y + top,
            w: w - left - right,
            h: h - top - bottom,
        }
    }
}

/// Hyprland window addresses look like `0x5593b79c7380`. The address is put
/// into a Lua expression for focus dispatches, so anything else is refused:
/// a quote in it would end the string and run whatever followed.
pub fn is_window_address(text: &str) -> bool {
    text.strip_prefix("0x").is_some_and(|hex| {
        !hex.is_empty() && hex.len() <= 16 && hex.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

/// A rule or window class that is safe to put inside a Lua string and a
/// regular expression: letters, digits and the separators class names use.
pub fn is_plain_identifier(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 120
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// The Lua that installs a rule floating every window of `class` at `size`
/// and `at`. Refuses anything that is not a plain identifier, because both
/// the name and the class are spliced into code.
pub fn float_rule_lua(
    name: &str,
    class: &str,
    size: (i32, i32),
    at: (i32, i32),
    take_focus: bool,
) -> Result<String, String> {
    if !is_plain_identifier(name) || !is_plain_identifier(class) {
        return Err(format!(
            "refusing a rule name or class that is not a plain identifier: {name:?}, {class:?}"
        ));
    }
    // The class is matched as a regular expression, so its dots are literal.
    let pattern = class.replace('.', "\\\\.");
    Ok(format!(
        "hl.window_rule({{ name = \"{name}\", enabled = true, match = {{ class = \"^{pattern}$\" }}, float = true, size = \"{} {}\", move = \"{} {}\"{} }})",
        size.0,
        size.1,
        at.0,
        at.1,
        if take_focus {
            ""
        } else {
            ", no_initial_focus = true"
        }
    ))
}

/// The Lua for a rule that shows every window of `class` floating at `size`
/// and `at`, and never lets it take the keyboard: for a tooltip.
pub fn tip_rule_lua(
    name: &str,
    class: &str,
    size: (i32, i32),
    at: (i32, i32),
) -> Result<String, String> {
    let rule = float_rule_lua(name, class, size, at, false)?;
    Ok(rule.replacen(
        ", no_initial_focus = true",
        ", no_initial_focus = true, no_focus = true",
        1,
    ))
}

pub fn remove_rule_lua(name: &str) -> Result<String, String> {
    if !is_plain_identifier(name) {
        return Err(format!(
            "refusing a rule name that is not a plain identifier: {name:?}"
        ));
    }
    Ok(format!(
        "hl.window_rule({{ name = \"{name}\", enabled = false }})"
    ))
}

pub fn parse_clients(json: &str) -> Result<Vec<Client>, serde_json::Error> {
    serde_json::from_str(json)
}

pub fn parse_monitors(json: &str) -> Result<Vec<Monitor>, serde_json::Error> {
    serde_json::from_str(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITORS: &str = r#"[{"id":0,"name":"HDMI-A-2","width":5120,"height":1440,
        "x":0,"y":0,"scale":1.25,"reserved":[0,0,0,0],"transform":0}]"#;

    #[test]
    fn a_scaled_monitors_usable_area_is_in_logical_pixels() {
        let monitors = parse_monitors(MONITORS).expect("sample is valid");
        assert_eq!(
            monitors[0].usable_bounds(),
            Rect {
                x: 0,
                y: 0,
                w: 4096,
                h: 1152
            }
        );
    }

    #[test]
    fn reserved_bar_space_is_taken_off_the_usable_area() {
        let mut m = parse_monitors(MONITORS).expect("sample is valid").remove(0);
        m.reserved = [0, 40, 0, 0];
        let b = m.usable_bounds();
        assert_eq!((b.y, b.h), (40, 1112));
    }

    #[test]
    fn a_client_parses_and_ignores_the_fields_it_does_not_read() {
        let json = r#"[{"address":"0x1","class":"com.thisisgm.flea.picker","title":"Save",
            "pid":42,"at":[1611,289],"size":[875,600],"floating":true,
            "workspace":{"id":1,"name":"1"},"monitor":0,"mapped":true,"hidden":false}]"#;
        let clients = parse_clients(json).expect("sample is valid");
        assert_eq!(clients[0].pid, 42);
        assert_eq!(clients[0].rect().right(), 2486);
    }

    #[test]
    fn real_window_addresses_are_accepted() {
        assert!(is_window_address("0x5593b79c7380"));
        assert!(is_window_address("0x1"));
    }

    #[test]
    fn an_address_that_could_break_out_of_a_lua_string_is_refused() {
        for bad in [
            "",
            "0x",
            "5593b7",
            "0xZZ",
            "0x1\" }) os.execute(\"x\") --",
            "0x12 34",
            "0x11111111111111111",
        ] {
            assert!(!is_window_address(bad), "{bad}");
        }
    }

    #[test]
    fn the_float_rule_matches_the_class_exactly_with_literal_dots() {
        let lua = float_rule_lua(
            "ofolder-strip",
            "ofolder.strip",
            (230, 600),
            (2492, 289),
            true,
        )
        .expect("plain");
        assert!(lua.contains("class = \"^ofolder\\\\.strip$\""), "{lua}");
        assert!(
            lua.contains("size = \"230 600\"") && lua.contains("move = \"2492 289\""),
            "{lua}"
        );
    }

    #[test]
    fn a_rule_can_keep_a_window_from_taking_focus_when_it_opens() {
        let quiet = float_rule_lua("n", "c", (1, 1), (0, 0), false).expect("plain");
        let normal = float_rule_lua("n", "c", (1, 1), (0, 0), true).expect("plain");
        assert!(quiet.contains("no_initial_focus = true"), "{quiet}");
        assert!(!normal.contains("no_initial_focus"), "{normal}");
    }

    #[test]
    fn a_tip_rule_floats_its_window_and_never_lets_it_take_focus() {
        let lua = tip_rule_lua(
            "ofolder-strip-tip",
            "ofolder.strip.tip",
            (200, 30),
            (10, 20),
        )
        .expect("plain");
        assert!(
            lua.contains("class = \"^ofolder\\\\.strip\\\\.tip$\""),
            "{lua}"
        );
        assert!(lua.contains("no_initial_focus = true"), "{lua}");
        assert!(lua.contains("no_focus = true"), "{lua}");
        assert!(tip_rule_lua("n", "c\") x", (1, 1), (0, 0)).is_err());
    }

    #[test]
    fn a_rule_name_or_class_with_quotes_is_refused_before_it_reaches_lua() {
        assert!(float_rule_lua("a\" }) os.execute(\"x\") --", "c", (1, 1), (0, 0), true).is_err());
        assert!(float_rule_lua("n", "c\") x", (1, 1), (0, 0), true).is_err());
        assert!(remove_rule_lua("bad name").is_err());
        assert!(remove_rule_lua("ofolder-strip").is_ok());
    }

    #[test]
    fn a_zero_scale_is_treated_as_one_rather_than_dividing_by_zero() {
        let mut m = parse_monitors(MONITORS).expect("sample is valid").remove(0);
        m.scale = 0.0;
        assert_eq!(m.usable_bounds().w, 5120);
    }
}
