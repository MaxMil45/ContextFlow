//! Custom desktop-shortcut icons, one per workspace.
//!
//! The picture the user chooses (PNG, JPEG, BMP or ICO) is converted to a
//! multi-size `.ico` and kept in `<data dir>/icons/`, so the shortcut keeps its
//! icon if the original file is moved or deleted. The file name carries a hash
//! of its contents: Explorer caches icons by path, so a new picture must get a
//! new path or the old icon keeps showing.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};
use sha2::{Digest, Sha256};

/// Largest picture accepted, before conversion.
pub const MAX_INPUT_BYTES: usize = 20 * 1024 * 1024;

/// Sizes Windows picks from for desktop, taskbar and Explorer views.
const SIZES: [u32; 5] = [16, 24, 32, 48, 256];

/// Where custom icons live: next to the database.
pub fn icon_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join("icons")
}

fn prefix(workspace_id: &str) -> String {
    format!("{workspace_id}-")
}

/// The workspace's custom icon, if one has been set.
pub fn custom_icon(dir: &Path, workspace_id: &str) -> Option<PathBuf> {
    let prefix = prefix(workspace_id);
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| {
            p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ico"))
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix))
        })
}

/// Convert any supported picture to a multi-size ICO. The picture is centred
/// on a transparent square first, so wide or tall pictures are not stretched.
pub fn to_ico(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() > MAX_INPUT_BYTES {
        bail!(
            "that picture is too large ({} MB); the limit is {} MB",
            bytes.len() / (1024 * 1024),
            MAX_INPUT_BYTES / (1024 * 1024)
        );
    }
    let format = image::guess_format(bytes)
        .context("that file isn't a picture ContextFlow can read (use PNG, JPEG, BMP or ICO)")?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Bmp | ImageFormat::Ico
    ) {
        bail!("{format:?} pictures aren't supported; use PNG, JPEG, BMP or ICO");
    }
    let img = image::load_from_memory_with_format(bytes, format)
        .context("the picture could not be read")?;
    let square = squared(img);

    let frames = SIZES
        .iter()
        .map(|&s| {
            let resized = square.resize_exact(s, s, FilterType::Lanczos3).to_rgba8();
            image::codecs::ico::IcoFrame::as_png(
                resized.as_raw(),
                s,
                s,
                image::ExtendedColorType::Rgba8,
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .context("the icon could not be built")?;
    let mut out = Cursor::new(Vec::new());
    image::codecs::ico::IcoEncoder::new(&mut out)
        .encode_images(&frames)
        .context("the icon could not be built")?;
    Ok(out.into_inner())
}

fn squared(img: DynamicImage) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    if w == h {
        return img;
    }
    let side = w.max(h);
    let mut canvas = image::RgbaImage::new(side, side);
    image::imageops::overlay(
        &mut canvas,
        &img.to_rgba8(),
        i64::from((side - w) / 2),
        i64::from((side - h) / 2),
    );
    DynamicImage::ImageRgba8(canvas)
}

/// Save `picture` as the workspace's icon, replacing any earlier one.
/// Returns the path of the new `.ico`.
pub fn set_custom_icon(dir: &Path, workspace_id: &str, picture: &[u8]) -> Result<PathBuf> {
    let ico = to_ico(picture)?;
    let hash = hex::encode(&Sha256::digest(&ico)[..4]);
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(format!("{}{hash}.ico", prefix(workspace_id)));
    // Write the new file before removing the old one, so a failure never
    // leaves the shortcut pointing at nothing.
    let tmp = path.with_extension("ico.tmp");
    std::fs::write(&tmp, &ico).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))?;
    remove_others(dir, workspace_id, Some(&path));
    Ok(path)
}

/// Go back to the default icon.
pub fn clear_custom_icon(dir: &Path, workspace_id: &str) {
    remove_others(dir, workspace_id, None);
}

fn remove_others(dir: &Path, workspace_id: &str, keep: Option<&Path>) {
    let prefix = prefix(workspace_id);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for p in entries.filter_map(|e| e.ok().map(|e| e.path())) {
        let ours = p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(&prefix));
        if ours && Some(p.as_path()) != keep {
            let _ = std::fs::remove_file(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 30, 255]));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn png_becomes_a_multi_size_ico() {
        let ico = to_ico(&png(300, 120)).unwrap();
        assert_eq!(&ico[..4], &[0, 0, 1, 0], "ICO header");
        assert_eq!(u16::from_le_bytes([ico[4], ico[5]]) as usize, SIZES.len());
        // It reads back as an image.
        let back = image::load_from_memory_with_format(&ico, ImageFormat::Ico).unwrap();
        assert_eq!(back.width(), back.height());
    }

    #[test]
    fn non_pictures_are_refused() {
        assert!(to_ico(b"not a picture at all").is_err());
    }

    #[test]
    fn setting_replaces_the_previous_icon_and_clearing_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let first = set_custom_icon(dir.path(), "ws1", &png(64, 64)).unwrap();
        let other = set_custom_icon(dir.path(), "ws2", &png(64, 64)).unwrap();
        let second = set_custom_icon(dir.path(), "ws1", &png(32, 48)).unwrap();
        assert_ne!(first, second, "a new picture gets a new path");
        assert!(!first.exists());
        assert_eq!(custom_icon(dir.path(), "ws1"), Some(second));
        clear_custom_icon(dir.path(), "ws1");
        assert_eq!(custom_icon(dir.path(), "ws1"), None);
        assert_eq!(
            custom_icon(dir.path(), "ws2"),
            Some(other),
            "other workspaces untouched"
        );
    }
}
