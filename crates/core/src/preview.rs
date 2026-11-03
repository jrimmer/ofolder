//! What the preview panel under a dialog shows, and when.
//!
//! The panel appears only while the dialog's selected item is an image or a
//! video, told by its extension; anything else, or no selection, hides it.
//! A selection has to hold still for a moment before the panel follows it,
//! so scrolling through a folder with the arrow keys does not flicker.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use toml::{Table, Value};

/// How long a selection must hold before the panel follows it.
pub const SETTLE: Duration = Duration::from_millis(150);
/// The default largest file previewed, in MiB.
pub const DEFAULT_MAX_MB: u64 = 64;
/// The sizes offered in the settings window, in MiB.
pub const MAX_MB_CHOICES: [u64; 5] = [16, 32, 64, 128, 256];
/// The largest picture decoded, in pixels across times down; a bigger one
/// is described, not drawn.
pub const MAX_PIXELS: u64 = 120_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Image => "Image",
            Kind::Video => "Video",
        }
    }
}

const IMAGES: [&str; 9] = [
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "svg",
];
const VIDEOS: [&str; 6] = ["mp4", "mkv", "webm", "mov", "avi", "m4v"];

/// An image or a video by its extension, ignoring case; anything else, or
/// no extension, is neither.
pub fn classify(path: &Path) -> Option<Kind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if IMAGES.contains(&ext.as_str()) {
        Some(Kind::Image)
    } else if VIDEOS.contains(&ext.as_str()) {
        Some(Kind::Video)
    } else {
        None
    }
}

/// The preview settings, under `[preview]` in the settings file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreviewSettings {
    /// Show the panel when an image or video is selected.
    pub enabled: bool,
    /// Make a still from a video with `ffmpeg` when it is installed.
    pub video_thumbnails: bool,
    /// Files larger than this many MiB are described, not previewed.
    pub max_mb: u64,
}

impl Default for PreviewSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            video_thumbnails: true,
            max_mb: DEFAULT_MAX_MB,
        }
    }
}

impl PreviewSettings {
    pub fn max_bytes(&self) -> u64 {
        self.max_mb.saturating_mul(1024 * 1024)
    }

    /// Reads `[preview]` leniently: a bad value is reported and the default
    /// kept, an unknown key is reported and ignored.
    pub fn from_table(table: &Table, warnings: &mut Vec<String>) -> Self {
        let mut out = Self::default();
        for (key, value) in table {
            match (key.as_str(), value) {
                ("enabled", Value::Boolean(on)) => out.enabled = *on,
                ("video_thumbnails", Value::Boolean(on)) => out.video_thumbnails = *on,
                ("max_mb", Value::Integer(n)) if (1..=4096).contains(n) => {
                    out.max_mb = u64::try_from(*n).unwrap_or(DEFAULT_MAX_MB);
                }
                ("max_mb", Value::Integer(_)) => warnings.push(
                    "setting `preview.max_mb` must be from 1 to 4096 and was ignored".to_owned(),
                ),
                ("enabled" | "video_thumbnails", _) => warnings.push(format!(
                    "setting `preview.{key}` must be true or false and was ignored"
                )),
                ("max_mb", _) => warnings.push(
                    "setting `preview.max_mb` must be a whole number and was ignored".to_owned(),
                ),
                _ => warnings.push(format!(
                    "setting `preview.{key}` is not known and was ignored"
                )),
            }
        }
        out
    }
}

/// `3.4 MB`, `812 KB`, `17 bytes`.
pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1000.0;
    // Precision loss is fine: this is a figure for people to read.
    #[allow(clippy::cast_precision_loss)]
    let b = bytes as f64;
    if bytes < 1000 {
        format!("{bytes} bytes")
    } else if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / KB / KB)
    } else {
        format!("{:.1} GB", b / KB / KB / KB)
    }
}

/// The caption under a preview: name, kind, size in pixels when known,
/// file size and when it was changed, joined with dots.
pub fn caption(
    name: &str,
    kind: Kind,
    pixels: Option<(u32, u32)>,
    bytes: u64,
    modified: Option<&str>,
) -> String {
    let mut parts = vec![name.to_owned(), kind.name().to_owned()];
    if let Some((w, h)) = pixels {
        parts.push(format!("{w} × {h}"));
    }
    parts.push(human_size(bytes));
    if let Some(when) = modified {
        parts.push(when.to_owned());
    }
    parts.join("  ·  ")
}

/// Follows the dialog's selection, letting a change through only once it
/// has held for [`SETTLE`].
#[derive(Debug, Default)]
pub struct Settle {
    /// What the panel shows now.
    shown: Option<PathBuf>,
    /// A different selection seen, and when it was first seen.
    pending: Option<(Option<PathBuf>, Instant)>,
}

impl Settle {
    /// The item the panel shows.
    pub fn shown(&self) -> Option<&Path> {
        self.shown.as_deref()
    }

    /// Notes what is selected now (only an image or a video counts; give
    /// `None` for anything else). Returns the new item to show, or `Some(None)`
    /// to hide, when that changed; `None` when nothing changes yet.
    pub fn observe(
        &mut self,
        now_selected: Option<PathBuf>,
        now: Instant,
    ) -> Option<Option<PathBuf>> {
        if now_selected == self.shown {
            self.pending = None;
            return None;
        }
        match &self.pending {
            Some((candidate, since))
                if *candidate == now_selected
                    && now.saturating_duration_since(*since) >= SETTLE =>
            {
                self.pending = None;
                self.shown.clone_from(&now_selected);
                Some(now_selected)
            }
            Some((candidate, _)) if *candidate == now_selected => None,
            _ => {
                self.pending = Some((now_selected, now));
                None
            }
        }
    }

    /// Forgets what is shown, so the next selection is shown afresh.
    pub fn reset(&mut self) {
        self.shown = None;
        self.pending = None;
    }
}

/// Whether a picture of `w` by `h` pixels is small enough to decode.
pub fn decodable(w: u32, h: u32) -> bool {
    u64::from(w).saturating_mul(u64::from(h)) <= MAX_PIXELS && w > 0 && h > 0
}

/// The size to draw a `w` by `h` picture at to fit in `box_w` by `box_h`,
/// keeping its shape and never enlarging it.
pub fn fit((w, h): (u32, u32), (box_w, box_h): (u32, u32)) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (0, 0);
    }
    let scale = (f64::from(box_w) / f64::from(w))
        .min(f64::from(box_h) / f64::from(h))
        .min(1.0);
    // Rounded and kept at least one pixel; the values are within u32.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let out = (
        ((f64::from(w) * scale).round() as u32).max(1),
        ((f64::from(h) * scale).round() as u32).max(1),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_and_videos_are_told_by_extension_ignoring_case() {
        assert_eq!(classify(Path::new("/a/b.PNG")), Some(Kind::Image));
        assert_eq!(classify(Path::new("/a/b.jpeg")), Some(Kind::Image));
        assert_eq!(classify(Path::new("/a/logo.svg")), Some(Kind::Image));
        assert_eq!(classify(Path::new("/a/clip.MkV")), Some(Kind::Video));
        assert_eq!(classify(Path::new("/a/notes.txt")), None);
        assert_eq!(classify(Path::new("/a/png")), None);
        assert_eq!(classify(Path::new("/a/.png")), None);
        assert_eq!(classify(Path::new("/a/archive.tar.gz")), None);
    }

    #[test]
    fn preview_settings_default_on_and_read_leniently() {
        let table: Table = "enabled = false\nmax_mb = 5000\nvideo_thumbnails = \"yes\"\ncolour = 1"
            .parse()
            .expect("toml");
        let mut warnings = Vec::new();
        let s = PreviewSettings::from_table(&table, &mut warnings);
        assert!(!s.enabled);
        assert!(s.video_thumbnails);
        assert_eq!(s.max_mb, DEFAULT_MAX_MB);
        assert_eq!(warnings.len(), 3);
        assert_eq!(PreviewSettings::default().max_bytes(), 64 * 1024 * 1024);
        assert!(PreviewSettings::default().enabled);
    }

    #[test]
    fn sizes_read_the_way_people_say_them() {
        assert_eq!(human_size(17), "17 bytes");
        assert_eq!(human_size(812_000), "812 KB");
        assert_eq!(human_size(3_400_000), "3.4 MB");
        assert_eq!(human_size(2_500_000_000), "2.5 GB");
    }

    #[test]
    fn the_caption_names_the_file_its_kind_size_and_date() {
        assert_eq!(
            caption(
                "a.png",
                Kind::Image,
                Some((640, 400)),
                3_444,
                Some("2026-11-03 01:25")
            ),
            "a.png  ·  Image  ·  640 × 400  ·  3 KB  ·  2026-11-03 01:25"
        );
        assert_eq!(
            caption("c.mp4", Kind::Video, None, 15_257, None),
            "c.mp4  ·  Video  ·  15 KB"
        );
    }

    #[test]
    fn a_selection_must_hold_still_before_the_panel_follows_it() {
        let mut s = Settle::default();
        let t = Instant::now();
        let a = Some(PathBuf::from("/x/a.png"));
        let b = Some(PathBuf::from("/x/b.png"));
        assert_eq!(s.observe(a.clone(), t), None, "first sight only notes it");
        assert_eq!(s.observe(a.clone(), t + Duration::from_millis(100)), None);
        assert_eq!(
            s.observe(a.clone(), t + Duration::from_millis(200)),
            Some(a.clone())
        );
        assert_eq!(s.shown(), a.as_deref());
        // Passing over b on the way to a again changes nothing.
        assert_eq!(s.observe(b.clone(), t + Duration::from_millis(300)), None);
        assert_eq!(s.observe(a.clone(), t + Duration::from_millis(400)), None);
        assert_eq!(s.shown(), a.as_deref());
        // Settling on nothing hides it.
        assert_eq!(s.observe(None, t + Duration::from_millis(500)), None);
        assert_eq!(s.observe(None, t + Duration::from_millis(700)), Some(None));
        s.reset();
        assert_eq!(s.shown(), None);
    }

    #[test]
    fn huge_pictures_are_not_decoded_and_pictures_shrink_to_fit_but_never_grow() {
        assert!(decodable(8_000, 6_000));
        assert!(!decodable(20_000, 20_000));
        assert!(!decodable(0, 10));
        assert_eq!(fit((1600, 800), (800, 400)), (800, 400));
        assert_eq!(fit((800, 1600), (800, 400)), (200, 400));
        assert_eq!(fit((100, 50), (800, 400)), (100, 50));
        assert_eq!(fit((0, 50), (800, 400)), (0, 0));
    }
}
