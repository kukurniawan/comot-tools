//! The editable snip: a base image plus vector ink strokes, with undo/redo.
//!
//! Strokes stay as vectors (in image pixel coordinates) until the image is
//! rendered, so cropping and undo are cheap and lossless.

use std::sync::Arc;

use image::{imageops, RgbaImage};
use tiny_skia::{BlendMode, FillRule, IntSize, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Transform};

const MAX_HISTORY: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrokeKind {
    Pen,
    Highlighter,
}

#[derive(Clone, Debug)]
pub struct InkStroke {
    pub kind: StrokeKind,
    /// Straight (non-premultiplied) sRGBA.
    pub color: [u8; 4],
    /// Line width in image pixels.
    pub width: f32,
    /// Points in image pixel coordinates.
    pub points: Vec<[f32; 2]>,
}

impl InkStroke {
    /// Does a circle at `p` with `radius` touch this stroke?
    fn hit(&self, p: [f32; 2], radius: f32) -> bool {
        let r = radius + self.width * 0.5;
        let r2 = r * r;
        match self.points.as_slice() {
            [] => false,
            [a] => dist2(*a, p) <= r2,
            pts => pts.windows(2).any(|w| seg_dist2(w[0], w[1], p) <= r2),
        }
    }
}

fn dist2(a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
    dx * dx + dy * dy
}

fn seg_dist2(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= f32::EPSILON {
        0.0
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    };
    dist2([a[0] + dx * t, a[1] + dy * t], p)
}

#[derive(Clone)]
struct State {
    base: Arc<RgbaImage>,
    strokes: Vec<InkStroke>,
}

pub struct Document {
    state: State,
    undo: Vec<State>,
    redo: Vec<State>,
}

impl Document {
    pub fn new(img: RgbaImage) -> Self {
        Self {
            state: State {
                base: Arc::new(img),
                strokes: Vec::new(),
            },
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.state.base.dimensions()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Remember the current state so the next change can be undone.
    fn checkpoint(&mut self) {
        self.undo.push(self.state.clone());
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn add_stroke(&mut self, stroke: InkStroke) {
        if stroke.points.is_empty() {
            return;
        }
        self.checkpoint();
        self.state.strokes.push(stroke);
    }

    /// Remove every stroke touched by the eraser circle. Returns whether
    /// anything was removed. `checkpoint` should be true only for the first
    /// removal of an eraser drag so a whole drag undoes in one step.
    pub fn erase_at(&mut self, p: [f32; 2], radius: f32, checkpoint: bool) -> bool {
        if !self.state.strokes.iter().any(|s| s.hit(p, radius)) {
            return false;
        }
        if checkpoint {
            self.checkpoint();
        }
        self.state.strokes.retain(|s| !s.hit(p, radius));
        true
    }

    /// Crop to the given rectangle (image pixels). Strokes are kept as vectors
    /// and shifted; parts outside the new bounds are simply clipped on render.
    pub fn crop(&mut self, x: u32, y: u32, w: u32, h: u32) -> bool {
        let (iw, ih) = self.size();
        let x = x.min(iw);
        let y = y.min(ih);
        let w = w.min(iw - x);
        let h = h.min(ih - y);
        if w == 0 || h == 0 || (x == 0 && y == 0 && w == iw && h == ih) {
            return false;
        }
        self.checkpoint();
        let cropped = imageops::crop_imm(self.state.base.as_ref(), x, y, w, h).to_image();
        self.state.base = Arc::new(cropped);
        let (dx, dy) = (x as f32, y as f32);
        for s in &mut self.state.strokes {
            for p in &mut s.points {
                p[0] -= dx;
                p[1] -= dy;
            }
        }
        true
    }

    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                let cur = std::mem::replace(&mut self.state, prev);
                self.redo.push(cur);
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                let cur = std::mem::replace(&mut self.state, next);
                self.undo.push(cur);
                true
            }
            None => false,
        }
    }

    /// Flatten the base image and all strokes into a new image.
    pub fn render(&self) -> RgbaImage {
        let base = &self.state.base;
        if self.state.strokes.is_empty() {
            return base.as_ref().clone();
        }
        let (w, h) = base.dimensions();

        // tiny-skia works with premultiplied alpha.
        let mut data = base.as_raw().clone();
        for px in data.as_chunks_mut::<4>().0 {
            let a = u32::from(px[3]);
            if a < 255 {
                for c in &mut px[..3] {
                    *c = ((u32::from(*c) * a + 127) / 255) as u8;
                }
            }
        }
        let Some(mut pixmap) = IntSize::from_wh(w, h).and_then(|size| Pixmap::from_vec(data, size)) else {
            return base.as_ref().clone();
        };

        for stroke in &self.state.strokes {
            draw_stroke(&mut pixmap, stroke);
        }

        let mut out = pixmap.take();
        for px in out.as_chunks_mut::<4>().0 {
            let a = u32::from(px[3]);
            if a > 0 && a < 255 {
                for c in &mut px[..3] {
                    *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        RgbaImage::from_raw(w, h, out).expect("pixmap has the same size as the image")
    }
}

fn draw_stroke(pixmap: &mut Pixmap, s: &InkStroke) {
    let mut paint = Paint::default();
    let [r, g, b, a] = s.color;
    paint.set_color_rgba8(r, g, b, a);
    paint.anti_alias = true;
    if s.kind == StrokeKind::Highlighter {
        // Like a real marker: white becomes the marker colour, dark text stays dark.
        paint.blend_mode = BlendMode::Multiply;
    }

    if let [[x, y]] = s.points.as_slice() {
        if let Some(dot) = PathBuilder::from_circle(*x, *y, (s.width * 0.5).max(0.5)) {
            pixmap.fill_path(&dot, &paint, FillRule::Winding, Transform::identity(), None);
        }
        return;
    }

    let mut pb = PathBuilder::new();
    let mut pts = s.points.iter();
    let Some(first) = pts.next() else { return };
    pb.move_to(first[0], first[1]);
    for p in pts {
        pb.line_to(p[0], p[1]);
    }
    let Some(path) = pb.finish() else { return };
    let style = tiny_skia::Stroke {
        width: s.width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Default::default()
    };
    pixmap.stroke_path(&path, &paint, &style, Transform::identity(), None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn white(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255]))
    }

    fn line(kind: StrokeKind, color: [u8; 4]) -> InkStroke {
        InkStroke {
            kind,
            color,
            width: 4.0,
            points: vec![[2.0, 10.0], [18.0, 10.0]],
        }
    }

    #[test]
    fn pen_draws_on_render() {
        let mut doc = Document::new(white(20, 20));
        doc.add_stroke(line(StrokeKind::Pen, [255, 0, 0, 255]));
        let img = doc.render();
        assert_eq!(img.get_pixel(10, 10).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(10, 2).0, [255, 255, 255, 255]);
    }

    #[test]
    fn highlighter_multiplies() {
        let mut base = white(20, 20);
        base.put_pixel(10, 10, Rgba([0, 0, 0, 255]));
        let mut doc = Document::new(base);
        doc.add_stroke(line(StrokeKind::Highlighter, [255, 255, 0, 255]));
        let img = doc.render();
        assert_eq!(img.get_pixel(5, 10).0, [255, 255, 0, 255], "white turns yellow");
        assert_eq!(img.get_pixel(10, 10).0, [0, 0, 0, 255], "black stays black");
    }

    #[test]
    fn crop_shifts_strokes_and_undo_restores() {
        let mut doc = Document::new(white(20, 20));
        doc.add_stroke(line(StrokeKind::Pen, [0, 0, 255, 255]));
        assert!(doc.crop(5, 5, 10, 10));
        assert_eq!(doc.size(), (10, 10));
        assert_eq!(doc.render().get_pixel(5, 5).0, [0, 0, 255, 255]);
        assert!(doc.undo());
        assert_eq!(doc.size(), (20, 20));
        assert!(doc.redo());
        assert_eq!(doc.size(), (10, 10));
    }

    #[test]
    fn eraser_removes_touched_strokes_only() {
        let mut doc = Document::new(white(20, 20));
        doc.add_stroke(line(StrokeKind::Pen, [0, 0, 0, 255]));
        assert!(!doc.erase_at([10.0, 1.0], 1.0, true));
        assert!(doc.erase_at([10.0, 11.0], 1.0, true));
        assert_eq!(doc.render().get_pixel(10, 10).0, [255, 255, 255, 255]);
        assert!(doc.undo());
        assert_eq!(doc.render().get_pixel(10, 10).0, [0, 0, 0, 255]);
    }
}
