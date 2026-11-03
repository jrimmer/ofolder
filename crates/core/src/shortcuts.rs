//! Keyboard shortcuts for the rail icons: which commands exist, how a key
//! combination is written, and which command holds which combination.
//!
//! A combination is kept in the settings file as `SUPER+ALT+F9` and handed to
//! Hyprland as `SUPER + ALT + F9`. The compositor, not ofolder, sees the keys,
//! so a combination is bound there while a dialog is open; the rules here
//! keep a binding from taking keys every application needs.

use std::collections::BTreeMap;
use std::fmt;

/// Which rail a command belongs to, for grouping them in the settings window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    GoSomewhere,
    DoSomethingHere,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::GoSomewhere => "Go somewhere",
            Group::DoSomethingHere => "Do something here",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandDef {
    /// The same id the rail layout uses for the icon, plus `settings`.
    pub id: &'static str,
    pub title: &'static str,
    pub group: Group,
}

const fn go(id: &'static str, title: &'static str) -> CommandDef {
    CommandDef {
        id,
        title,
        group: Group::GoSomewhere,
    }
}

const fn act(id: &'static str, title: &'static str) -> CommandDef {
    CommandDef {
        id,
        title,
        group: Group::DoSomethingHere,
    }
}

/// Every command a shortcut can run, in the order the settings window lists
/// them.
pub const COMMANDS: &[CommandDef] = &[
    go("back", "Back"),
    go("forward", "Forward"),
    go("parent", "Parent folder"),
    go("favorites", "Favorites"),
    go("places", "Places and drives"),
    go("recent_folders", "Recent folders"),
    go("recent_files", "Recent files"),
    act("new_folder", "New folder"),
    act("find", "Find a folder"),
    act("favorite", "Add or remove favorite"),
    act("copy_path", "Copy this folder's path"),
    act("file_manager", "Open in file manager"),
    act("terminal", "Open in terminal"),
    act("settings", "Settings"),
    act("preview", "Show or hide the preview"),
];

pub fn command(id: &str) -> Option<&'static CommandDef> {
    COMMANDS.iter().find(|c| c.id == id)
}

/// Modifier bits, as Hyprland numbers them in `hyprctl binds -j`.
pub const SHIFT: u8 = 1;
pub const CTRL: u8 = 4;
pub const ALT: u8 = 8;
pub const SUPER: u8 = 64;

/// The modifiers in the order they are written.
const MOD_NAMES: [(u8, &str, &str); 4] = [
    (SUPER, "SUPER", "Super"),
    (CTRL, "CTRL", "Ctrl"),
    (ALT, "ALT", "Alt"),
    (SHIFT, "SHIFT", "Shift"),
];

/// Punctuation keys by their key names, with the character each stands for.
const PUNCTUATION: [(&str, char); 11] = [
    ("comma", ','),
    ("period", '.'),
    ("slash", '/'),
    ("semicolon", ';'),
    ("apostrophe", '\''),
    ("bracketleft", '['),
    ("bracketright", ']'),
    ("minus", '-'),
    ("equal", '='),
    ("grave", '`'),
    ("backslash", '\\'),
];

/// Named keys: the canonical name, then other spellings accepted for it.
const NAMED: [(&str, &[&str]); 15] = [
    ("SPACE", &[]),
    ("RETURN", &["ENTER"]),
    ("TAB", &[]),
    ("BACKSPACE", &[]),
    ("DELETE", &["DEL"]),
    ("INSERT", &["INS"]),
    ("HOME", &[]),
    ("END", &[]),
    ("Page_Up", &["PAGEUP", "PRIOR", "PGUP"]),
    ("Page_Down", &["PAGEDOWN", "NEXT", "PGDN"]),
    ("LEFT", &[]),
    ("RIGHT", &[]),
    ("UP", &[]),
    ("DOWN", &[]),
    ("ESCAPE", &["ESC"]),
];

/// Why a key combination cannot be a shortcut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComboError {
    /// Only modifiers were pressed.
    ModifierOnly,
    /// A key that takes nothing from typing needs no modifier, but only the
    /// function keys are such keys.
    NeedsModifier,
    /// Ctrl or Shift alone would take keys every application uses.
    NeedsSuperOrAlt,
    /// Escape cancels and closes things everywhere.
    Escape,
    Unknown(String),
}

impl fmt::Display for ComboError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComboError::ModifierOnly => write!(f, "a modifier on its own is not a shortcut"),
            ComboError::NeedsModifier => write!(
                f,
                "add Super or Alt: only F1 to F12 can be a shortcut on their own"
            ),
            ComboError::NeedsSuperOrAlt => write!(
                f,
                "add Super or Alt: with only Ctrl or Shift the key would be taken from every \
                 application while a dialog is open"
            ),
            ComboError::Escape => write!(f, "Escape cannot be a shortcut"),
            ComboError::Unknown(key) => write!(f, "{key:?} is not a key ofolder can bind"),
        }
    }
}

impl std::error::Error for ComboError {}

/// A modifier set and one key.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Combo {
    mods: u8,
    /// The canonical key name.
    key: String,
}

fn is_function_key(key: &str) -> bool {
    key.strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=12).contains(&n))
}

/// The canonical name of a key, from any accepted spelling, or None.
pub fn canonical_key(name: &str) -> Option<String> {
    let upper = name.trim().to_ascii_uppercase();
    if upper.len() == 1 {
        let c = upper.chars().next()?;
        if c.is_ascii_alphanumeric() {
            return Some(upper);
        }
        return PUNCTUATION
            .iter()
            .find(|(_, ch)| *ch == c)
            .map(|(n, _)| (*n).to_owned());
    }
    if is_function_key(&upper) {
        return Some(upper);
    }
    if let Some((n, _)) = PUNCTUATION
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(&upper))
    {
        return Some((*n).to_owned());
    }
    NAMED
        .iter()
        .find(|(n, aliases)| n.eq_ignore_ascii_case(&upper) || aliases.contains(&upper.as_str()))
        .map(|(n, _)| (*n).to_owned())
}

fn modifier_bit(name: &str) -> Option<u8> {
    match name.trim().to_ascii_uppercase().as_str() {
        "SUPER" | "LOGO" | "MOD4" | "WIN" | "META" => Some(SUPER),
        "CTRL" | "CONTROL" => Some(CTRL),
        "ALT" | "MOD1" => Some(ALT),
        "SHIFT" => Some(SHIFT),
        _ => None,
    }
}

impl Combo {
    /// A combination, checked against the rules for a shortcut.
    pub fn new(mods: u8, key: &str) -> Result<Combo, ComboError> {
        if modifier_bit(key).is_some() {
            return Err(ComboError::ModifierOnly);
        }
        let key = canonical_key(key).ok_or_else(|| ComboError::Unknown(key.to_owned()))?;
        if key == "ESCAPE" {
            return Err(ComboError::Escape);
        }
        let mods = mods & (SUPER | CTRL | ALT | SHIFT);
        if !is_function_key(&key) {
            if mods == 0 {
                return Err(ComboError::NeedsModifier);
            }
            if mods & (SUPER | ALT) == 0 {
                return Err(ComboError::NeedsSuperOrAlt);
            }
        }
        Ok(Combo { mods, key })
    }

    /// Reads `SUPER+ALT+F9`, `super + alt + f9` and the like.
    pub fn parse(text: &str) -> Result<Combo, ComboError> {
        let parts: Vec<&str> = text
            .split('+')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
        // A trailing "+" was the plus key itself, which needs Shift and "=".
        let Some((key, mods)) = parts.split_last() else {
            return Err(ComboError::Unknown(text.to_owned()));
        };
        let mut bits = 0;
        for m in mods {
            bits |= modifier_bit(m).ok_or_else(|| ComboError::Unknown((*m).to_owned()))?;
        }
        Combo::new(bits, key)
    }

    pub fn modmask(&self) -> u8 {
        self.mods
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    fn joined(&self, sep: &str, pretty: bool) -> String {
        let mut parts: Vec<String> = MOD_NAMES
            .iter()
            .filter(|(bit, _, _)| self.mods & bit != 0)
            .map(|(_, raw, nice)| if pretty { *nice } else { *raw }.to_owned())
            .collect();
        let key = if pretty {
            match PUNCTUATION.iter().find(|(n, _)| *n == self.key) {
                Some((_, c)) => c.to_string(),
                None => self.key.replace('_', " "),
            }
        } else {
            self.key.clone()
        };
        parts.push(key);
        parts.join(sep)
    }

    /// How the settings file keeps it: `SUPER+ALT+F9`.
    pub fn to_setting(&self) -> String {
        self.joined("+", false)
    }

    /// How Hyprland's `hl.bind` takes it: `SUPER + ALT + F9`.
    pub fn bind_keys(&self) -> String {
        self.joined(" + ", false)
    }

    /// How the settings window shows it: `Super+Alt+F9`.
    pub fn label(&self) -> String {
        self.joined("+", true)
    }
}

/// Keys as gpui names them that differ from the canonical names, with the
/// unshifted key a shifted character comes from.
const SHIFTED: [(char, &str); 21] = [
    ('!', "1"),
    ('@', "2"),
    ('#', "3"),
    ('$', "4"),
    ('%', "5"),
    ('^', "6"),
    ('&', "7"),
    ('*', "8"),
    ('(', "9"),
    (')', "0"),
    ('_', "minus"),
    ('+', "equal"),
    ('<', "comma"),
    ('>', "period"),
    ('?', "slash"),
    (':', "semicolon"),
    ('"', "apostrophe"),
    ('{', "bracketleft"),
    ('}', "bracketright"),
    ('|', "backslash"),
    ('~', "grave"),
];

/// The modifiers held with a recorded key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_key: bool,
}

/// The combination for a key as it arrives in a window. The toolkit names a
/// shifted character by the character (`&` for Shift+7, `A` for Shift+a), but
/// Hyprland matches the unshifted key with Shift held, so that is what is
/// kept.
pub fn from_keystroke(held: Held, key: &str) -> Result<Combo, ComboError> {
    let mut mods = 0;
    for (on, bit) in [
        (held.ctrl, CTRL),
        (held.alt, ALT),
        (held.shift, SHIFT),
        (held.super_key, SUPER),
    ] {
        if on {
            mods |= bit;
        }
    }
    let lower = key.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "shift" | "control" | "ctrl" | "alt" | "platform" | "super" | "function" | "fn" | "meta"
    ) {
        return Err(ComboError::ModifierOnly);
    }
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.clone().next()) {
        if c.is_ascii_uppercase() {
            return Combo::new(mods | SHIFT, &c.to_string());
        }
        if let Some((_, base)) = SHIFTED.iter().find(|(s, _)| *s == c) {
            return Combo::new(mods | SHIFT, base);
        }
    }
    let name = match lower.as_str() {
        "enter" => "RETURN",
        "pageup" => "Page_Up",
        "pagedown" => "Page_Down",
        other => other,
    };
    Combo::new(mods, name)
}

/// Which command holds which combination. Every command has at most one, and
/// a combination belongs to at most one command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shortcuts {
    map: BTreeMap<&'static str, Combo>,
}

impl Shortcuts {
    pub fn get(&self, id: &str) -> Option<&Combo> {
        self.map.get(id)
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The command that holds `combo`.
    pub fn holder(&self, combo: &Combo) -> Option<&'static str> {
        self.map
            .iter()
            .find(|(_, c)| *c == combo)
            .map(|(id, _)| *id)
    }

    /// Sets or clears `id`'s combination. Refuses an unknown command and a
    /// combination another command holds; the caller settles that first.
    pub fn set(&mut self, id: &str, combo: Option<Combo>) -> bool {
        let Some(def) = command(id) else {
            return false;
        };
        match combo {
            Some(c) => {
                if self.holder(&c).is_some_and(|h| h != def.id) {
                    return false;
                }
                self.map.insert(def.id, c);
            }
            None => {
                self.map.remove(def.id);
            }
        }
        true
    }

    /// Gives `combo` to `id` and the combination `id` had (if any) to the
    /// command that held `combo`.
    pub fn swap(&mut self, id: &str, combo: Combo) -> bool {
        let Some(def) = command(id) else {
            return false;
        };
        let old = self.map.remove(def.id);
        if let Some(other) = self.holder(&combo) {
            match old {
                Some(o) => {
                    self.map.insert(other, o);
                }
                None => {
                    self.map.remove(other);
                }
            }
        }
        self.map.insert(def.id, combo);
        true
    }

    /// Gives `combo` to `id`, taking it from whichever command held it.
    pub fn take(&mut self, id: &str, combo: Combo) -> bool {
        if command(id).is_none() {
            return false;
        }
        if let Some(other) = self.holder(&combo) {
            self.map.remove(other);
        }
        self.set(id, Some(combo))
    }

    /// Every assigned command, in the order of [`COMMANDS`].
    pub fn iter(&self) -> impl Iterator<Item = (&'static CommandDef, &Combo)> {
        COMMANDS
            .iter()
            .filter_map(|def| self.map.get(def.id).map(|c| (def, c)))
    }

    /// Reads `[shortcuts]` entries: an unknown command or a value that is
    /// not a usable combination is reported and ignored, and a combination
    /// given twice goes to the first command and is reported for the second.
    pub fn from_entries<'a>(
        entries: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
        warnings: &mut Vec<String>,
    ) -> Shortcuts {
        let mut out = Shortcuts::default();
        let mut found: Vec<(&'static CommandDef, Combo)> = Vec::new();
        for (id, text) in entries {
            let Some(def) = command(id) else {
                warnings.push(format!(
                    "setting `shortcuts.{id}` is not a known command and was ignored"
                ));
                continue;
            };
            let Some(text) = text else {
                warnings.push(format!(
                    "setting `shortcuts.{id}` ignored: expected keys as text, like \"SUPER+ALT+F\""
                ));
                continue;
            };
            match Combo::parse(text) {
                Ok(combo) => found.push((def, combo)),
                Err(e) => warnings.push(format!("setting `shortcuts.{id}` ignored: {e}")),
            }
        }
        // First in the order of the commands wins, so the result does not
        // depend on how the file happens to order them.
        found.sort_by_key(|(def, _)| COMMANDS.iter().position(|c| c.id == def.id));
        for (def, combo) in found {
            if let Some(holder) = out.holder(&combo) {
                warnings.push(format!(
                    "setting `shortcuts.{}` ignored: {} is already the shortcut for `{holder}`",
                    def.id,
                    combo.to_setting()
                ));
            } else {
                out.map.insert(def.id, combo);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(ctrl: bool, alt: bool, shift: bool, super_key: bool) -> Held {
        Held {
            ctrl,
            alt,
            shift,
            super_key,
        }
    }

    #[test]
    fn every_rail_icon_and_the_settings_have_a_command_with_a_unique_id() {
        for icon in crate::rails::LEFT_ICONS
            .iter()
            .chain(crate::rails::RIGHT_ICONS)
        {
            assert!(command(icon).is_some(), "{icon} has no command");
        }
        assert!(command("settings").is_some());
        let mut ids: Vec<_> = COMMANDS.iter().map(|c| c.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), COMMANDS.len());
    }

    #[test]
    fn commands_are_grouped_by_the_rail_their_icon_is_on() {
        for icon in crate::rails::LEFT_ICONS {
            assert_eq!(command(icon).map(|c| c.group), Some(Group::GoSomewhere));
        }
        for icon in crate::rails::RIGHT_ICONS {
            assert_eq!(command(icon).map(|c| c.group), Some(Group::DoSomethingHere));
        }
    }

    #[test]
    fn a_combination_is_written_with_modifiers_in_a_fixed_order() {
        let c = Combo::parse("shift+alt + super+f9").expect("valid");
        assert_eq!(c.to_setting(), "SUPER+ALT+SHIFT+F9");
        assert_eq!(c.bind_keys(), "SUPER + ALT + SHIFT + F9");
        assert_eq!(c.label(), "Super+Alt+Shift+F9");
        assert_eq!(c.modmask(), SUPER | ALT | SHIFT);
    }

    #[test]
    fn other_spellings_of_keys_and_modifiers_are_accepted() {
        assert_eq!(
            Combo::parse("control+mod1+enter").map(|c| c.to_setting()),
            Ok("CTRL+ALT+RETURN".to_owned())
        );
        assert_eq!(
            Combo::parse("LOGO+pgup").map(|c| c.to_setting()),
            Ok("SUPER+Page_Up".to_owned())
        );
        assert_eq!(
            Combo::parse("SUPER+,").map(|c| c.to_setting()),
            Ok("SUPER+comma".to_owned())
        );
        assert_eq!(
            Combo::parse("SUPER+comma").map(|c| c.label()),
            Ok("Super+,".to_owned())
        );
    }

    #[test]
    fn a_function_key_may_stand_alone_but_other_keys_need_super_or_alt() {
        assert!(Combo::parse("F9").is_ok());
        assert!(Combo::parse("F12").is_ok());
        assert_eq!(Combo::parse("F13"), Err(ComboError::Unknown("F13".into())));
        assert_eq!(Combo::parse("A"), Err(ComboError::NeedsModifier));
        assert_eq!(Combo::parse("CTRL+C"), Err(ComboError::NeedsSuperOrAlt));
        assert_eq!(
            Combo::parse("CTRL+SHIFT+T"),
            Err(ComboError::NeedsSuperOrAlt)
        );
        assert!(Combo::parse("CTRL+ALT+T").is_ok());
        assert!(Combo::parse("SUPER+SHIFT+7").is_ok());
    }

    #[test]
    fn modifiers_alone_escape_and_nonsense_are_refused() {
        assert_eq!(Combo::parse("SUPER+ALT"), Err(ComboError::ModifierOnly));
        assert_eq!(Combo::parse("SUPER+ESC"), Err(ComboError::Escape));
        assert!(matches!(
            Combo::parse("SUPER+é"),
            Err(ComboError::Unknown(_))
        ));
        assert!(matches!(
            Combo::parse("HYPER+A"),
            Err(ComboError::Unknown(_))
        ));
        assert!(matches!(Combo::parse(""), Err(ComboError::Unknown(_))));
    }

    #[test]
    fn keys_as_the_toolkit_delivers_them_become_hyprland_key_names() {
        let k = |h, key| from_keystroke(h, key).map(|c| c.to_setting());
        let sa = held(false, true, false, true);
        assert_eq!(k(sa, "f"), Ok("SUPER+ALT+F".into()));
        assert_eq!(k(sa, "7"), Ok("SUPER+ALT+7".into()));
        assert_eq!(k(sa, "f9"), Ok("SUPER+ALT+F9".into()));
        assert_eq!(k(sa, "space"), Ok("SUPER+ALT+SPACE".into()));
        assert_eq!(k(sa, "enter"), Ok("SUPER+ALT+RETURN".into()));
        assert_eq!(k(sa, "left"), Ok("SUPER+ALT+LEFT".into()));
        assert_eq!(k(sa, "pageup"), Ok("SUPER+ALT+Page_Up".into()));
        assert_eq!(k(sa, ","), Ok("SUPER+ALT+comma".into()));
        assert_eq!(k(sa, "/"), Ok("SUPER+ALT+slash".into()));
        assert_eq!(k(sa, "["), Ok("SUPER+ALT+bracketleft".into()));
        assert_eq!(k(held(false, false, false, false), "f5"), Ok("F5".into()));
    }

    #[test]
    fn a_shifted_character_is_kept_as_its_key_with_shift_held() {
        let k = |h, key| from_keystroke(h, key).map(|c| c.to_setting());
        // Ctrl+Shift+7 arrives as "&", with or without the shift flag.
        assert_eq!(
            k(held(true, true, false, false), "&"),
            Ok("CTRL+ALT+SHIFT+7".into())
        );
        assert_eq!(
            k(held(false, true, true, true), "&"),
            Ok("SUPER+ALT+SHIFT+7".into())
        );
        assert_eq!(
            k(held(false, false, false, true), "A"),
            Ok("SUPER+SHIFT+A".into())
        );
        assert_eq!(
            k(held(false, true, true, false), "?"),
            Ok("ALT+SHIFT+slash".into())
        );
        assert_eq!(
            k(held(false, true, false, false), "_"),
            Ok("ALT+SHIFT+minus".into())
        );
    }

    #[test]
    fn a_modifier_pressed_on_its_own_is_not_recorded() {
        for key in ["shift", "control", "alt", "platform", "super"] {
            assert_eq!(
                from_keystroke(held(true, true, false, false), key),
                Err(ComboError::ModifierOnly)
            );
        }
    }

    #[test]
    fn a_combination_belongs_to_one_command() {
        let mut s = Shortcuts::default();
        let f9 = Combo::parse("SUPER+ALT+F9").expect("valid");
        assert!(s.set("places", Some(f9.clone())));
        assert!(!s.set("find", Some(f9.clone())), "held by places");
        assert_eq!(s.holder(&f9), Some("places"));
        assert!(!s.set("nonsense", Some(f9.clone())));
        assert!(s.set("places", None));
        assert!(s.is_empty());
    }

    #[test]
    fn swapping_gives_each_command_the_others_combination() {
        let mut s = Shortcuts::default();
        let a = Combo::parse("SUPER+ALT+A").expect("valid");
        let b = Combo::parse("SUPER+ALT+B").expect("valid");
        s.set("places", Some(a.clone()));
        s.set("find", Some(b.clone()));
        assert!(s.swap("find", a.clone()));
        assert_eq!(s.get("find"), Some(&a));
        assert_eq!(s.get("places"), Some(&b));
        // With nothing to give back, the other command is left without one.
        let mut t = Shortcuts::default();
        t.set("places", Some(a.clone()));
        t.swap("find", a.clone());
        assert_eq!(t.get("find"), Some(&a));
        assert_eq!(t.get("places"), None);
    }

    #[test]
    fn taking_a_combination_clears_it_from_the_command_that_had_it() {
        let mut s = Shortcuts::default();
        let a = Combo::parse("SUPER+ALT+A").expect("valid");
        s.set("places", Some(a.clone()));
        assert!(s.take("find", a.clone()));
        assert_eq!(s.get("find"), Some(&a));
        assert_eq!(s.get("places"), None);
    }

    #[test]
    fn entries_are_read_leniently_and_the_first_command_keeps_a_doubled_combination() {
        let mut warnings = Vec::new();
        let s = Shortcuts::from_entries(
            [
                ("find", Some("SUPER+ALT+F")),
                ("places", Some("super+alt+f")),
                ("junk", Some("SUPER+ALT+J")),
                ("back", Some("CTRL+C")),
                ("parent", None),
                ("terminal", Some("SUPER+ALT+T")),
            ],
            &mut warnings,
        );
        // Places comes before find in the command order, so it keeps F.
        assert_eq!(
            s.get("places").map(Combo::to_setting).as_deref(),
            Some("SUPER+ALT+F")
        );
        assert_eq!(s.get("find"), None);
        assert_eq!(
            s.get("terminal").map(Combo::to_setting).as_deref(),
            Some("SUPER+ALT+T")
        );
        assert_eq!(s.get("back"), None);
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("shortcuts.find")));
        assert!(warnings.iter().any(|w| w.contains("shortcuts.junk")));
        assert!(warnings.iter().any(|w| w.contains("shortcuts.back")));
        assert!(warnings.iter().any(|w| w.contains("shortcuts.parent")));
    }

    #[test]
    fn assigned_commands_come_back_in_command_order() {
        let mut s = Shortcuts::default();
        s.set(
            "terminal",
            Some(Combo::parse("SUPER+ALT+T").expect("valid")),
        );
        s.set("back", Some(Combo::parse("SUPER+ALT+B").expect("valid")));
        let ids: Vec<_> = s.iter().map(|(d, _)| d.id).collect();
        assert_eq!(ids, ["back", "terminal"]);
    }
}
