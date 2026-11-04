//! The preview panel under the dialog: a picture of the selected image, or a
//! still from the selected video, with a caption. It never takes the
//! keyboard; a click on the caption copies the file's path.
//!
//! Loading happens off the UI thread. A picture is decoded and shrunk to the
//! panel's size there, so a huge photo costs the panel no more than a small
//! one; a load the selection has moved on from is dropped, and a video's
//! `ffmpeg` is stopped.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant, UNIX_EPOCH};

use gpui::{
    Context, Entity, Image, ImageFormat, IntoElement, Render, Subscription, Window, div, img,
    prelude::*, px, rgb,
};
use ofolder_core::messages::format_local;
use ofolder_core::preview::{Kind, PreviewSettings, caption, decodable, fit};

use crate::colors::colors;
use crate::view::{Shared, local_offset};

/// The largest a picture is kept, in pixels: about twice the panel, for
/// sharp drawing on a scaled monitor.
const KEEP_SIZE: (u32, u32) = (2048, 600);
/// How long `ffmpeg` may take for a still before it is stopped.
const STILL_LIMIT: Duration = Duration::from_secs(3);

/// What the panel shows for one selected file.
#[derive(Clone)]
pub struct Content {
    pub path: PathBuf,
    pub image: Option<Arc<Image>>,
    pub caption: String,
    /// Shown instead of a picture: too large, unreadable, no still.
    pub note: Option<String>,
}

/// A picture ready to draw, with its size in pixels when known.
type Picture = (Arc<Image>, Option<(u32, u32)>);

/// Whether the load numbered `mine` is still the one wanted.
fn current(generation: &AtomicU64, mine: u64) -> bool {
    generation.load(Ordering::Relaxed) == mine
}

/// Loads what the panel shows for `path`. `None` when the selection moved
/// on before it finished.
pub fn load(
    path: &Path,
    kind: Kind,
    settings: PreviewSettings,
    generation: &AtomicU64,
    mine: u64,
) -> Option<Content> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let content = |picture: Result<Picture, String>, bytes: u64, modified: Option<String>| {
        let (image, pixels, note) = match picture {
            Ok((image, pixels)) => (Some(image), pixels, None),
            Err(why) => (None, None, Some(why)),
        };
        Content {
            path: path.to_path_buf(),
            image,
            caption: caption(&name, kind, pixels, bytes, modified.as_deref()),
            note,
        }
    };
    // Followed through a link, but only to a regular file: never a pipe or
    // a device, which could block or never end.
    let meta = match fs::metadata(path) {
        Ok(meta) if meta.is_file() => meta,
        Ok(_) => return Some(content(Err("Not a regular file".to_owned()), 0, None)),
        Err(_) => return Some(content(Err("Cannot be read".to_owned()), 0, None)),
    };
    let bytes = meta.len();
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .map(|at| {
            let mut text = format_local(at, local_offset(at));
            // Minutes are enough under a picture.
            text.truncate(16);
            text
        });
    if bytes > settings.max_bytes() {
        let why = format!("Larger than {} MB, so not previewed", settings.max_mb);
        return Some(content(Err(why), bytes, modified));
    }
    let picture = match kind {
        Kind::Image if is_svg(path) => fs::read(path)
            .map(|data| (Arc::new(Image::from_bytes(ImageFormat::Svg, data)), None))
            .map_err(|_| "Cannot be read".to_owned()),
        Kind::Image => fs::read(path)
            .map_err(|_| "Cannot be read".to_owned())
            .and_then(|data| shrink(&data)),
        Kind::Video if !settings.video_thumbnails => Err("Video stills are turned off".to_owned()),
        Kind::Video => video_still(path, generation, mine)
            .and_then(|png| shrink(&png))
            // The still's size is not the video's; leave it out.
            .map(|(image, _)| (image, None)),
    };
    current(generation, mine).then(|| content(picture, bytes, modified))
}

fn is_svg(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
}

/// What a decoder may allocate and how large a side it may accept. The `image` crate's own default
/// is 512 MiB with no side limit; stating it here keeps the bound visible and lets it be tested.
fn decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(30_000);
    limits.max_image_height = Some(30_000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    limits
}

/// Decodes a picture, shrinks it to [`KEEP_SIZE`] and hands it over as PNG.
fn shrink(data: &[u8]) -> Result<Picture, String> {
    let reader = image::ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|_| "Not a picture this can show".to_owned())?;
    let (w, h) = reader
        .into_dimensions()
        .map_err(|_| "Not a picture this can show".to_owned())?;
    if !decodable(w, h) {
        return Err(format!("{w} × {h} is too large to preview"));
    }
    let mut reader = image::ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|_| "Not a picture this can show".to_owned())?;
    reader.limits(decode_limits());
    let decoded = reader
        .decode()
        .map_err(|_| "The picture is damaged or cut short".to_owned())?;
    let (tw, th) = fit((w, h), KEEP_SIZE);
    let small = if (tw, th) == (w, h) {
        decoded
    } else {
        decoded.thumbnail(tw, th)
    };
    let mut png = Vec::new();
    small
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|_| "The picture could not be prepared".to_owned())?;
    Ok((
        Arc::new(Image::from_bytes(ImageFormat::Png, png)),
        Some((w, h)),
    ))
}

/// `ffmpeg` on the search path, if it is installed.
fn ffmpeg() -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|dir| Path::new(dir).join("ffmpeg"))
        .find(|p| p.is_file())
}

/// A still from the video at `path` as PNG bytes, made by `ffmpeg` into a
/// private temporary file that is removed afterwards.
fn video_still(path: &Path, generation: &AtomicU64, mine: u64) -> Result<Vec<u8>, String> {
    let ffmpeg = ffmpeg().ok_or("No thumbnail tool (ffmpeg) was found")?;
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("ofolder");
    fs::create_dir_all(&dir).map_err(|_| "No place for a video still".to_owned())?;
    let out = dir.join(format!("still-{}-{mine}.png", std::process::id()));
    // One second in, then from the start for a clip shorter than that.
    let mut result = Err("No still could be made from this video".to_owned());
    for seek in ["1", "0"] {
        match run_ffmpeg(&ffmpeg, path, seek, &out, generation, mine) {
            Ok(()) => match fs::read(&out) {
                Ok(png) if !png.is_empty() => {
                    result = Ok(png);
                    break;
                }
                _ => {}
            },
            Err(stopped) => {
                result = Err(stopped);
                break;
            }
        }
    }
    fs::remove_file(&out).ok();
    result
}

/// Runs `ffmpeg` once. Stops it when it runs over [`STILL_LIMIT`] or the
/// selection moves on.
fn run_ffmpeg(
    ffmpeg: &Path,
    video: &Path,
    seek: &str,
    out: &Path,
    generation: &AtomicU64,
    mine: u64,
) -> Result<(), String> {
    // `file:` keeps a name that looks like an option or a protocol a file.
    let mut input = std::ffi::OsString::from("file:");
    input.push(video.as_os_str());
    let mut child = Command::new(ffmpeg)
        .args(["-loglevel", "error", "-y", "-ss", seek, "-i"])
        .arg(&input)
        .args(["-frames:v", "1", "-vf", "scale=640:-1"])
        .arg(out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "ffmpeg could not be started".to_owned())?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) if !current(generation, mine) || start.elapsed() > STILL_LIMIT => {
                child.kill().ok();
                child.wait().ok();
                return Err(if current(generation, mine) {
                    "The video took too long to read".to_owned()
                } else {
                    "Moved on".to_owned()
                });
            }
            Ok(None) => sleep(Duration::from_millis(20)),
            Err(_) => return Err("ffmpeg could not be watched".to_owned()),
        }
    }
}

/// The panel's window.
pub struct PreviewWindow {
    shared: Entity<Shared>,
    _subscriptions: Vec<Subscription>,
}

impl PreviewWindow {
    pub fn new(shared: Entity<Shared>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&shared, |_, _, cx| cx.notify());
        Self {
            shared,
            _subscriptions: vec![observe],
        }
    }
}

impl Render for PreviewWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.shared.read(cx).preview_content.clone();
        let frame = div()
            .size_full()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .overflow_hidden()
            .bg(rgb(colors().bg))
            .border_1()
            .border_color(rgb(colors().field_border))
            .text_color(rgb(colors().text))
            .text_size(px(12.));
        let Some(content) = content else {
            return frame.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(colors().muted))
                    .child("Loading…"),
            );
        };
        let body = match (&content.image, &content.note) {
            (Some(image), _) => div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .justify_center()
                .child(img(image.clone()).size_full()),
            (None, note) => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(colors().muted))
                .child(note.clone().unwrap_or_default()),
        };
        frame.child(body).child(
            div()
                .id("preview-caption")
                .flex_none()
                .px_1()
                .truncate()
                .text_color(rgb(colors().muted))
                .cursor_pointer()
                .hover(|s| s.text_color(rgb(colors().text)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.shared.update(cx, |s, cx| s.copy_preview_path(cx));
                }))
                .child(content.caption),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ofolder-preview-{label}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("dir");
        dir
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let picture = image::RgbImage::from_pixel(w, h, image::Rgb([40, 120, 200]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(picture)
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("encode");
        out
    }

    #[test]
    fn a_picture_is_shrunk_to_the_panel_and_keeps_its_own_size_in_the_caption() {
        let dir = scratch("shrink");
        let path = dir.join("wide.png");
        fs::write(&path, png(4000, 1000)).expect("write");
        let generation = AtomicU64::new(1);
        let content = load(
            &path,
            Kind::Image,
            PreviewSettings::default(),
            &generation,
            1,
        )
        .expect("loaded");
        assert!(content.image.is_some() && content.note.is_none());
        assert!(
            content.caption.contains("4000 × 1000"),
            "{}",
            content.caption
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_damaged_or_too_large_file_is_described_not_drawn() {
        let dir = scratch("bad");
        let cut = dir.join("cut.png");
        let mut data = png(300, 300);
        data.truncate(data.len() / 2);
        fs::write(&cut, data).expect("write");
        let generation = AtomicU64::new(1);
        let settings = PreviewSettings::default();
        let content = load(&cut, Kind::Image, settings, &generation, 1).expect("loaded");
        assert!(content.image.is_none() && content.note.is_some());
        let big = dir.join("big.png");
        fs::write(&big, vec![0u8; 2 * 1024 * 1024]).expect("write");
        let small = PreviewSettings {
            max_mb: 1,
            ..settings
        };
        let content = load(&big, Kind::Image, small, &generation, 1).expect("loaded");
        assert!(content.note.expect("note").contains("Larger than 1 MB"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_link_to_something_that_is_not_a_regular_file_is_not_followed_into() {
        let dir = scratch("link");
        let link = dir.join("dev.png");
        std::os::unix::fs::symlink("/dev/zero", &link).expect("link");
        let generation = AtomicU64::new(1);
        let content = load(
            &link,
            Kind::Image,
            PreviewSettings::default(),
            &generation,
            1,
        )
        .expect("loaded");
        assert_eq!(content.note.as_deref(), Some("Not a regular file"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_load_the_selection_moved_on_from_is_dropped() {
        let dir = scratch("stale");
        let path = dir.join("a.png");
        fs::write(&path, png(10, 10)).expect("write");
        let generation = AtomicU64::new(2);
        assert!(
            load(
                &path,
                Kind::Image,
                PreviewSettings::default(),
                &generation,
                1
            )
            .is_none()
        );
        fs::remove_dir_all(&dir).ok();
    }

    /// A PNG made by `image`, with its header's size rewritten (and the checksum fixed), as a file
    /// that claims to be far larger than its data would be.
    fn png_claiming(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(1, 1, image::Rgb([1, 2, 3])))
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("encodes");
        // The IHDR chunk's data starts at byte 16: width, height; its CRC covers type and data.
        out[16..20].copy_from_slice(&w.to_be_bytes());
        out[20..24].copy_from_slice(&h.to_be_bytes());
        let mut crc: u32 = 0xffff_ffff;
        for byte in &out[12..29] {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        out[29..33].copy_from_slice(&(!crc).to_be_bytes());
        out
    }

    #[test]
    fn a_picture_that_claims_to_be_enormous_is_refused_before_anything_is_decoded() {
        let err = shrink(&png_claiming(60_000, 60_000)).expect_err("refused");
        assert!(err.contains("too large"), "{err}");
    }

    #[test]
    fn the_decoder_limits_are_stated_and_stricter_than_unlimited() {
        let limits = decode_limits();
        assert_eq!(limits.max_alloc, Some(512 * 1024 * 1024));
        assert_eq!(limits.max_image_width, Some(30_000));
        assert_eq!(limits.max_image_height, Some(30_000));
    }
}
