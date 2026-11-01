//! The desktop theme's colours, for the panels.
//!
//! Omarchy keeps each theme as a `colors.toml`. The panels read the one for
//! the theme in use when a dialog opens, so a theme change shows on the next
//! dialog. A missing or partly broken file falls back to the built-in dark
//! palette, colour by colour: a bad value costs only that colour.

use std::path::{Path, PathBuf};

use toml::{Table, Value};

/// A colour as `0xRRGGBB`.
pub type Rgb = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub dark: bool,
    pub background: Rgb,
    /// Raised surfaces: buttons and rows.
    pub surface: Rgb,
    pub foreground: Rgb,
    pub muted: Rgb,
    pub accent: Rgb,
    pub selection: Rgb,
    pub warning: Rgb,
    pub danger: Rgb,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            dark: true,
            background: 0x1b_1d_23,
            surface: 0x2b_2e_36,
            foreground: 0xe6_e6_e6,
            muted: 0x8a_90_9c,
            accent: 0x4f_c1_c9,
            selection: 0x3a_3f_4b,
            warning: 0xe7_b0_4c,
            danger: 0xf0_82_7a,
        }
    }
}

fn colour(table: &Table, key: &str) -> Option<Rgb> {
    let text = table.get(key)?.as_str()?.trim().strip_prefix('#')?;
    (text.len() == 6)
        .then(|| u32::from_str_radix(text, 16).ok())
        .flatten()
}

/// The palette in a `colors.toml`. Anything missing or not a `#rrggbb` colour
/// keeps the default for that colour; text that is not TOML at all gives the
/// default palette.
pub fn parse(colors_toml: &str) -> Palette {
    let d = Palette::default();
    let Ok(table) = colors_toml.parse::<Table>() else {
        return d;
    };
    let pick = |key: &str, fallback: Rgb| colour(&table, key).unwrap_or(fallback);
    Palette {
        dark: match table.get("mode") {
            Some(Value::String(m)) => m != "light",
            _ => d.dark,
        },
        background: pick("background", d.background),
        surface: pick("lighter_background", d.surface),
        foreground: pick("foreground", d.foreground),
        muted: pick("dark_foreground", d.muted),
        accent: pick("accent", d.accent),
        selection: pick("selection", d.selection),
        warning: pick("yellow", d.warning),
        danger: pick("red", d.danger),
    }
}

/// The folder name for a theme as `omarchy-theme-current` prints it
/// (`Theme Liftoff` or just `Liftoff`): lower case, spaces as hyphens.
pub fn theme_dir_name(printed: &str) -> Option<String> {
    let name = printed.trim();
    let name = name.strip_prefix("Theme ").unwrap_or(name).trim();
    if name.is_empty() || name.contains('/') || name.contains("..") {
        return None;
    }
    Some(name.to_lowercase().replace(' ', "-"))
}

/// The `colors.toml` for a theme: the user's own copy wins over the system's.
pub fn locate(
    name: &str,
    user_themes: &Path,
    system_themes: &Path,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    [user_themes, system_themes]
        .into_iter()
        .map(|root| root.join(name).join("colors.toml"))
        .find(|p| exists(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_theme_file_sets_the_palette() {
        let p = parse(
            "mode = \"dark\"\naccent = \"#89b4fa\"\nbackground = \"#1e1e2e\"\nlighter_background = \"#313244\"\nforeground = \"#cdd6f4\"\ndark_foreground = \"#6c7086\"\nselection = \"#45475a\"\nyellow = \"#f9e2af\"\nred = \"#f38ba8\"\n",
        );
        assert_eq!(
            (p.background, p.accent, p.foreground),
            (0x1e1e2e, 0x89b4fa, 0xcdd6f4)
        );
        assert_eq!(
            (p.surface, p.muted, p.selection),
            (0x313244, 0x6c7086, 0x45475a)
        );
        assert_eq!((p.warning, p.danger, p.dark), (0xf9e2af, 0xf38ba8, true));
    }

    #[test]
    fn a_light_theme_is_marked_light() {
        assert!(!parse("mode = \"light\"").dark);
    }

    #[test]
    fn a_bad_colour_costs_only_that_colour() {
        let p = parse("accent = \"blue\"\nbackground = \"#abcdef\"\nforeground = \"#12\"\nred = 5");
        let d = Palette::default();
        assert_eq!(p.background, 0xabcdef);
        assert_eq!(
            (p.accent, p.foreground, p.danger),
            (d.accent, d.foreground, d.danger)
        );
    }

    #[test]
    fn text_that_is_not_toml_gives_the_default_palette() {
        assert_eq!(parse("{{ not toml"), Palette::default());
        assert_eq!(parse(""), Palette::default());
    }

    #[test]
    fn the_theme_name_is_turned_into_its_folder_name() {
        assert_eq!(
            theme_dir_name("Theme Liftoff\n").as_deref(),
            Some("liftoff")
        );
        assert_eq!(
            theme_dir_name("Catppuccin Latte").as_deref(),
            Some("catppuccin-latte")
        );
        assert_eq!(theme_dir_name("   "), None);
    }

    #[test]
    fn a_theme_name_that_could_leave_the_themes_folder_is_refused() {
        assert_eq!(theme_dir_name("../../etc"), None);
        assert_eq!(theme_dir_name("a/b"), None);
    }

    #[test]
    fn the_users_own_copy_of_a_theme_wins_over_the_systems() {
        let user = Path::new("/home/j/.config/omarchy/themes");
        let system = Path::new("/usr/share/omarchy/themes");
        let both = |p: &Path| p.ends_with("liftoff/colors.toml");
        assert_eq!(
            locate("liftoff", user, system, &both),
            Some(user.join("liftoff/colors.toml"))
        );
        let only_system = |p: &Path| p.starts_with(system);
        assert_eq!(
            locate("liftoff", user, system, &only_system),
            Some(system.join("liftoff/colors.toml"))
        );
        assert_eq!(locate("liftoff", user, system, &|_| false), None);
    }
}
