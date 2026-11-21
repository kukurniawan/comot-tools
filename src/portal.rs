//! xdg-desktop-portal access: screenshots and the native "Save as" dialog.
//!
//! The portal is the supported way to capture the screen on Wayland (GNOME,
//! KDE, wlroots…) and also works on X11 desktops that run the portal service.

use std::future::Future;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use ashpd::desktop::file_chooser::{FileFilter, SelectedFiles};
use ashpd::desktop::screenshot::Screenshot;
use ashpd::desktop::ResponseError;
use image::RgbaImage;

fn block_on<F: Future>(fut: F) -> Result<F::Output> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start async runtime")?;
    Ok(rt.block_on(fut))
}

pub struct Shot {
    pub image: RgbaImage,
    /// The desktop's own screenshot UI was used, so the user already picked
    /// the area there and our selection overlay should be skipped.
    pub interactive: bool,
}

async fn screenshot_path(interactive: bool) -> ashpd::Result<Option<PathBuf>> {
    let response = Screenshot::request()
        .interactive(interactive)
        .modal(false)
        .send()
        .await?
        .response()?;
    Ok(response.uri().to_file_path().ok())
}

/// Take a screenshot of the whole desktop. `Ok(None)` = the user cancelled.
///
/// A silent (non-interactive) capture is tried first. GNOME refuses it unless
/// the calling app was granted the screenshot permission (the "app" is derived
/// from the systemd scope, e.g. the terminal it was started from); in that case
/// fall back to the interactive portal, which shows GNOME's screenshot UI.
pub fn capture_screen() -> Result<Option<Shot>> {
    let (result, interactive) = block_on(async {
        match screenshot_path(false).await {
            Err(ashpd::Error::Response(ResponseError::Other)) => (screenshot_path(true).await, true),
            other => (other, false),
        }
    })?;

    let path = match result {
        Ok(Some(path)) => path,
        Ok(None) => return Err(anyhow!("portal returned a non-file URI")),
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => return Ok(None),
        Err(e) => return Err(e).context("screenshot request to xdg-desktop-portal failed"),
    };
    let image = image::open(&path)
        .with_context(|| format!("cannot load screenshot {}", path.display()))?
        .to_rgba8();

    // The portal writes a file (GNOME: into ~/Pictures/Screenshots). It is only
    // our raw capture, so remove it; the user saves the snip explicitly.
    let _ = std::fs::remove_file(&path);
    Ok(Some(Shot { image, interactive }))
}

/// Show the native save dialog. `Ok(None)` means the user cancelled.
pub fn save_file_dialog(suggested_name: &str) -> Result<Option<PathBuf>> {
    let result = block_on(async {
        SelectedFiles::save_file()
            .title("Save snip")
            .accept_label("Save")
            .modal(true)
            .current_name(suggested_name)
            .filter(FileFilter::new("PNG image").mimetype("image/png").glob("*.png"))
            .filter(FileFilter::new("JPEG image").mimetype("image/jpeg").glob("*.jpg"))
            .send()
            .await?
            .response()
    })?;

    match result {
        Ok(files) => Ok(files.uris().first().and_then(|u| u.to_file_path().ok())),
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => Ok(None),
        Err(e) => Err(e).context("save dialog via xdg-desktop-portal failed"),
    }
}
