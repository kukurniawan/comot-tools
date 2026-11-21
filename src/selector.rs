//! Fullscreen overlay showing the frozen screenshot; the user drags a rectangle.

use std::cell::RefCell;
use std::rc::Rc;

use eframe::egui::{self, pos2, vec2, Color32, CursorIcon, Key, Pos2, Rect, Sense};
use image::{imageops, RgbaImage};

use crate::gfx;

/// Returns the selected part of `shot`, or `None` if the user cancelled.
pub fn run(shot: RgbaImage) -> anyhow::Result<Option<RgbaImage>> {
    let result: Rc<RefCell<Option<RgbaImage>>> = Rc::new(RefCell::new(None));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Snipping Tool — select area")
            .with_app_id("snipping-tool")
            .with_fullscreen(true)
            .with_decorations(false),
        ..Default::default()
    };

    let app_result = Rc::clone(&result);
    eframe::run_native(
        "Snipping Tool",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(SelectorApp {
                shot,
                texture: None,
                start: None,
                current: None,
                result: app_result,
                closing: false,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("selection overlay failed: {e}"))?;

    let snip = result.borrow_mut().take();
    Ok(snip)
}

struct SelectorApp {
    shot: RgbaImage,
    texture: Option<egui::TextureHandle>,
    /// Drag start / current position in screen points.
    start: Option<Pos2>,
    current: Option<Pos2>,
    result: Rc<RefCell<Option<RgbaImage>>>,
    closing: bool,
}

impl SelectorApp {
    fn finish(&mut self, ctx: &egui::Context, snip: Option<RgbaImage>) {
        if self.closing {
            return;
        }
        *self.result.borrow_mut() = snip;
        self.closing = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for SelectorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.texture.is_none() {
            self.texture = Some(gfx::upload(ctx, "screenshot", &self.shot));
        }

        let (escape, enter, right_click) = ctx.input(|i| {
            (
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::Enter),
                i.pointer.secondary_clicked(),
            )
        });
        if right_click && self.start.is_some() {
            // Right click while dragging: abandon this rectangle only.
            self.start = None;
            self.current = None;
        } else if escape || right_click {
            self.finish(ctx, None);
            return;
        }
        if enter {
            let full = self.shot.clone();
            self.finish(ctx, Some(full));
            return;
        }

        let (w, h) = (self.shot.width() as f32, self.shot.height() as f32);
        let frame = egui::Frame::none().fill(Color32::BLACK);
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let screen = ui.max_rect();
            // The screenshot may cover several monitors; fit it without distortion.
            let scale = (screen.width() / w).min(screen.height() / h);
            let img_rect = Rect::from_center_size(screen.center(), vec2(w * scale, h * scale));

            let response = ui.allocate_rect(screen, Sense::click_and_drag());
            if response.hovered() {
                ctx.set_cursor_icon(CursorIcon::Crosshair);
            }

            let clamp = |p: Pos2| {
                pos2(
                    p.x.clamp(img_rect.min.x, img_rect.max.x),
                    p.y.clamp(img_rect.min.y, img_rect.max.y),
                )
            };
            let (pressed, down, pointer) = ui.input(|i| {
                (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.latest_pos())
            });

            if pressed {
                if let Some(p) = pointer {
                    self.start = Some(clamp(p));
                    self.current = self.start;
                }
            }

            let mut snip = None;
            if let Some(start) = self.start {
                if down {
                    if let Some(p) = pointer {
                        self.current = Some(clamp(p));
                    }
                } else {
                    // Mouse released: cut the selection out of the full-resolution image.
                    let sel = Rect::from_two_pos(start, self.current.unwrap_or(start));
                    self.start = None;
                    self.current = None;
                    if sel.width() >= 4.0 && sel.height() >= 4.0 {
                        let to_px = |v: f32, origin: f32, max: f32, round: fn(f32) -> f32| {
                            round((v - origin) / scale).clamp(0.0, max) as u32
                        };
                        let x0 = to_px(sel.min.x, img_rect.min.x, w, f32::floor);
                        let y0 = to_px(sel.min.y, img_rect.min.y, h, f32::floor);
                        let x1 = to_px(sel.max.x, img_rect.min.x, w, f32::ceil);
                        let y1 = to_px(sel.max.y, img_rect.min.y, h, f32::ceil);
                        if x1 > x0 && y1 > y0 {
                            snip = Some(imageops::crop_imm(&self.shot, x0, y0, x1 - x0, y1 - y0).to_image());
                        }
                    }
                }
            }

            let painter = ui.painter();
            if let Some(tex) = &self.texture {
                painter.image(tex.id(), img_rect, gfx::full_uv(), Color32::WHITE);
            }
            let dim = Color32::from_black_alpha(120);
            match (self.start, self.current) {
                (Some(a), Some(b)) => {
                    let sel = Rect::from_two_pos(a, b);
                    gfx::dim_outside(painter, screen, sel, dim);
                    gfx::selection_frame(painter, sel);
                    let label = format!(
                        "{} × {}",
                        (sel.width() / scale).round() as u32,
                        (sel.height() / scale).round() as u32
                    );
                    let y = (sel.max.y + 8.0).min(screen.max.y - 32.0);
                    gfx::pill(painter, pos2(sel.center().x, y), &label);
                }
                _ => {
                    painter.rect_filled(screen, 0.0, dim);
                    gfx::pill(
                        painter,
                        pos2(screen.center().x, screen.min.y + 28.0),
                        "Drag to snip an area   •   Enter: full screen   •   Esc: cancel",
                    );
                }
            }

            if snip.is_some() {
                self.finish(ctx, snip);
            }
        });
    }
}
