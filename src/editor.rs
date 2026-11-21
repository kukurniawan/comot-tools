//! Main window: toolbar (New / mode / delay / tools / undo / copy / save) and the canvas.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui::{
    self, pos2, vec2, Color32, CursorIcon, Key, KeyboardShortcut, Modifiers, Pos2, Rect, RichText, Sense, Shape,
};

use crate::document::{InkStroke, StrokeKind};
use crate::gfx;
use crate::portal;
use crate::session::{self, Action, Session, SnipMode, Tool};

const PEN_COLORS: [Color32; 7] = [
    Color32::from_rgb(20, 20, 20),
    Color32::WHITE,
    Color32::from_rgb(232, 17, 35),
    Color32::from_rgb(0, 120, 215),
    Color32::from_rgb(16, 137, 62),
    Color32::from_rgb(255, 140, 0),
    Color32::from_rgb(136, 23, 152),
];

const HIGHLIGHT_COLORS: [Color32; 5] = [
    Color32::from_rgb(255, 240, 0),
    Color32::from_rgb(120, 255, 90),
    Color32::from_rgb(0, 230, 255),
    Color32::from_rgb(255, 120, 210),
    Color32::from_rgb(255, 170, 40),
];

const DELAYS: [u32; 4] = [0, 3, 5, 10];

/// Size of the toolbar relative to egui's defaults (2.0 = 200%).
const TOOLBAR_SCALE: f32 = 2.0;

/// Eraser radius in screen points.
const ERASER_RADIUS: f32 = 10.0;

type SaveResult = anyhow::Result<Option<PathBuf>>;

/// Show the editor window until it is closed. `session.next` tells the caller
/// what to do afterwards (start a snip or quit).
pub fn run(session: &mut Session) -> anyhow::Result<()> {
    let shared = Rc::new(RefCell::new(std::mem::take(session)));
    shared.borrow_mut().next = Action::Quit;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Snipping Tool")
            .with_app_id("snipping-tool")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([560.0, 320.0]),
        ..Default::default()
    };

    let app_shared = Rc::clone(&shared);
    let result = eframe::run_native(
        "Snipping Tool",
        options,
        Box::new(move |_cc| Ok(Box::new(EditorApp::new(app_shared)))),
    );

    *session = std::mem::take(&mut *shared.borrow_mut());
    result.map_err(|e| anyhow::anyhow!("editor window failed: {e}"))
}

struct EditorApp {
    session: Rc<RefCell<Session>>,
    texture: Option<egui::TextureHandle>,
    /// The document changed and the texture must be re-rendered.
    dirty: bool,
    /// Displayed scale of the image (for the status bar).
    last_scale: f32,
    /// Stroke being drawn right now.
    current: Option<InkStroke>,
    erasing: bool,
    erase_checkpointed: bool,
    /// Crop selection in image pixels.
    crop_start: Option<Pos2>,
    crop_sel: Option<Rect>,
    /// Pending "Save as" dialog running on a background thread.
    save_rx: Option<Receiver<SaveResult>>,
}

impl EditorApp {
    fn new(session: Rc<RefCell<Session>>) -> Self {
        Self {
            session,
            texture: None,
            dirty: true,
            last_scale: 1.0,
            current: None,
            erasing: false,
            erase_checkpointed: false,
            crop_start: None,
            crop_sel: None,
            save_rx: None,
        }
    }

    // ----- actions ---------------------------------------------------------

    fn new_snip(&mut self, ctx: &egui::Context, s: &mut Session) {
        if self.save_rx.is_some() {
            return;
        }
        s.next = Action::Snip;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn set_tool(&mut self, s: &mut Session, tool: Tool) {
        s.settings.tool = tool;
        if tool != Tool::Crop {
            self.clear_crop();
        }
    }

    fn clear_crop(&mut self) {
        self.crop_sel = None;
        self.crop_start = None;
    }

    fn after_history_change(&mut self) {
        self.dirty = true;
        self.current = None;
        self.clear_crop();
    }

    fn undo(&mut self, s: &mut Session) {
        if s.doc.as_mut().is_some_and(|d| d.undo()) {
            self.after_history_change();
        }
    }

    fn redo(&mut self, s: &mut Session) {
        if s.doc.as_mut().is_some_and(|d| d.redo()) {
            self.after_history_change();
        }
    }

    fn apply_crop(&mut self, s: &mut Session) {
        let sel = self.crop_sel.take();
        self.crop_start = None;
        let (Some(sel), Some(doc)) = (sel, s.doc.as_mut()) else {
            return;
        };
        let x0 = sel.min.x.floor().max(0.0) as u32;
        let y0 = sel.min.y.floor().max(0.0) as u32;
        let x1 = sel.max.x.ceil().max(0.0) as u32;
        let y1 = sel.max.y.ceil().max(0.0) as u32;
        if x1 > x0 && y1 > y0 && doc.crop(x0, y0, x1 - x0, y1 - y0) {
            self.dirty = true;
            let (w, h) = doc.size();
            s.status = format!("Cropped to {w} × {h}");
        }
    }

    fn copy(&mut self, s: &mut Session) {
        if s.doc.is_none() {
            return;
        }
        s.status = match s.copy_to_clipboard() {
            Ok(()) => "Copied to clipboard".into(),
            Err(e) => format!("Copy failed: {e:#}"),
        };
    }

    fn save(&mut self, ctx: &egui::Context, s: &mut Session) {
        if s.doc.is_none() || self.save_rx.is_some() {
            return;
        }
        let name = session::default_save_path()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Snip.png".into());
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        // The portal dialog is another process; don't block our UI thread on it.
        std::thread::spawn(move || {
            let _ = tx.send(portal::save_file_dialog(&name));
            ctx.request_repaint();
        });
        self.save_rx = Some(rx);
        s.status = "Choose where to save…".into();
    }

    fn poll_save(&mut self, s: &mut Session) {
        let Some(rx) = &self.save_rx else { return };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("save dialog ended unexpectedly")),
        };
        self.save_rx = None;
        let Some(doc) = s.doc.as_ref() else { return };

        s.status = match result {
            Ok(None) => "Save cancelled".into(),
            Ok(Some(path)) => match session::save_image(&doc.render(), &path) {
                Ok(p) => format!("Saved to {}", p.display()),
                Err(e) => format!("Save failed: {e:#}"),
            },
            Err(dialog_err) => {
                // No portal file chooser available: save to the default folder instead.
                let path = session::default_save_path();
                match session::save_image(&doc.render(), &path) {
                    Ok(p) => format!("Save dialog unavailable ({dialog_err}); saved to {}", p.display()),
                    Err(e) => format!("Save failed: {e:#}"),
                }
            }
        };
    }

    // ----- input -----------------------------------------------------------

    fn shortcuts(&mut self, ctx: &egui::Context, s: &mut Session) {
        let cmd = |k: Key| KeyboardShortcut::new(Modifiers::COMMAND, k);
        let (new, redo, undo, save, copy) = ctx.input_mut(|i| {
            let new = i.consume_shortcut(&cmd(Key::N));
            // Check Ctrl+Shift+Z before Ctrl+Z (the latter also matches with Shift held).
            let redo = i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z))
                || i.consume_shortcut(&cmd(Key::Y));
            let undo = i.consume_shortcut(&cmd(Key::Z));
            let save = i.consume_shortcut(&cmd(Key::S));
            // egui-winit turns Ctrl+C into Event::Copy.
            let copy = i.consume_shortcut(&cmd(Key::C)) || i.events.iter().any(|e| matches!(e, egui::Event::Copy));
            (new, redo, undo, save, copy)
        });
        let (tool, enter, escape) = ctx.input(|i| {
            let tool = if !i.modifiers.is_none() {
                None
            } else if i.key_pressed(Key::P) {
                Some(Tool::Pen)
            } else if i.key_pressed(Key::H) {
                Some(Tool::Highlighter)
            } else if i.key_pressed(Key::E) {
                Some(Tool::Eraser)
            } else if i.key_pressed(Key::C) {
                Some(Tool::Crop)
            } else {
                None
            };
            (tool, i.key_pressed(Key::Enter), i.key_pressed(Key::Escape))
        });

        if new {
            self.new_snip(ctx, s);
            return;
        }
        if undo {
            self.undo(s);
        }
        if redo {
            self.redo(s);
        }
        if copy {
            self.copy(s);
        }
        if save {
            self.save(ctx, s);
        }
        if s.doc.is_some() {
            if let Some(t) = tool {
                self.set_tool(s, t);
            }
        }
        if s.settings.tool == Tool::Crop {
            if enter {
                self.apply_crop(s);
            }
            if escape {
                self.clear_crop();
            }
        }
    }

    // ----- panels ----------------------------------------------------------

    fn toolbar(&mut self, ctx: &egui::Context, s: &mut Session) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            scale_style(ui.style_mut(), TOOLBAR_SCALE);
            ui.add_space(6.0 * TOOLBAR_SCALE);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0 * TOOLBAR_SCALE;
                let busy = self.save_rx.is_some();

                let new_btn = egui::Button::new(RichText::new("✂ New").strong());
                if ui.add_enabled(!busy, new_btn).on_hover_text("New snip (Ctrl+N)").clicked() {
                    self.new_snip(ctx, s);
                }
                egui::ComboBox::from_id_salt("snip_mode")
                    .width(110.0 * TOOLBAR_SCALE)
                    .selected_text(s.settings.mode.label())
                    .show_ui(ui, |ui| {
                        // The dropdown is a separate popup that doesn't inherit our style.
                        scale_style(ui.style_mut(), TOOLBAR_SCALE);
                        for m in [SnipMode::Rectangle, SnipMode::FullScreen] {
                            ui.selectable_value(&mut s.settings.mode, m, m.label());
                        }
                    });
                egui::ComboBox::from_id_salt("snip_delay")
                    .width(100.0 * TOOLBAR_SCALE)
                    .selected_text(delay_label(s.settings.delay_secs))
                    .show_ui(ui, |ui| {
                        scale_style(ui.style_mut(), TOOLBAR_SCALE);
                        for d in DELAYS {
                            ui.selectable_value(&mut s.settings.delay_secs, d, delay_label(d));
                        }
                    });

                if s.doc.is_none() {
                    return;
                }

                ui.separator();
                let tools = [
                    (Tool::Pen, "🖊 Pen", "Pen (P)"),
                    (Tool::Highlighter, "🖍 Highlighter", "Highlighter (H)"),
                    (Tool::Eraser, "⌫ Eraser", "Eraser — drag over strokes to remove them (E)"),
                    (Tool::Crop, "✂ Crop", "Crop (C)"),
                ];
                for (tool, label, hint) in tools {
                    if ui.selectable_label(s.settings.tool == tool, label).on_hover_text(hint).clicked() {
                        self.set_tool(s, tool);
                    }
                }

                ui.separator();
                match s.settings.tool {
                    Tool::Pen => {
                        color_row(ui, &mut s.settings.pen_color, &PEN_COLORS);
                        ui.add(egui::Slider::new(&mut s.settings.pen_size, 1.0..=30.0).suffix(" px"));
                    }
                    Tool::Highlighter => {
                        color_row(ui, &mut s.settings.highlight_color, &HIGHLIGHT_COLORS);
                        ui.add(egui::Slider::new(&mut s.settings.highlight_size, 6.0..=60.0).suffix(" px"));
                    }
                    Tool::Eraser => {
                        ui.label(RichText::new("Drag over strokes to erase them").weak());
                    }
                    Tool::Crop => {
                        let has_sel = self.crop_sel.is_some();
                        if ui
                            .add_enabled(has_sel, egui::Button::new("✔ Apply"))
                            .on_hover_text("Apply crop (Enter)")
                            .clicked()
                        {
                            self.apply_crop(s);
                        }
                        if ui
                            .add_enabled(has_sel, egui::Button::new("✖ Cancel"))
                            .on_hover_text("Clear selection (Esc)")
                            .clicked()
                        {
                            self.clear_crop();
                        }
                    }
                }

                ui.separator();
                let (can_undo, can_redo) = s
                    .doc
                    .as_ref()
                    .map(|d| (d.can_undo(), d.can_redo()))
                    .unwrap_or_default();
                if ui.add_enabled(can_undo, egui::Button::new("⟲ Undo")).on_hover_text("Ctrl+Z").clicked() {
                    self.undo(s);
                }
                if ui.add_enabled(can_redo, egui::Button::new("⟳ Redo")).on_hover_text("Ctrl+Y").clicked() {
                    self.redo(s);
                }

                ui.separator();
                if ui.button("📋 Copy").on_hover_text("Copy to clipboard (Ctrl+C)").clicked() {
                    self.copy(s);
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("💾 Save"))
                    .on_hover_text("Save as PNG or JPEG (Ctrl+S)")
                    .clicked()
                {
                    self.save(ctx, s);
                }
            });
            ui.add_space(6.0 * TOOLBAR_SCALE);
        });
    }

    fn status_bar(&self, ctx: &egui::Context, s: &Session) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(doc) = &s.doc {
                    let (w, h) = doc.size();
                    ui.label(format!("{w} × {h} px"));
                    ui.separator();
                    ui.label(format!("{:.0}%", self.last_scale * 100.0));
                    ui.separator();
                }
                ui.label(s.status.as_str());
            });
        });
    }

    fn canvas(&mut self, ctx: &egui::Context, s: &mut Session) {
        let settings = s.settings.clone();
        let frame = egui::Frame::none()
            .fill(ctx.style().visuals.extreme_bg_color)
            .inner_margin(16.0);

        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let Some(doc) = s.doc.as_mut() else {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        RichText::new("Click “✂ New” or press Ctrl+N to take a snip")
                            .size(18.0)
                            .weak(),
                    );
                });
                return;
            };

            // Fit the image into the available space (never upscale).
            let (iw, ih) = doc.size();
            let (iw, ih) = (iw as f32, ih as f32);
            let avail = ui.available_rect_before_wrap();
            let scale = (avail.width() / iw).min(avail.height() / ih).clamp(0.01, 1.0);
            self.last_scale = scale;
            let rect = Rect::from_center_size(avail.center(), vec2(iw * scale, ih * scale));
            let response = ui.allocate_rect(rect, Sense::click_and_drag());

            let to_img = move |p: Pos2| pos2((p.x - rect.min.x) / scale, (p.y - rect.min.y) / scale);
            let to_screen = move |p: [f32; 2]| pos2(rect.min.x + p[0] * scale, rect.min.y + p[1] * scale);

            let (pressed, down, pointer) = ui.input(|i| {
                (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.latest_pos())
            });
            let start_here = pressed && response.contains_pointer();
            let hovered = response.hovered();

            // --- interaction ---
            match settings.tool {
                Tool::Pen | Tool::Highlighter => {
                    if hovered {
                        ctx.set_cursor_icon(CursorIcon::Crosshair);
                    }
                    if start_here {
                        if let Some(p) = pointer {
                            let (kind, color, size) = if settings.tool == Tool::Pen {
                                (StrokeKind::Pen, settings.pen_color, settings.pen_size)
                            } else {
                                (StrokeKind::Highlighter, settings.highlight_color, settings.highlight_size)
                            };
                            let ip = to_img(p);
                            self.current = Some(InkStroke {
                                kind,
                                color: color.to_srgba_unmultiplied(),
                                // Size is chosen in screen pixels, stored in image pixels.
                                width: size / scale,
                                points: vec![[ip.x, ip.y]],
                            });
                        }
                    }
                    if down {
                        if let (Some(p), Some(cur)) = (pointer, self.current.as_mut()) {
                            let ip = to_img(p);
                            let last = cur.points[cur.points.len() - 1];
                            if (ip.x - last[0]).hypot(ip.y - last[1]) * scale >= 1.0 {
                                cur.points.push([ip.x, ip.y]);
                            }
                        }
                    } else if let Some(stroke) = self.current.take() {
                        doc.add_stroke(stroke);
                        self.dirty = true;
                    }
                }
                Tool::Eraser => {
                    if hovered {
                        ctx.set_cursor_icon(CursorIcon::None);
                    }
                    if start_here {
                        self.erasing = true;
                        self.erase_checkpointed = false;
                    }
                    if self.erasing {
                        if down {
                            if let Some(p) = pointer {
                                let ip = to_img(p);
                                if doc.erase_at([ip.x, ip.y], ERASER_RADIUS / scale, !self.erase_checkpointed) {
                                    self.erase_checkpointed = true;
                                    self.dirty = true;
                                }
                            }
                        } else {
                            self.erasing = false;
                        }
                    }
                }
                Tool::Crop => {
                    if hovered {
                        ctx.set_cursor_icon(CursorIcon::Crosshair);
                    }
                    let clamp = |p: Pos2| pos2(p.x.clamp(0.0, iw), p.y.clamp(0.0, ih));
                    if start_here {
                        if let Some(p) = pointer {
                            self.crop_start = Some(clamp(to_img(p)));
                            self.crop_sel = None;
                        }
                    }
                    if let Some(start) = self.crop_start {
                        if down {
                            if let Some(p) = pointer {
                                self.crop_sel = Some(Rect::from_two_pos(start, clamp(to_img(p))));
                            }
                        } else {
                            self.crop_start = None;
                            let too_small = self
                                .crop_sel
                                .map_or(true, |r| r.width() * scale < 3.0 || r.height() * scale < 3.0);
                            if too_small {
                                self.crop_sel = None;
                            }
                        }
                    }
                }
            }

            // --- drawing ---
            if self.dirty || self.texture.is_none() {
                self.texture = Some(gfx::upload(ctx, "canvas", &doc.render()));
                self.dirty = false;
            }

            ui.painter()
                .rect_filled(rect.translate(vec2(0.0, 2.0)).expand(1.0), 2.0, Color32::from_black_alpha(60));
            let painter = ui.painter_at(rect);
            if let Some(tex) = &self.texture {
                painter.image(tex.id(), rect, gfx::full_uv(), Color32::WHITE);
            }
            if let Some(cur) = &self.current {
                draw_live_stroke(&painter, cur, scale, to_screen);
            }

            match settings.tool {
                Tool::Eraser => {
                    if let (true, Some(p)) = (hovered || self.erasing, pointer) {
                        let r = ERASER_RADIUS;
                        ui.painter().circle_stroke(p, r, egui::Stroke::new(2.5_f32, Color32::from_gray(30)));
                        ui.painter().circle_stroke(p, r, egui::Stroke::new(1.0_f32, Color32::WHITE));
                    }
                }
                Tool::Crop => match self.crop_sel {
                    Some(sel) => {
                        let sr = Rect::from_min_max(to_screen([sel.min.x, sel.min.y]), to_screen([sel.max.x, sel.max.y]));
                        gfx::dim_outside(&painter, rect, sr, Color32::from_black_alpha(140));
                        gfx::selection_frame(&painter, sr);
                        let label = if self.crop_start.is_some() {
                            format!("{} × {}", sel.width().round() as u32, sel.height().round() as u32)
                        } else {
                            "Enter: apply crop   •   Esc: cancel".to_owned()
                        };
                        gfx::pill(&painter, pos2(sr.center().x, sr.min.y + 8.0), &label);
                    }
                    None => {
                        painter.rect_filled(rect, 0.0, Color32::from_black_alpha(50));
                        gfx::pill(&painter, pos2(rect.center().x, rect.min.y + 12.0), "Drag to select the area to keep");
                    }
                },
                Tool::Pen | Tool::Highlighter => {}
            }
        });
    }
}

impl eframe::App for EditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let shared = Rc::clone(&self.session);
        let mut s = shared.borrow_mut();
        self.poll_save(&mut s);
        self.shortcuts(ctx, &mut s);
        self.toolbar(ctx, &mut s);
        self.status_bar(ctx, &s);
        self.canvas(ctx, &mut s);
    }
}

fn delay_label(secs: u32) -> String {
    if secs == 0 {
        "No delay".into()
    } else {
        format!("{secs} s delay")
    }
}

fn color_row(ui: &mut egui::Ui, current: &mut Color32, palette: &[Color32]) {
    for &c in palette {
        if swatch(ui, c, *current == c).clicked() {
            *current = c;
        }
    }
    egui::color_picker::color_edit_button_srgba(ui, current, egui::color_picker::Alpha::Opaque)
        .on_hover_text("Custom colour");
}

fn swatch(ui: &mut egui::Ui, color: Color32, selected: bool) -> egui::Response {
    let k = TOOLBAR_SCALE;
    let (rect, response) = ui.allocate_exact_size(vec2(20.0 * k, 20.0 * k), Sense::click());
    let painter = ui.painter();
    let c = rect.center();
    if selected {
        painter.circle_stroke(c, 9.5 * k, egui::Stroke::new(2.0 * k, ui.visuals().selection.stroke.color));
    } else if response.hovered() {
        painter.circle_stroke(c, 9.5 * k, egui::Stroke::new(1.0 * k, ui.visuals().weak_text_color()));
    }
    painter.circle_filled(c, 7.0 * k, color);
    painter.circle_stroke(c, 7.0 * k, egui::Stroke::new(1.0_f32, Color32::from_gray(110)));
    response
}

/// Scale fonts and widget sizes of a style (used for the toolbar only).
fn scale_style(style: &mut egui::Style, k: f32) {
    for font in style.text_styles.values_mut() {
        font.size *= k;
    }
    let s = &mut style.spacing;
    s.item_spacing *= k;
    s.button_padding *= k;
    s.interact_size *= k;
    s.icon_width *= k;
    s.icon_width_inner *= k;
    s.icon_spacing *= k;
    s.slider_width *= k;
    s.combo_width *= k;
}

/// Preview of the stroke being drawn (the final look comes from `Document::render`).
fn draw_live_stroke(painter: &egui::Painter, s: &InkStroke, scale: f32, to_screen: impl Fn([f32; 2]) -> Pos2) {
    let [r, g, b, a] = s.color;
    let color = match s.kind {
        StrokeKind::Pen => Color32::from_rgba_unmultiplied(r, g, b, a),
        StrokeKind::Highlighter => Color32::from_rgba_unmultiplied(r, g, b, 110),
    };
    let width = (s.width * scale).max(1.0);
    if let [p] = s.points.as_slice() {
        painter.circle_filled(to_screen(*p), width / 2.0, color);
    } else {
        let pts = s.points.iter().map(|&p| to_screen(p)).collect();
        painter.add(Shape::line(pts, egui::Stroke::new(width, color)));
    }
}
