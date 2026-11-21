//! xdg-desktop-portal access: screenshots and the native "Save as" dialog.
//!
//! The portal is the supported way to capture the screen on Wayland (GNOME,
//! KDE, wlroots…) and also works on X11 desktops that run the portal service.

use std::future::Future;
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};
use ashpd::desktop::file_chooser::{FileFilter, SelectedFiles};
use ashpd::desktop::screenshot::Screenshot;
use ashpd::desktop::ResponseError;
use image::RgbaImage;
use tokio::runtime::Runtime;

/// Run a portal future on the process-wide runtime.
///
/// The runtime must live as long as the process: ashpd caches one global D-Bus
/// connection, and zbus runs that connection's reader task on the runtime that
/// created it. With a throwaway runtime per call, the second request would wait
/// forever for a reply nobody reads (the "second snip does nothing" bug).
fn block_on<F: Future>(fut: F) -> Result<F::Output> {
    static RUNTIME: OnceLock<std::io::Result<Runtime>> = OnceLock::new();
    let rt = RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("portal")
                .enable_all()
                .build()
        })
        .as_ref()
        .map_err(|e| anyhow!("cannot start async runtime: {e}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use ashpd::desktop::settings::Settings;

    async fn read_color_scheme() -> ashpd::Result<()> {
        Settings::new().await?.color_scheme().await?;
        Ok(())
    }

    /// Needs a desktop session with xdg-desktop-portal: `cargo test -- --ignored`.
    /// Regression test for requests after the first one hanging forever.
    #[test]
    #[ignore]
    fn portal_works_for_repeated_requests() {
        for i in 0..3 {
            let (tx, rx) = std::sync::mpsc::channel();
            // Each request from a fresh thread, like the capture / save dialog threads.
            std::thread::spawn(move || {
                let _ = tx.send(block_on(read_color_scheme()).map(|r| r.is_ok()));
            });
            let ok = rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap_or_else(|_| panic!("portal request #{i} hung"))
                .expect("runtime");
            assert!(ok, "portal request #{i} failed");
        }
    }
}
