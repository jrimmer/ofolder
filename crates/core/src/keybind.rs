//! Shortcuts as Hyprland key bindings: the Lua that adds and removes one,
//! and reading `hyprctl binds -j` to see which combinations the user's own
//! configuration already uses, so ofolder never takes one of those.

use std::path::Path;

use serde::Deserialize;

use crate::shortcuts::{ALT, CTRL, Combo, SHIFT, SUPER, canonical_key, command};

/// Every binding ofolder adds carries a description starting with this, so
/// its own bindings can be told from the user's, and found again after a
/// crash.
pub const OURS: &str = "ofolder: ";

/// One binding from `hyprctl binds -j`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Bind {
    #[serde(default)]
    pub modmask: u32,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub submap: String,
    #[serde(default)]
    pub mouse: bool,
}

impl Bind {
    pub fn is_ours(&self) -> bool {
        self.description.starts_with(OURS)
    }

    /// The modifiers that matter for matching: Caps Lock and Num Lock bits
    /// are left out.
    fn mods(&self) -> u8 {
        (self.modmask & u32::from(SUPER | CTRL | ALT | SHIFT)) as u8
    }

    /// How `hl.unbind` names this binding, for one of ofolder's own.
    pub fn bind_keys(&self) -> Option<String> {
        if self.key.is_empty() {
            return None;
        }
        Combo::new(self.mods(), &self.key)
            .ok()
            .map(|c| c.bind_keys())
    }
}

pub fn parse_binds(json: &str) -> Result<Vec<Bind>, serde_json::Error> {
    serde_json::from_str(json)
}

/// Keys a configuration may bind by key code instead of by name. Hyprland
/// reports such a binding with an empty key, so which key it is cannot be
/// read back; a combination on one of these keys is then taken to be in use.
fn often_bound_by_code(key: &str) -> bool {
    key.len() == 1 && key.as_bytes()[0].is_ascii_digit() || key == "minus" || key == "equal"
}

/// What the user's own configuration does with `combo`, when it binds it:
/// the binding's description, or a plain phrase when it has none. Bindings
/// ofolder added, mouse bindings and bindings inside a submap are not the
/// user's global keys and are left out.
pub fn taken_by(combo: &Combo, binds: &[Bind]) -> Option<String> {
    binds
        .iter()
        .filter(|b| !b.is_ours() && !b.mouse && b.submap.is_empty())
        .filter(|b| b.mods() == combo.modmask())
        .find(|b| {
            if b.key.is_empty() {
                often_bound_by_code(combo.key())
            } else {
                canonical_key(&b.key).as_deref() == Some(combo.key())
            }
        })
        .map(|b| {
            if b.description.trim().is_empty() {
                "a binding in your Hyprland configuration".to_owned()
            } else {
                format!(
                    "\"{}\" in your Hyprland configuration",
                    b.description.trim()
                )
            }
        })
}

/// The command a binding runs: this program, single-quoted for the shell,
/// with `do <command>`. Refuses a path that could break out of the quotes or
/// out of the Lua string it is put in.
pub fn do_command_line(exe: &Path, id: &str) -> Result<String, String> {
    let def = command(id).ok_or_else(|| format!("no command {id:?}"))?;
    let path = exe
        .to_str()
        .filter(|p| exe.is_absolute() && !p.is_empty())
        .ok_or_else(|| format!("refusing a program path that is not absolute text: {exe:?}"))?;
    if path
        .chars()
        .any(|c| matches!(c, '\'' | '"' | '\\') || c.is_control())
    {
        return Err(format!(
            "refusing a program path with quotes, backslashes or control characters: {path:?}"
        ));
    }
    Ok(format!("'{path}' do {}", def.id))
}

/// The Lua that binds `combo` to run `command_line`, described so it can be
/// found again. Refuses text that could end the Lua string.
pub fn bind_lua(combo: &Combo, command_line: &str, title: &str) -> Result<String, String> {
    let unsafe_text = |t: &str| t.chars().any(|c| matches!(c, '"' | '\\') || c.is_control());
    if unsafe_text(command_line) || unsafe_text(title) {
        return Err("refusing text with quotes, backslashes or control characters".to_owned());
    }
    Ok(format!(
        "hl.bind(\"{}\", hl.dsp.exec_cmd(\"{command_line}\"), {{ description = \"{OURS}{title}\" }})",
        combo.bind_keys()
    ))
}

/// The Lua that removes every binding of `combo`. Only combinations the
/// user's configuration does not bind are ever bound by ofolder, so this
/// removes only its own.
pub fn unbind_lua(combo: &Combo) -> String {
    format!("hl.unbind(\"{}\")", combo.bind_keys())
}

/// The same, for a binding named as `hl.bind` names it, read back from a
/// record; None when the text is not a combination.
pub fn unbind_lua_for(keys: &str) -> Option<String> {
    Combo::parse(keys).ok().map(|c| unbind_lua(&c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const BINDS: &str = r#"[
      {"modmask":72,"key":"F","description":"Full width","submap":"","mouse":false},
      {"modmask":64,"key":"","keycode":0,"description":"Switch to workspace 1","submap":"","mouse":false},
      {"modmask":64,"key":"RETURN","description":"","submap":"","mouse":false},
      {"modmask":90,"key":"Delete","description":"Lock with caps on","submap":"","mouse":false},
      {"modmask":72,"key":"mouse:272","description":"Move","submap":"","mouse":true},
      {"modmask":72,"key":"G","description":"In a submap","submap":"resize","mouse":false},
      {"modmask":72,"key":"F9","description":"ofolder: Places and drives","submap":"","mouse":false}
    ]"#;

    fn combo(text: &str) -> Combo {
        Combo::parse(text).expect("valid combination")
    }

    #[test]
    fn a_combination_the_users_configuration_binds_is_named() {
        let binds = parse_binds(BINDS).expect("parses");
        assert_eq!(
            taken_by(&combo("SUPER+ALT+F"), &binds).as_deref(),
            Some("\"Full width\" in your Hyprland configuration")
        );
        assert_eq!(
            taken_by(&combo("SUPER+ENTER"), &binds).as_deref(),
            Some("a binding in your Hyprland configuration")
        );
        // The caps-lock bit does not hide a binding.
        assert!(taken_by(&combo("SUPER+ALT+DELETE"), &binds).is_some());
    }

    #[test]
    fn a_digit_is_taken_when_the_configuration_binds_keys_by_code() {
        let binds = parse_binds(BINDS).expect("parses");
        assert!(taken_by(&combo("SUPER+3"), &binds).is_some());
        assert!(taken_by(&combo("SUPER+ALT+3"), &binds).is_none());
    }

    #[test]
    fn ofolders_own_mouse_and_submap_bindings_do_not_count() {
        let binds = parse_binds(BINDS).expect("parses");
        assert!(taken_by(&combo("SUPER+ALT+F9"), &binds).is_none());
        assert!(taken_by(&combo("SUPER+ALT+G"), &binds).is_none());
        assert!(taken_by(&combo("SUPER+ALT+H"), &binds).is_none());
        let ours: Vec<_> = binds.iter().filter(|b| b.is_ours()).collect();
        assert_eq!(ours.len(), 1);
        assert_eq!(ours[0].bind_keys().as_deref(), Some("SUPER + ALT + F9"));
    }

    #[test]
    fn a_binding_runs_this_program_with_the_command() {
        let line = do_command_line(&PathBuf::from("/home/u/.local/bin/ofolder"), "places")
            .expect("plain path");
        assert_eq!(line, "'/home/u/.local/bin/ofolder' do places");
        let lua = bind_lua(&combo("SUPER+ALT+F9"), &line, "Places and drives").expect("safe");
        assert_eq!(
            lua,
            "hl.bind(\"SUPER + ALT + F9\", hl.dsp.exec_cmd(\"'/home/u/.local/bin/ofolder' do places\"), \
             { description = \"ofolder: Places and drives\" })"
        );
        assert_eq!(
            unbind_lua(&combo("SUPER+ALT+F9")),
            "hl.unbind(\"SUPER + ALT + F9\")"
        );
        assert_eq!(
            unbind_lua_for("SUPER + ALT + F9").as_deref(),
            Some("hl.unbind(\"SUPER + ALT + F9\")")
        );
        assert_eq!(unbind_lua_for("\") os.execute(\"x"), None);
    }

    #[test]
    fn a_path_or_title_that_could_break_out_is_refused() {
        for bad in [
            "/tmp/it's/ofolder",
            "/tmp/a\"b/ofolder",
            "/tmp/a\\b/ofolder",
            "/tmp/a\nb/ofolder",
            "relative/ofolder",
        ] {
            assert!(
                do_command_line(&PathBuf::from(bad), "places").is_err(),
                "{bad:?}"
            );
        }
        assert!(do_command_line(&PathBuf::from("/usr/bin/ofolder"), "rm -rf").is_err());
        assert!(bind_lua(&combo("F9"), "x\") os.execute(\"y", "t").is_err());
        assert!(bind_lua(&combo("F9"), "ok", "t\"").is_err());
        // A space in the path is fine inside the single quotes.
        assert!(do_command_line(&PathBuf::from("/opt/my apps/ofolder"), "find").is_ok());
    }
}
