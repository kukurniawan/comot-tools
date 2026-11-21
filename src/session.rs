//! State that survives across the editor / selector windows.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use eframe::egui::Color32;
use image::{DynamicImage, ImageFormat, RgbaImage};

use crate::document::Document;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnipMode {
    Rectangle,
    FullScreen,
}

impl SnipMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rectangle => "Rectangle",
            Self::FullScreen => "Full screen",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Highlighter,
    Eraser,
    Crop,
}

/// What `main` should do after a window closes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Action {
    ShowEditor,
    Snip,
    #[default]
    Quit,
}

#[derive(Clone)]
pub struct Settings {
    pub mode: SnipMode,
    pub delay_secs: u32,
    pub tool: Tool,
    pub pen_color: Color32,
    pub pen_size: f32,
    pub highlight_color: Color32,
    pub highlight_size: f32,
    /// Copy every new snip to the clipboard automatically (like Windows).
    pub auto_copy: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: SnipMode::Rectangle,
            delay_secs: 0,
            tool: Tool::Pen,
            pen_color: Color32::from_rgb(232, 17, 35),
            pen_size: 4.0,
            highlight_color: Color32::from_rgb(255, 240, 0),
            highlight_size: 20.0,
            auto_copy: true,
        }
    }
}

#[derive(Default)]
pub struct Session {
    pub doc: Option<Document>,
    pub settings: Settings,
    pub status: String,
    pub next: Action,
    /// Kept alive for the whole process: on X11 the clipboard content is
    /// served by this object, so dropping it would clear the clipboard.
    clipboard: Option<arboard::Clipboard>,
}

impl Session {
    pub fn copy_to_clipboard(&mut self) -> Result<()> {
        let img = self.doc.as_ref().context("nothing to copy")?.render();
        if self.clipboard.is_none() {
            self.clipboard = Some(arboard::Clipboard::new().context("cannot open clipboard")?);
        }
        let clipboard = self.clipboard.as_mut().expect("clipboard initialised above");
        clipboard
            .set_image(arboard::ImageData {
                width: img.width() as usize,
                height: img.height() as usize,
                bytes: Cow::Borrowed(img.as_raw()),
            })
            .context("cannot put image on clipboard")?;
        Ok(())
    }
}

/// `~/Pictures/Screenshots/Snip 2026-11-21 14-03-22.png` (localised Pictures dir).
pub fn default_save_path() -> PathBuf {
    let base = dirs::picture_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Screenshots").join(format!(
        "Snip {}.png",
        chrono::Local::now().format("%Y-%m-%d %H-%M-%S")
    ))
}

/// Save as PNG or JPEG depending on the extension (PNG if none/unknown).
/// Returns the path actually written.
pub fn save_image(img: &RgbaImage, path: &Path) -> Result<PathBuf> {
    let mut path = path.to_path_buf();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let format = match ext.as_deref() {
        Some("jpg" | "jpeg") => ImageFormat::Jpeg,
        Some("png") => ImageFormat::Png,
        _ => {
            let mut s = path.into_os_string();
            s.push(".png");
            path = PathBuf::from(s);
            ImageFormat::Png
        }
    };

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }

    let written = match format {
        ImageFormat::Jpeg => DynamicImage::ImageRgba8(img.clone())
            .to_rgb8()
            .save_with_format(&path, format),
        _ => img.save_with_format(&path, format),
    };
    written.with_context(|| format!("cannot write {}", path.display()))?;
    Ok(path)
}
