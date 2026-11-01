//! Where the strip sits beside a dialog, and where it goes when the right
//! side has no room.

/// A rectangle in logical pixels, the unit Hyprland reports for windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Right,
    Left,
    /// Neither side has room, so the strip overlaps the dialog's right edge.
    Overlap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub rect: Rect,
    pub side: Side,
}

/// The top-left corner that centres a window of `size` in `bounds`.
pub fn centered(bounds: Rect, size: (i32, i32)) -> (i32, i32) {
    (
        bounds.x + (bounds.w - size.0) / 2,
        bounds.y + (bounds.h - size.1) / 2,
    )
}

/// The strip's rectangle for a dialog, clamped to the monitor's usable area
/// when it is known.
pub fn place_strip(dialog: Rect, bounds: Option<Rect>, strip_width: i32, gap: i32) -> Placement {
    let mut x = dialog.right() + gap;
    let mut y = dialog.y;
    let mut h = dialog.h;
    let mut side = Side::Right;
    if let Some(b) = bounds {
        if x + strip_width > b.right() {
            let left = dialog.x - gap - strip_width;
            if left >= b.x {
                x = left;
                side = Side::Left;
            } else {
                x = b.right() - strip_width;
                side = Side::Overlap;
            }
        }
        h = h.min(b.h);
        y = y.min(b.bottom() - h).max(b.y);
    }
    Placement {
        rect: Rect {
            x,
            y,
            w: strip_width,
            h,
        },
        side,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: Rect = Rect {
        x: 0,
        y: 0,
        w: 4096,
        h: 1152,
    };

    fn dialog_at(x: i32) -> Rect {
        Rect {
            x,
            y: 289,
            w: 875,
            h: 600,
        }
    }

    #[test]
    fn the_strip_sits_a_gap_to_the_right_of_a_dialog_with_room() {
        let p = place_strip(dialog_at(1611), Some(MONITOR), 230, 6);
        assert_eq!(p.side, Side::Right);
        assert_eq!(
            p.rect,
            Rect {
                x: 2492,
                y: 289,
                w: 230,
                h: 600
            }
        );
    }

    #[test]
    fn the_strip_flips_left_when_the_dialog_is_against_the_right_edge() {
        let p = place_strip(dialog_at(3211), Some(MONITOR), 230, 6);
        assert_eq!(p.side, Side::Left);
        assert_eq!(p.rect.x, 3211 - 6 - 230);
        assert!(p.rect.right() <= 3211);
    }

    #[test]
    fn the_strip_overlaps_when_neither_side_has_room() {
        let narrow = Rect {
            x: 0,
            y: 0,
            w: 1000,
            h: 800,
        };
        let p = place_strip(
            Rect {
                x: 10,
                y: 10,
                w: 990,
                h: 600,
            },
            Some(narrow),
            230,
            6,
        );
        assert_eq!(p.side, Side::Overlap);
        assert_eq!(p.rect.right(), narrow.right());
    }

    #[test]
    fn the_strip_is_clamped_to_the_monitors_height_and_top() {
        let tall = Rect {
            x: 100,
            y: -50,
            w: 800,
            h: 2000,
        };
        let p = place_strip(tall, Some(MONITOR), 230, 6);
        assert_eq!(p.rect.h, MONITOR.h);
        assert_eq!(p.rect.y, MONITOR.y);
    }

    #[test]
    fn a_window_is_centred_in_the_monitors_usable_area() {
        assert_eq!(centered(MONITOR, (1000, 640)), (1548, 256));
    }

    #[test]
    fn without_monitor_bounds_the_strip_just_follows_the_dialog() {
        let p = place_strip(dialog_at(3211), None, 230, 6);
        assert_eq!(p.side, Side::Right);
        assert_eq!(p.rect.x, 3211 + 875 + 6);
    }
}
