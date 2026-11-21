//! Capture phase: an invisible window that keeps the event loop running while
//! the screenshot is taken.
//!
//! Why a window at all? On Wayland, winit only *schedules* the destruction of a
//! closed window; it disappears the next time the event loop runs. If we blocked
//! the main thread on the portal right after the editor closed, the editor would
//! stay on screen frozen (and end up in the screenshot). Running the capture as
//! its own eframe phase pumps the event loop, so the old window really goes away.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::portal::{self, Shot};

/// Time for the compositor to finish the editor's close animation.
const SETTLE: Duration = Duration::from_millis(400);

type CaptureResult = anyhow::Result<Option<Shot>>;

/// Wait `delay_secs`, then take a screenshot. `Ok(None)` = cancelled by the user.
pub fn run(delay_secs: u32) -> CaptureResult {
    let result: Rc<RefCell<Option<CaptureResult>>> = Rc::new(RefCell::new(None));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Snipping Tool")
            .with_app_id("snipping-tool")
            // Tiny, transparent and click-through. Don't lock the size (min == max):
            // winit's Wayland minimum is 2×1 and a smaller max is a protocol error.
            .with_inner_size([8.0, 8.0])
            .with_decorations(false)
            .with_transparent(true)
            .with_mouse_passthrough(true),
        ..Default::default()
    };

    let app_result = Rc::clone(&result);
    eframe::run_native(
        "Snipping Tool",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(CaptureApp {
                start_at: Instant::now() + SETTLE + Duration::from_secs(delay_secs.into()),
                rx: None,
                result: app_result,
                closing: false,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("capture window failed: {e}"))?;

    let outcome = result.borrow_mut().take();
    outcome.unwrap_or_else(|| Err(anyhow::anyhow!("capture was interrupted")))
}

struct CaptureApp {
    start_at: Instant,
    rx: Option<Receiver<CaptureResult>>,
    result: Rc<RefCell<Option<CaptureResult>>>,
    closing: bool,
}

impl eframe::App for CaptureApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.closing {
            return;
        }

        let Some(rx) = &self.rx else {
            let now = Instant::now();
            if now < self.start_at {
                ctx.request_repaint_after((self.start_at - now).min(Duration::from_millis(100)));
                return;
            }
            // The portal call blocks; run it off the UI thread.
            let (tx, rx) = mpsc::channel();
            let repaint = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(portal::capture_screen());
                repaint.request_repaint();
            });
            self.rx = Some(rx);
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        };

        let outcome = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
                return;
            }
            Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("screenshot thread ended unexpectedly")),
        };
        *self.result.borrow_mut() = Some(outcome);
        self.closing = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
