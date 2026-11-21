//! Snipping Tool — a small Windows-style screen snipping & annotation app.
//!
//! The app runs as a sequence of windows so that nothing of ours is on screen
//! while the screenshot is taken (important on Wayland, where a window cannot
//! be hidden programmatically):
//!
//!   editor window ──(New)──▶ invisible capture window ─▶ fullscreen selector ─▶ editor window …

mod capture;
mod document;
mod editor;
mod gfx;
mod portal;
mod selector;
mod session;

use document::Document;
use session::{Action, Session, SnipMode};

const USAGE: &str = "\
Snipping Tool

USAGE:
    snipping-tool [OPTIONS]

OPTIONS:
    -s, --snip     Start a new snip immediately (bind this to a keyboard shortcut)
    -f, --full     Use full-screen mode instead of rectangle selection
    -h, --help     Show this help
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |names: &[&str]| args.iter().any(|a| names.contains(&a.as_str()));

    if has(&["-h", "--help"]) {
        print!("{USAGE}");
        return;
    }

    let mut session = Session::default();
    if has(&["-f", "--full"]) {
        session.settings.mode = SnipMode::FullScreen;
    }

    let mut next = if has(&["-s", "--snip"]) {
        Action::Snip
    } else {
        Action::ShowEditor
    };

    loop {
        next = match next {
            Action::ShowEditor => {
                if let Err(e) = editor::run(&mut session) {
                    eprintln!("error: {e:#}");
                    std::process::exit(1);
                }
                std::mem::take(&mut session.next)
            }
            Action::Snip => {
                take_snip(&mut session);
                Action::ShowEditor
            }
            Action::Quit => break,
        };
    }
}

/// Capture the screen, let the user pick a region and store the result as the
/// current document.
fn take_snip(s: &mut Session) {
    let shot = match capture::run(s.settings.delay_secs) {
        Ok(Some(shot)) => shot,
        Ok(None) => {
            s.status = "Snip cancelled".into();
            return;
        }
        Err(e) => {
            eprintln!("screenshot failed: {e:#}");
            s.status = format!("Screenshot failed: {e:#}");
            return;
        }
    };

    // With the desktop's interactive screenshot UI the area is already chosen.
    let snip = match (s.settings.mode, shot.interactive) {
        (SnipMode::FullScreen, _) | (_, true) => Some(shot.image),
        (SnipMode::Rectangle, false) => match selector::run(shot.image) {
            Ok(snip) => snip,
            Err(e) => {
                eprintln!("selection failed: {e:#}");
                s.status = format!("Selection failed: {e:#}");
                return;
            }
        },
    };

    let Some(img) = snip else {
        s.status = "Snip cancelled".into();
        return;
    };

    let (w, h) = img.dimensions();
    s.doc = Some(Document::new(img));
    s.status = format!("New snip {w} × {h}");
    if s.settings.auto_copy {
        match s.copy_to_clipboard() {
            Ok(()) => s.status.push_str(" — copied to clipboard"),
            Err(e) => s.status.push_str(&format!(" — clipboard error: {e:#}")),
        }
    }
}
