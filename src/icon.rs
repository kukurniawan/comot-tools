//! The app icon, drawn in code with tiny-skia so there is a single source for
//! every size (window icon and the PNGs installed into the icon theme).
//!
//! Design (256×256 grid): blue rounded tile, dashed white snip selection with a
//! yellow highlighter stroke and a red pen squiggle, scissors in the corner.

use std::sync::Arc;

use eframe::egui;
use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LinearGradient, Paint, Path, PathBuilder, Pixmap, Point, SpreadMode,
    Stroke, StrokeDash, Transform,
};

/// Sizes installed into the hicolor icon theme.
pub const THEME_SIZES: [u32; 8] = [16, 24, 32, 48, 64, 128, 256, 512];

const BLUE_LIGHT: (u8, u8, u8) = (0x4f, 0x8c, 0xff);
const BLUE_DARK: (u8, u8, u8) = (0x24, 0x42, 0xc9);
const RED: (u8, u8, u8) = (0xff, 0x4d, 0x5e);
const YELLOW: (u8, u8, u8) = (0xff, 0xe6, 0x00);

/// Render the icon at `size`×`size` pixels.
pub fn render(size: u32) -> Pixmap {
    let mut pm = Pixmap::new(size, size).expect("icon size is non-zero");
    let k = size as f32 / 256.0;
    let ts = Transform::from_scale(k, k);

    // Tile with a diagonal gradient.
    let tile = Paint {
        anti_alias: true,
        shader: LinearGradient::new(
            Point::from_xy(8.0, 8.0),
            Point::from_xy(248.0, 248.0),
            vec![
                GradientStop::new(0.0, rgb(BLUE_LIGHT)),
                GradientStop::new(1.0, rgb(BLUE_DARK)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .expect("valid gradient"),
        ..Default::default()
    };
    pm.fill_path(&rounded_rect(8.0, 8.0, 240.0, 240.0, 52.0), &tile, FillRule::Winding, ts, None);

    // Snip selection: translucent fill + dashed border.
    let sel = rounded_rect(40.0, 40.0, 176.0, 132.0, 8.0);
    pm.fill_path(&sel, &solid(Color::from_rgba8(255, 255, 255, 41)), FillRule::Winding, ts, None);
    let dashed = Stroke {
        width: 8.0,
        line_cap: LineCap::Round,
        dash: StrokeDash::new(vec![22.0, 13.0], 0.0),
        ..Default::default()
    };
    pm.stroke_path(&sel, &solid(Color::WHITE), &dashed, ts, None);

    // Highlighter stroke.
    let mut pb = PathBuilder::new();
    pb.move_to(68.0, 84.0);
    pb.line_to(156.0, 84.0);
    stroke(&mut pm, pb.finish(), rgb(YELLOW), 20.0, ts);

    // Pen squiggle (two smooth cubic segments).
    let mut pb = PathBuilder::new();
    pb.move_to(68.0, 128.0);
    pb.cubic_to(88.0, 104.0, 108.0, 152.0, 128.0, 128.0);
    pb.cubic_to(148.0, 104.0, 168.0, 106.0, 188.0, 128.0);
    stroke(&mut pm, pb.finish(), rgb(RED), 9.0, ts);

    // Scissors in the bottom-right corner.
    let st = ts
        .pre_concat(Transform::from_translate(184.0, 190.0))
        .pre_concat(Transform::from_rotate(-38.0))
        .pre_concat(Transform::from_scale(0.82, 0.82));
    for (y0, y1) in [(15.0, -9.0), (-15.0, 9.0)] {
        let mut pb = PathBuilder::new();
        pb.move_to(-30.0, y0);
        pb.line_to(58.0, y1);
        stroke(&mut pm, pb.finish(), Color::WHITE, 12.0, st);
    }
    for cy in [-22.0, 22.0] {
        stroke(&mut pm, PathBuilder::from_circle(-46.0, cy, 16.0), rgb(RED), 10.0, st);
    }
    if let Some(pivot) = PathBuilder::from_circle(0.0, 0.0, 5.0) {
        pm.fill_path(&pivot, &solid(rgb(BLUE_DARK)), FillRule::Winding, st, None);
    }

    pm
}

/// Straight-alpha RGBA pixels of the icon.
pub fn rgba(size: u32) -> Vec<u8> {
    render(size)
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

/// Icon for window decorations / X11 (Wayland takes it from the .desktop file).
pub fn window_icon() -> Arc<egui::IconData> {
    const SIZE: u32 = 128;
    Arc::new(egui::IconData {
        rgba: rgba(SIZE),
        width: SIZE,
        height: SIZE,
    })
}

fn rgb((r, g, b): (u8, u8, u8)) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

fn solid(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    paint
}

fn stroke(pm: &mut Pixmap, path: Option<Path>, color: Color, width: f32, ts: Transform) {
    let Some(path) = path else { return };
    let style = Stroke {
        width,
        line_cap: LineCap::Round,
        ..Default::default()
    };
    pm.stroke_path(&path, &solid(color), &style, ts, None);
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    // Quarter circles approximated by cubics.
    let c = 0.552_284_8 * r;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + c, y, x + w, y + r - c, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + c, x + w - r + c, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - c, y + h, x, y + h - r + c, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - c, x + r - c, y, x + r, y);
    pb.close();
    pb.finish().expect("rounded rect path")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_transparent_corners_and_opaque_tile() {
        for size in THEME_SIZES {
            let px = rgba(size);
            assert_eq!(px.len(), (size * size * 4) as usize);
            assert_eq!(px[3], 0, "top-left corner is transparent at {size}px");
            let mid = ((size / 2) * size + size / 8) as usize * 4;
            assert_eq!(px[mid + 3], 255, "tile is opaque at {size}px");
        }
        // Handy for eyeballing the design: SNIP_ICON_OUT=/tmp/icon.png cargo test icon
        if let Ok(path) = std::env::var("SNIP_ICON_OUT") {
            render(256).save_png(path).expect("write icon preview");
        }
    }
}
