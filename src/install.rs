//! `--install` / `--uninstall`: put the app into the user's desktop menu.
//!
//! On Wayland the compositor shows the icon of the `.desktop` file whose name
//! matches the window's app id (`snipping-tool`), so the icon only appears once
//! the desktop entry and the themed icons are installed:
//!
//!   ~/.local/bin/snipping-tool
//!   ~/.local/share/applications/snipping-tool.desktop
//!   ~/.local/share/icons/hicolor/<N>x<N>/apps/snipping-tool.png

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::icon;

pub const APP_ID: &str = "snipping-tool";

struct Paths {
    bin: PathBuf,
    desktop: PathBuf,
    icons: PathBuf,
}

impl Paths {
    fn new() -> Result<Self> {
        let home = dirs::home_dir().context("cannot find home directory")?;
        let bin_dir = dirs::executable_dir().unwrap_or_else(|| home.join(".local/bin"));
        let data = dirs::data_dir().unwrap_or_else(|| home.join(".local/share"));
        Ok(Self {
            bin: bin_dir.join(APP_ID),
            desktop: data.join("applications").join(format!("{APP_ID}.desktop")),
            icons: data.join("icons/hicolor"),
        })
    }

    fn icon(&self, size: u32) -> PathBuf {
        self.icons.join(format!("{size}x{size}/apps/{APP_ID}.png"))
    }
}

pub fn install() -> Result<()> {
    let paths = Paths::new()?;

    // Binary (skip if we are already running the installed copy).
    let exe = std::env::current_exe().context("cannot locate the running executable")?;
    let same = fs::canonicalize(&exe).ok() == fs::canonicalize(&paths.bin).ok();
    if !same {
        create_parent(&paths.bin)?;
        // Remove first so replacing a running binary doesn't fail with "text file busy".
        let _ = fs::remove_file(&paths.bin);
        fs::copy(&exe, &paths.bin).with_context(|| format!("cannot copy binary to {}", paths.bin.display()))?;
    }
    println!("binary   {}", paths.bin.display());

    // Icons.
    for size in icon::THEME_SIZES {
        let path = paths.icon(size);
        create_parent(&path)?;
        icon::render(size)
            .save_png(&path)
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    println!("icons    {}/*/apps/{APP_ID}.png", paths.icons.display());

    // Desktop entry.
    create_parent(&paths.desktop)?;
    fs::write(&paths.desktop, desktop_entry(&paths.bin))
        .with_context(|| format!("cannot write {}", paths.desktop.display()))?;
    println!("launcher {}", paths.desktop.display());

    refresh_caches(&paths);
    println!("\nInstalled. \"Snipping Tool\" is now in the app menu.");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let paths = Paths::new()?;
    let mut files = vec![paths.desktop.clone(), paths.bin.clone()];
    files.extend(icon::THEME_SIZES.iter().map(|&s| paths.icon(s)));
    for f in files {
        match fs::remove_file(&f) {
            Ok(()) => println!("removed  {}", f.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("cannot remove {}: {e}", f.display()),
        }
    }
    refresh_caches(&paths);
    println!("\nUninstalled.");
    Ok(())
}

fn desktop_entry(bin: &Path) -> String {
    // Quote the path per the Desktop Entry spec in case it contains spaces.
    let exec = format!("\"{}\"", bin.display().to_string().replace('\\', "\\\\").replace('"', "\\\""));
    format!(
        "[Desktop Entry]
Type=Application
Name=Snipping Tool
GenericName=Screenshot Tool
Comment=Snip the screen and annotate it with pen, highlighter and crop
Exec={exec}
Icon={APP_ID}
Terminal=false
Categories=Utility;
Keywords=screenshot;snip;capture;crop;
StartupWMClass={APP_ID}
StartupNotify=true
Actions=new-snip;

[Desktop Action new-snip]
Name=New Snip
Exec={exec} --snip
"
    )
}

fn create_parent(path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    Ok(())
}

/// Best effort: tell the desktop about new icons / launchers. Both tools are optional.
fn refresh_caches(paths: &Paths) {
    if let Some(apps) = paths.desktop.parent() {
        let _ = Command::new("update-desktop-database").arg(apps).status();
    }
    let _ = Command::new("gtk-update-icon-cache")
        .args(["--force", "--ignore-theme-index", "--quiet"])
        .arg(&paths.icons)
        .status();
}
