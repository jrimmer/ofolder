//! The colours the panels draw with, all derived from the desktop palette.
//! This is the only place a colour is written as a number besides the
//! default palette in the core crate.

use std::sync::OnceLock;

use ofolder_core::theme::{Palette, Rgb};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colors {
    pub bg: Rgb,
    /// A raised block: a menu, a prompt, a message.
    pub panel: Rgb,
    /// The settings window's pane list.
    pub sidebar: Rgb,
    /// A button.
    pub raised: Rgb,
    pub hover: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
    pub accent: Rgb,
    pub red: Rgb,
    pub amber: Rgb,
    /// The icon of a mounted drive.
    pub drive: Rgb,
    /// The highlighted find result.
    pub highlight: Rgb,
    /// The chosen button of a pair.
    pub chosen: Rgb,
    pub tooltip_bg: Rgb,
    pub field_bg: Rgb,
    pub field_border: Rgb,
    pub placeholder: Rgb,
}

/// `a` moved `t` of the way to `b` (0.0 is `a`, 1.0 is `b`), channel by channel.
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let channel = |shift: u32| {
        let x = ((a >> shift) & 0xff) as f32;
        let y = ((b >> shift) & 0xff) as f32;
        (x + (y - x) * t).round().clamp(0., 255.) as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

impl Colors {
    pub fn from_palette(p: &Palette) -> Colors {
        // Dark themes sink into black for wells and tooltips, light themes
        // rise to white.
        let deep = if p.dark { 0x00_00_00 } else { 0xff_ff_ff };
        Colors {
            bg: p.background,
            panel: mix(p.background, p.surface, 0.5),
            sidebar: mix(p.background, deep, 0.18),
            raised: p.surface,
            hover: p.selection,
            text: p.foreground,
            muted: p.muted,
            accent: p.accent,
            red: p.danger,
            amber: p.warning,
            drive: mix(p.accent, p.foreground, 0.5),
            highlight: mix(p.background, p.accent, 0.25),
            chosen: mix(p.surface, p.accent, 0.4),
            tooltip_bg: mix(p.background, deep, 0.4),
            field_bg: mix(p.background, deep, 0.25),
            field_border: mix(p.background, p.foreground, 0.2),
            placeholder: mix(p.background, p.muted, 0.8),
        }
    }
}

static CURRENT: OnceLock<Colors> = OnceLock::new();

/// Sets the colours for this process, once. Later calls are ignored.
pub fn set(colors: Colors) {
    CURRENT.set(colors).ok();
}

/// The colours in use: those set at startup, else the default palette's.
pub fn colors() -> &'static Colors {
    CURRENT.get_or_init(|| Colors::from_palette(&Palette::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How bright a colour looks, 0 (black) to 255 (white).
    pub fn brightness(c: Rgb) -> f32 {
        let r = ((c >> 16) & 0xff) as f32;
        let g = ((c >> 8) & 0xff) as f32;
        let b = (c & 0xff) as f32;
        0.299 * r + 0.587 * g + 0.114 * b
    }

    fn light() -> Palette {
        Palette {
            dark: false,
            background: 0xf5_f1_e8,
            surface: 0xe3_dd_cc,
            foreground: 0x2b_2a_27,
            muted: 0x6e_6a_5f,
            accent: 0x1d_6f_a5,
            selection: 0xd2_ca_b4,
            warning: 0x9a_6a_00,
            danger: 0xb3_26_1e,
        }
    }

    #[test]
    fn mixing_moves_between_two_colours_channel_by_channel() {
        assert_eq!(mix(0x00_00_00, 0xff_ff_ff, 0.0), 0x00_00_00);
        assert_eq!(mix(0x00_00_00, 0xff_ff_ff, 1.0), 0xff_ff_ff);
        assert_eq!(mix(0x00_00_00, 0xff_ff_ff, 0.5), 0x80_80_80);
        assert_eq!(mix(0xff_00_00, 0x00_00_ff, 0.5), 0x80_00_80);
    }

    #[test]
    fn the_default_palette_gives_the_background_text_and_hover_the_panels_always_had() {
        let c = Colors::from_palette(&Palette::default());
        assert_eq!(c.bg, 0x1b_1d_23);
        assert_eq!(c.text, 0xe6_e6_e6);
        assert_eq!(c.hover, 0x3a_3f_4b);
        assert_eq!(c.raised, 0x2b_2e_36);
    }

    #[test]
    fn the_default_panel_shade_stays_within_a_few_steps_of_the_old_one() {
        let c = Colors::from_palette(&Palette::default());
        let old = 0x24_27_2f_u32;
        for shift in [16, 8, 0] {
            let a = ((c.panel >> shift) & 0xff) as i32;
            let b = ((old >> shift) & 0xff) as i32;
            assert!((a - b).abs() <= 4, "channel {shift}: {a} vs {b}");
        }
    }

    #[test]
    fn a_light_palette_gives_a_lighter_background_than_its_text() {
        let c = Colors::from_palette(&light());
        assert!(brightness(c.bg) > brightness(c.text) + 100.);
        assert!(brightness(c.bg) > brightness(c.muted));
    }

    #[test]
    fn a_dark_palette_gives_a_darker_background_than_its_text() {
        let c = Colors::from_palette(&Palette::default());
        assert!(brightness(c.text) > brightness(c.bg) + 100.);
    }

    #[test]
    fn wells_and_tooltips_go_darker_on_dark_themes_and_lighter_on_light_ones() {
        let dark = Colors::from_palette(&Palette::default());
        assert!(brightness(dark.field_bg) < brightness(dark.bg));
        assert!(brightness(dark.tooltip_bg) < brightness(dark.bg));
        let l = Colors::from_palette(&light());
        assert!(brightness(l.field_bg) > brightness(l.bg));
        assert!(brightness(l.tooltip_bg) > brightness(l.bg));
    }

    #[test]
    fn the_message_colours_come_straight_from_the_palette() {
        let p = light();
        let c = Colors::from_palette(&p);
        assert_eq!((c.red, c.amber, c.accent), (p.danger, p.warning, p.accent));
    }

    #[test]
    fn the_borders_hover_and_highlight_stay_distinct_from_the_background() {
        for p in [Palette::default(), light()] {
            let c = Colors::from_palette(&p);
            for shade in [c.hover, c.highlight, c.field_border, c.raised] {
                assert_ne!(shade, c.bg);
            }
        }
    }
}
