//! Small drawing helpers shared by the editor and the selection overlay.

use eframe::egui::{self, pos2, vec2, Color32, ColorImage, FontId, Painter, Pos2, Rect, TextureHandle, TextureOptions};
use image::{imageops, RgbaImage};

/// Upload an image as a texture, downscaling if it exceeds the GPU limit.
pub fn upload(ctx: &egui::Context, name: &str, img: &RgbaImage) -> TextureHandle {
    let max = ctx.input(|i| i.max_texture_side).max(1) as u32;
    let color_image = if img.width() > max || img.height() > max {
        let k = max as f32 / img.width().max(img.height()) as f32;
        let w = ((img.width() as f32 * k) as u32).clamp(1, max);
        let h = ((img.height() as f32 * k) as u32).clamp(1, max);
        let small = imageops::resize(img, w, h, imageops::FilterType::Triangle);
        ColorImage::from_rgba_unmultiplied([w as usize, h as usize], small.as_raw())
    } else {
        ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
    };
    ctx.load_texture(name, color_image, TextureOptions::LINEAR)
}

/// The full-texture UV rectangle.
pub fn full_uv() -> Rect {
    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))
}

/// Darken everything in `outer` except `inner`.
pub fn dim_outside(painter: &Painter, outer: Rect, inner: Rect, color: Color32) {
    let i = inner.intersect(outer);
    let rects = [
        Rect::from_min_max(outer.min, pos2(outer.max.x, i.min.y)),
        Rect::from_min_max(pos2(outer.min.x, i.max.y), outer.max),
        Rect::from_min_max(pos2(outer.min.x, i.min.y), pos2(i.min.x, i.max.y)),
        Rect::from_min_max(pos2(i.max.x, i.min.y), pos2(outer.max.x, i.max.y)),
    ];
    for r in rects {
        if r.is_positive() {
            painter.rect_filled(r, 0.0, color);
        }
    }
}

/// A rounded dark label whose top edge is centred on `center_top`.
pub fn pill(painter: &Painter, center_top: Pos2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(14.0), Color32::WHITE);
    let size = galley.size() + vec2(20.0, 10.0);
    let rect = Rect::from_min_size(pos2(center_top.x - size.x / 2.0, center_top.y), size);
    painter.rect_filled(rect, size.y / 2.0, Color32::from_black_alpha(200));
    painter.galley(rect.min + vec2(10.0, 5.0), galley, Color32::WHITE);
}

/// Selection border with small square handles on the corners.
pub fn selection_frame(painter: &Painter, r: Rect) {
    painter.rect_stroke(r, 0.0, egui::Stroke::new(1.5_f32, Color32::WHITE));
    for c in [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()] {
        let h = Rect::from_center_size(c, vec2(8.0, 8.0));
        painter.rect_filled(h, 1.0, Color32::WHITE);
        painter.rect_stroke(h, 1.0, egui::Stroke::new(1.0_f32, Color32::from_gray(60)));
    }
}
