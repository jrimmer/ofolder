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
    /// Whether the point lies inside, counting the left and top edges.
    pub fn contains(&self, (x, y): (i32, i32)) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

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

/// How the two panels ended up around the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrangement {
    /// Left panel on the left of the dialog, right panel on its right.
    Split,
    /// No room on the left: both panels on the right, left panel first.
    PackedRight,
    /// No room on the right: both panels on the left, left panel first.
    PackedLeft,
    /// Neither side has room for both: against the monitor's right edge, over the dialog.
    Overlap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panels {
    pub left: Rect,
    pub right: Rect,
    pub arrangement: Arrangement,
}

/// Where the left panel (places to go) and the right panel (things to do)
/// belong around a dialog. Each stands on its own side when there is room.
/// When one side is short, both panels pack onto the other side in the same
/// order, and when neither side can hold both they overlap the dialog's edge.
/// Heights follow the dialog, clamped to the monitor's usable area.
pub fn place_panels(
    dialog: Rect,
    bounds: Option<Rect>,
    left_w: i32,
    right_w: i32,
    gap: i32,
) -> Panels {
    let mut y = dialog.y;
    let mut h = dialog.h;
    if let Some(b) = bounds {
        h = h.min(b.h);
        y = y.min(b.bottom() - h).max(b.y);
    }
    let panel = |x: i32, w: i32| Rect { x, y, w, h };

    let split_left_x = dialog.x - gap - left_w;
    let split_right_x = dialog.right() + gap;
    let Some(b) = bounds else {
        return Panels {
            left: panel(split_left_x, left_w),
            right: panel(split_right_x, right_w),
            arrangement: Arrangement::Split,
        };
    };
    let left_fits = split_left_x >= b.x;
    let right_fits = split_right_x + right_w <= b.right();
    if left_fits && right_fits {
        return Panels {
            left: panel(split_left_x, left_w),
            right: panel(split_right_x, right_w),
            arrangement: Arrangement::Split,
        };
    }
    let both_right_x = dialog.right() + gap;
    if both_right_x + left_w + gap + right_w <= b.right() {
        return Panels {
            left: panel(both_right_x, left_w),
            right: panel(both_right_x + left_w + gap, right_w),
            arrangement: Arrangement::PackedRight,
        };
    }
    let both_left_right_edge = dialog.x - gap;
    let packed_left_x = both_left_right_edge - right_w - gap - left_w;
    if packed_left_x >= b.x {
        return Panels {
            left: panel(packed_left_x, left_w),
            right: panel(packed_left_x + left_w + gap, right_w),
            arrangement: Arrangement::PackedLeft,
        };
    }
    let right_x = b.right() - right_w;
    Panels {
        left: panel(right_x - gap - left_w, left_w),
        right: panel(right_x, right_w),
        arrangement: Arrangement::Overlap,
    }
}

/// Where a rail's popup menu goes: flush against the rail on the side that
/// faces the dialog (or the side away from it, for a message that must not
/// cover the dialog's buttons), its top level with the clicked icon
/// (`anchor_y` below the rail's top), moved up to keep its bottom within the
/// rail and then kept on the monitor. When that side has no room it opens on
/// the other side.
pub fn place_menu(
    rail: Rect,
    dialog: Rect,
    bounds: Option<Rect>,
    anchor_y: i32,
    size: (i32, i32),
    face_dialog: bool,
) -> Rect {
    let (w, h) = size;
    let rail_centre = rail.x + rail.w / 2;
    let dialog_centre = dialog.x + dialog.w / 2;
    let toward_right = (rail_centre <= dialog_centre) == face_dialog;
    let right_x = rail.right();
    let left_x = rail.x - w;
    let mut x = if toward_right { right_x } else { left_x };
    let mut y = rail.y + anchor_y.clamp(0, rail.h.max(0));
    y = y.min(rail.bottom() - h).max(rail.y);
    if let Some(b) = bounds {
        let fits = |x: i32| x >= b.x && x + w <= b.right();
        if !fits(x) {
            let other = if toward_right { left_x } else { right_x };
            x = if fits(other) {
                other
            } else {
                x.min(b.right() - w).max(b.x)
            };
        }
        y = y.min(b.bottom() - h).max(b.y);
    }
    Rect { x, y, w, h }
}

/// Where a rail icon's tooltip goes: `gap` beside the rail on the side
/// that faces the dialog, so it never sits under the pointer on the rail,
/// centred on the icon (`centre_y`, absolute) and kept on the monitor. When
/// that side has no room it opens on the other side.
pub fn place_tip(
    rail: Rect,
    dialog: Rect,
    bounds: Option<Rect>,
    centre_y: i32,
    size: (i32, i32),
    gap: i32,
) -> Rect {
    let (w, h) = size;
    let toward_right = rail.x + rail.w / 2 <= dialog.x + dialog.w / 2;
    let right_x = rail.right() + gap;
    let left_x = rail.x - gap - w;
    let mut x = if toward_right { right_x } else { left_x };
    let mut y = centre_y - h / 2;
    if let Some(b) = bounds {
        let fits = |x: i32| x >= b.x && x + w <= b.right();
        if !fits(x) {
            let other = if toward_right { left_x } else { right_x };
            x = if fits(other) {
                other
            } else {
                x.min(b.right() - w).max(b.x)
            };
        }
        y = y.min(b.bottom() - h).max(b.y);
    }
    Rect { x, y, w, h }
}

/// The preview panel's place under `dialog`: same left edge and width,
/// `gap` below its bottom edge, at most `max_h` tall and cut short at the
/// monitor's bottom. `None` when less than `min_h` fits there.
pub fn place_below(
    dialog: Rect,
    bounds: Option<Rect>,
    gap: i32,
    max_h: i32,
    min_h: i32,
) -> Option<Rect> {
    let y = dialog.bottom() + gap;
    let room = bounds.map_or(max_h, |b| b.bottom() - y);
    let h = max_h.min(room);
    (h >= min_h && dialog.w > 0).then_some(Rect {
        x: dialog.x,
        y,
        w: dialog.w,
        h,
    })
}

/// The message bar's place over `dialog`: same left edge and width, `gap` above its top edge and
/// `h` tall. `None` when that would leave the monitor's top.
pub fn place_above(dialog: Rect, bounds: Option<Rect>, gap: i32, h: i32) -> Option<Rect> {
    let y = dialog.y - gap - h;
    let fits = bounds.is_none_or(|b| y >= b.y);
    (fits && dialog.w > 0 && h > 0).then_some(Rect {
        x: dialog.x,
        y,
        w: dialog.w,
        h,
    })
}

/// A window of `size` centred in `area`, kept inside it when it is larger.
pub fn centre_in(area: Rect, (w, h): (i32, i32)) -> Rect {
    let w = w.min(area.w);
    let h = h.min(area.h);
    Rect {
        x: area.x + (area.w - w) / 2,
        y: area.y + (area.h - h) / 2,
        w,
        h,
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

    #[test]
    fn each_panel_stands_on_its_own_side_of_a_dialog_with_room() {
        let p = place_panels(dialog_at(1611), Some(MONITOR), 200, 230, 6);
        assert_eq!(p.arrangement, Arrangement::Split);
        assert_eq!(p.left.x, 1611 - 6 - 200);
        assert_eq!(p.right.x, 1611 + 875 + 6);
        assert_eq!((p.left.y, p.left.h), (289, 600));
        assert_eq!((p.right.y, p.right.h), (289, 600));
    }

    #[test]
    fn with_no_room_on_the_left_both_panels_pack_to_the_right_in_order() {
        let p = place_panels(dialog_at(100), Some(MONITOR), 200, 230, 6);
        assert_eq!(p.arrangement, Arrangement::PackedRight);
        assert_eq!(p.left.x, 100 + 875 + 6);
        assert_eq!(p.right.x, p.left.right() + 6);
        assert!(p.right.right() <= MONITOR.right());
    }

    #[test]
    fn with_no_room_on_the_right_both_panels_pack_to_the_left_in_order() {
        let p = place_panels(dialog_at(3200), Some(MONITOR), 200, 230, 6);
        assert_eq!(p.arrangement, Arrangement::PackedLeft);
        assert_eq!(p.right.right() + 6, 3200);
        assert_eq!(p.right.x, p.left.right() + 6);
        assert!(p.left.x >= MONITOR.x);
    }

    #[test]
    fn with_room_on_neither_side_the_panels_overlap_against_the_right_edge() {
        let narrow = Rect {
            x: 0,
            y: 0,
            w: 1000,
            h: 800,
        };
        let dialog = Rect {
            x: 50,
            y: 10,
            w: 900,
            h: 600,
        };
        let p = place_panels(dialog, Some(narrow), 200, 230, 6);
        assert_eq!(p.arrangement, Arrangement::Overlap);
        assert_eq!(p.right.right(), narrow.right());
        assert_eq!(p.left.right() + 6, p.right.x);
    }

    #[test]
    fn panel_heights_follow_the_dialog_and_stay_on_the_monitor() {
        let tall = Rect {
            x: 1000,
            y: -50,
            w: 800,
            h: 2000,
        };
        let p = place_panels(tall, Some(MONITOR), 200, 230, 6);
        assert_eq!((p.left.h, p.right.h), (MONITOR.h, MONITOR.h));
        assert_eq!((p.left.y, p.right.y), (MONITOR.y, MONITOR.y));
    }

    #[test]
    fn without_monitor_bounds_the_panels_just_stand_on_their_sides() {
        let p = place_panels(dialog_at(3200), None, 200, 230, 6);
        assert_eq!(p.arrangement, Arrangement::Split);
        assert_eq!(p.left.x, 3200 - 6 - 200);
        assert_eq!(p.right.x, 3200 + 875 + 6);
    }

    fn rail(x: i32) -> Rect {
        Rect {
            x,
            y: 200,
            w: 52,
            h: 600,
        }
    }

    fn screen() -> Option<Rect> {
        Some(Rect {
            x: 0,
            y: 0,
            w: 4096,
            h: 1152,
        })
    }

    #[test]
    fn a_left_rails_menu_opens_flush_to_its_right_over_the_dialog_at_the_icon() {
        let dialog = dialog_at(1000);
        let m = place_menu(rail(940), dialog, screen(), 96, (280, 300), true);
        assert_eq!((m.x, m.y, m.w, m.h), (992, 296, 280, 300));
    }

    #[test]
    fn a_right_rails_menu_opens_flush_to_its_left() {
        let dialog = dialog_at(1000);
        let r = rail(dialog.right() + 6);
        let m = place_menu(r, dialog, screen(), 0, (280, 300), true);
        assert_eq!((m.x, m.y), (r.x - 280, 200));
    }

    #[test]
    fn a_menu_near_the_rails_foot_is_lifted_to_end_with_the_rail() {
        let m = place_menu(rail(940), dialog_at(1000), screen(), 560, (280, 300), true);
        assert_eq!(m.bottom(), 800);
    }

    #[test]
    fn a_menu_without_room_on_the_facing_side_opens_on_the_other_side() {
        // The rail stands against the right edge and the dialog lies to its
        // right only notionally: no room for the menu there.
        let r = Rect {
            x: 4096 - 52,
            y: 200,
            w: 52,
            h: 600,
        };
        let dialog = Rect {
            x: 4096,
            y: 200,
            w: 10,
            h: 600,
        };
        let m = place_menu(r, dialog, screen(), 0, (280, 300), true);
        assert_eq!(m.x, r.x - 280);
    }

    #[test]
    fn a_message_opens_on_the_rails_outer_side_so_the_dialog_stays_clear() {
        let dialog = dialog_at(1000);
        let r = rail(dialog.right() + 6);
        let m = place_menu(r, dialog, screen(), 100_000, (300, 64), false);
        assert_eq!(m.x, r.right());
        assert_eq!(m.bottom(), r.bottom());
    }

    #[test]
    fn a_menu_stays_on_the_monitor_even_when_the_rail_does_not() {
        let r = Rect {
            x: 940,
            y: 1000,
            w: 52,
            h: 600,
        };
        let m = place_menu(r, dialog_at(1000), screen(), 0, (280, 300), true);
        assert_eq!(m.bottom(), 1152);
    }

    #[test]
    fn a_tip_opens_a_gap_beside_its_rail_over_the_dialog_centred_on_the_icon() {
        let dialog = dialog_at(1000);
        let left = rail(dialog.x - 6 - 52);
        let t = place_tip(left, dialog, screen(), 400, (200, 30), 4);
        assert_eq!((t.x, t.y), (left.right() + 4, 385));
        let right = rail(dialog.right() + 6);
        let t = place_tip(right, dialog, screen(), 400, (200, 30), 4);
        assert_eq!(t.right(), right.x - 4);
    }

    #[test]
    fn a_tip_with_no_room_on_the_dialog_side_opens_on_the_other_side() {
        let r = Rect {
            x: 0,
            y: 200,
            w: 52,
            h: 600,
        };
        // The dialog lies to the left of a rail at the monitor's left edge.
        let dialog = Rect {
            x: -900,
            y: 200,
            w: 890,
            h: 600,
        };
        let t = place_tip(r, dialog, screen(), 300, (200, 30), 4);
        assert_eq!(t.x, r.right() + 4);
    }

    #[test]
    fn a_tip_near_the_monitors_foot_stays_on_the_monitor() {
        let dialog = dialog_at(1000);
        let r = rail(dialog.right() + 6);
        let t = place_tip(r, dialog, screen(), 1150, (200, 60), 4);
        assert_eq!(t.bottom(), 1152);
    }

    #[test]
    fn a_rect_contains_its_top_left_edge_but_not_its_far_edges() {
        let r = Rect {
            x: 10,
            y: 20,
            w: 52,
            h: 600,
        };
        assert!(r.contains((10, 20)));
        assert!(r.contains((61, 619)));
        assert!(!r.contains((62, 300)));
        assert!(!r.contains((30, 620)));
        assert!(!r.contains((9, 300)));
    }

    #[test]
    fn a_window_is_centred_in_an_area_and_shrunk_to_fit_it() {
        let area = Rect {
            x: 100,
            y: 50,
            w: 1000,
            h: 800,
        };
        assert_eq!(
            centre_in(area, (780, 560)),
            Rect {
                x: 210,
                y: 170,
                w: 780,
                h: 560
            }
        );
        assert_eq!(
            centre_in(area, (2000, 100)),
            Rect {
                x: 100,
                y: 400,
                w: 1000,
                h: 100
            }
        );
    }

    #[test]
    fn the_message_bar_sits_over_the_dialog_as_wide_as_it_and_is_left_out_without_room() {
        let dialog = Rect {
            x: 400,
            y: 300,
            w: 800,
            h: 500,
        };
        let bar = place_above(dialog, Some(MONITOR), 3, 47).expect("room above");
        assert_eq!((bar.x, bar.w, bar.h), (400, 800, 47));
        assert_eq!(bar.bottom() + 3, dialog.y, "a gap above the dialog");
        let high = Rect { y: 40, ..dialog };
        assert_eq!(place_above(high, Some(MONITOR), 3, 47), None);
        assert!(
            place_above(high, None, 3, 47).is_some(),
            "no monitor known: placed"
        );
    }

    #[test]
    fn the_preview_sits_under_the_dialog_as_wide_as_it_and_stops_at_the_monitor_bottom() {
        let dialog = Rect {
            x: 100,
            y: 100,
            w: 800,
            h: 500,
        };
        let below = place_below(dialog, Some(MONITOR), 3, 280, 140).expect("room");
        assert_eq!(
            below,
            Rect {
                x: 100,
                y: 603,
                w: 800,
                h: 280
            }
        );
        let low = Rect { y: 500, ..dialog };
        let cut = place_below(low, Some(MONITOR), 3, 280, 140).expect("some room");
        assert_eq!(cut.bottom(), MONITOR.bottom());
        assert!(cut.h < 280);
        let lower = Rect {
            y: MONITOR.bottom() - 600,
            ..dialog
        };
        assert_eq!(place_below(lower, Some(MONITOR), 3, 280, 140), None);
        assert_eq!(
            place_below(dialog, None, 3, 280, 140).map(|r| r.h),
            Some(280)
        );
    }
}
