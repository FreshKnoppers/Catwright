//! Software renderer for egui: rasterizes the tessellated meshes of a frame
//! on the CPU into an RGBA image. No window, no display server, no GPU, so it
//! works in sandboxes without X11/Wayland. Used by `--screenshot <png>
//! --headless` and by tests that want to look at a view (`render_ui`).
//!
//! It mirrors egui_glow: vertex colours and textures are premultiplied sRGBA
//! and are multiplied and blended in gamma space (`src + dst * (1 - src.a)`),
//! the font atlas comes from `FontImage::srgba_pixels(None)`, and clip
//! rectangles are rounded to whole pixels. Anti-aliasing comes from egui's
//! feathered meshes, so one sample per pixel centre is enough.

use egui::epaint::textures::TextureFilter;
use egui::epaint::{ClippedPrimitive, ImageData, Primitive, Vertex};
use egui::{Color32, Pos2, TextureId, TexturesDelta};
use std::collections::HashMap;

/// One texture in premultiplied sRGBA, 0..1 per channel.
struct Texture {
    width: usize,
    height: usize,
    pixels: Vec<[f32; 4]>,
    linear: bool,
}

impl Texture {
    fn texel(&self, x: usize, y: usize) -> [f32; 4] {
        self.pixels[y.min(self.height - 1) * self.width + x.min(self.width - 1)]
    }

    /// Sample at `uv` (0..1), clamped to the edge like egui's default.
    fn sample(&self, uv: Pos2) -> [f32; 4] {
        let x = uv.x * self.width as f32 - 0.5;
        let y = uv.y * self.height as f32 - 0.5;
        if !self.linear {
            let xi = x.round().max(0.0) as usize;
            let yi = y.round().max(0.0) as usize;
            return self.texel(xi, yi);
        }
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let xi = x0.max(0.0) as usize;
        let yi = y0.max(0.0) as usize;
        let xj = if x0 < 0.0 { 0 } else { xi + 1 };
        let yj = if y0 < 0.0 { 0 } else { yi + 1 };
        let (a, b) = (self.texel(xi, yi), self.texel(xj, yi));
        let (c, d) = (self.texel(xi, yj), self.texel(xj, yj));
        let mut out = [0.0; 4];
        for k in 0..4 {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bottom = c[k] + (d[k] - c[k]) * fx;
            out[k] = top + (bottom - top) * fy;
        }
        out
    }
}

fn unit(c: Color32) -> [f32; 4] {
    [
        c.r() as f32 / 255.0,
        c.g() as f32 / 255.0,
        c.b() as f32 / 255.0,
        c.a() as f32 / 255.0,
    ]
}

/// The textures egui uploaded (font atlas, images), kept across frames like
/// a GPU would keep them.
#[derive(Default)]
pub struct Textures {
    map: HashMap<TextureId, Texture>,
}

impl Textures {
    /// Apply the new and changed textures of a frame (before painting).
    pub fn set(&mut self, delta: &TexturesDelta) {
        for (id, d) in &delta.set {
            let ([w, h], pixels): ([usize; 2], Vec<[f32; 4]>) = match &d.image {
                ImageData::Color(img) => (img.size, img.pixels.iter().map(|&c| unit(c)).collect()),
                ImageData::Font(img) => (img.size, img.srgba_pixels(None).map(unit).collect()),
            };
            let linear = d.options.magnification == TextureFilter::Linear;
            match (d.pos, self.map.get_mut(id)) {
                (Some([x0, y0]), Some(tex)) => {
                    for y in 0..h {
                        for x in 0..w {
                            let (tx, ty) = (x0 + x, y0 + y);
                            if tx < tex.width && ty < tex.height {
                                tex.pixels[ty * tex.width + tx] = pixels[y * w + x];
                            }
                        }
                    }
                }
                _ => {
                    self.map.insert(
                        *id,
                        Texture {
                            width: w.max(1),
                            height: h.max(1),
                            pixels: if pixels.is_empty() {
                                vec![[1.0; 4]]
                            } else {
                                pixels
                            },
                            linear,
                        },
                    );
                }
            }
        }
    }

    /// Free the textures a frame released (after painting).
    pub fn free(&mut self, delta: &TexturesDelta) {
        for id in &delta.free {
            self.map.remove(id);
        }
    }
}

/// Edge function: twice the signed area of (a, b, p).
fn edge(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

/// Rasterize the primitives of one frame into a `width` × `height` image
/// (physical pixels) over an opaque `background`.
pub fn rasterize(
    primitives: &[ClippedPrimitive],
    textures: &Textures,
    pixels_per_point: f32,
    [width, height]: [usize; 2],
    background: Color32,
) -> image::RgbaImage {
    let mut buf = vec![unit(background); width * height];
    for prim in primitives {
        let Primitive::Mesh(mesh) = &prim.primitive else {
            continue; // paint callbacks need a GPU; the app has none
        };
        let tex = textures.map.get(&mesh.texture_id);
        // clip rect in whole pixels, as egui_glow's scissor does
        let clip = prim.clip_rect;
        let cx0 = ((clip.min.x * pixels_per_point).round().max(0.0) as usize).min(width);
        let cy0 = ((clip.min.y * pixels_per_point).round().max(0.0) as usize).min(height);
        let cx1 = ((clip.max.x * pixels_per_point).round().max(0.0) as usize).min(width);
        let cy1 = ((clip.max.y * pixels_per_point).round().max(0.0) as usize).min(height);
        if cx0 >= cx1 || cy0 >= cy1 {
            continue;
        }
        for tri in mesh.indices.as_chunks::<3>().0 {
            let v = [
                &mesh.vertices[tri[0] as usize],
                &mesh.vertices[tri[1] as usize],
                &mesh.vertices[tri[2] as usize],
            ];
            triangle(
                &mut buf,
                width,
                [cx0, cy0, cx1, cy1],
                pixels_per_point,
                v,
                tex,
            );
        }
    }
    let mut img = image::RgbaImage::new(width as u32, height as u32);
    for (px, c) in img.pixels_mut().zip(&buf) {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        *px = image::Rgba([q(c[0]), q(c[1]), q(c[2]), q(c[3])]);
    }
    img
}

fn triangle(
    buf: &mut [[f32; 4]],
    width: usize,
    [cx0, cy0, cx1, cy1]: [usize; 4],
    ppp: f32,
    v: [&Vertex; 3],
    tex: Option<&Texture>,
) {
    let p = v.map(|v| Pos2::new(v.pos.x * ppp, v.pos.y * ppp));
    let area = edge(p[0], p[1], p[2]);
    if area.abs() < 1e-8 {
        return;
    }
    let min_x = p[0].x.min(p[1].x).min(p[2].x).floor().max(cx0 as f32) as usize;
    let max_x = (p[0].x.max(p[1].x).max(p[2].x).ceil().max(0.0) as usize).min(cx1);
    let min_y = p[0].y.min(p[1].y).min(p[2].y).floor().max(cy0 as f32) as usize;
    let max_y = (p[0].y.max(p[1].y).max(p[2].y).ceil().max(0.0) as usize).min(cy1);
    let colors = v.map(|v| unit(v.color));
    // a triangle with one colour and one uv (rects, lines) skips the
    // interpolation; most of the UI is drawn like that
    let flat = v[0].color == v[1].color
        && v[1].color == v[2].color
        && v[0].uv == v[1].uv
        && v[1].uv == v[2].uv;
    let flat_src = flat.then(|| shade(colors[0], v[0].uv, tex));
    for y in min_y..max_y {
        for x in min_x..max_x {
            let c = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
            // barycentric weights; the top-left rule keeps shared edges
            // from being painted twice
            let mut w = [
                edge(p[1], p[2], c),
                edge(p[2], p[0], c),
                edge(p[0], p[1], c),
            ];
            if area < 0.0 {
                w = w.map(|e| -e);
            }
            let edges = [(p[1], p[2]), (p[2], p[0]), (p[0], p[1])];
            let inside = w
                .iter()
                .zip(edges)
                .all(|(&e, (a, b))| e > 0.0 || (e == 0.0 && top_left(a, b, area < 0.0)));
            if !inside {
                continue;
            }
            let src = match flat_src {
                Some(s) => s,
                None => {
                    let total = area.abs();
                    let l = w.map(|e| e / total);
                    let mut col = [0.0; 4];
                    for k in 0..4 {
                        col[k] = colors[0][k] * l[0] + colors[1][k] * l[1] + colors[2][k] * l[2];
                    }
                    let uv = Pos2::new(
                        v[0].uv.x * l[0] + v[1].uv.x * l[1] + v[2].uv.x * l[2],
                        v[0].uv.y * l[0] + v[1].uv.y * l[1] + v[2].uv.y * l[2],
                    );
                    shade(col, uv, tex)
                }
            };
            let dst = &mut buf[y * width + x];
            let keep = 1.0 - src[3];
            for k in 0..4 {
                dst[k] = src[k] + dst[k] * keep;
            }
        }
    }
}

/// Vertex colour times texel, both premultiplied (egui_glow's shader).
fn shade(color: [f32; 4], uv: Pos2, tex: Option<&Texture>) -> [f32; 4] {
    match tex {
        Some(t) => {
            let s = t.sample(uv);
            [
                color[0] * s[0],
                color[1] * s[1],
                color[2] * s[2],
                color[3] * s[3],
            ]
        }
        None => color,
    }
}

/// Top-left fill rule for the edge a→b (y grows downwards). `flipped`: the
/// triangle is wound the other way, so the edge's direction is reversed.
fn top_left(a: Pos2, b: Pos2, flipped: bool) -> bool {
    let (a, b) = if flipped { (b, a) } else { (a, b) };
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    (dy == 0.0 && dx < 0.0) || dy > 0.0
}

/// Runs `frames` frames of `ui` on a fresh context with the app's fonts and
/// theme at `size` points (scale 1) and returns the last frame as an image.
/// For tests: `render_ui([1360, 860], 3, |ctx| { … }).save(path)` shows what
/// a view really looks like without a display.
#[cfg(test)]
pub fn render_ui(
    [w, h]: [u32; 2],
    frames: usize,
    mut ui: impl FnMut(&egui::Context),
) -> image::RgbaImage {
    let ctx = egui::Context::default();
    crate::fonts::install_fallback_fonts(&ctx);
    crate::theme::apply(&ctx);
    let mut textures = Textures::default();
    let mut last = None;
    for i in 0..frames.max(1) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(w as f32, h as f32),
            )),
            time: Some(i as f64 / 60.0),
            ..Default::default()
        };
        let out = ctx.run(input, |ctx| ui(ctx));
        textures.set(&out.textures_delta);
        if i + 1 == frames.max(1) {
            let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
            last = Some(rasterize(
                &prims,
                &textures,
                out.pixels_per_point,
                [w as usize, h as usize],
                crate::theme::BG,
            ));
        }
        textures.free(&out.textures_delta);
    }
    last.expect("at least one frame ran")
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::epaint::Mesh;

    fn quad(rect: egui::Rect, color: Color32) -> ClippedPrimitive {
        let mut mesh = Mesh::default();
        mesh.add_colored_rect(rect, color);
        ClippedPrimitive {
            clip_rect: egui::Rect::EVERYTHING,
            primitive: Primitive::Mesh(mesh),
        }
    }

    fn white_texture() -> Textures {
        // rects sample the font atlas's white texel; a 1×1 white texture
        // answers every uv the same way
        let mut t = Textures::default();
        t.map.insert(
            TextureId::default(),
            Texture {
                width: 1,
                height: 1,
                pixels: vec![[1.0; 4]],
                linear: true,
            },
        );
        t
    }

    #[test]
    fn a_rect_fills_exactly_its_pixels() {
        let r = egui::Rect::from_min_max(egui::pos2(2.0, 3.0), egui::pos2(6.0, 5.0));
        let img = rasterize(
            &[quad(r, Color32::from_rgb(250, 80, 15))],
            &white_texture(),
            1.0,
            [10, 8],
            Color32::BLACK,
        );
        let mut filled = 0;
        for (x, y, px) in img.enumerate_pixels() {
            let inside = (2..6).contains(&x) && (3..5).contains(&y);
            if inside {
                assert_eq!(px.0, [250, 80, 15, 255], "{x},{y}");
                filled += 1;
            } else {
                assert_eq!(px.0, [0, 0, 0, 255], "{x},{y}");
            }
        }
        // the two triangles share a diagonal: no pixel is painted twice,
        // none is missed
        assert_eq!(filled, 8);
    }

    #[test]
    fn translucent_colours_blend_premultiplied_and_clip() {
        let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(4.0, 4.0));
        // 50 % white, premultiplied: (128,128,128,128)
        let mut half = quad(full, Color32::from_rgba_premultiplied(128, 128, 128, 128));
        half.clip_rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(2.0, 4.0));
        let img = rasterize(&[half], &white_texture(), 1.0, [4, 4], Color32::BLACK);
        assert_eq!(img.get_pixel(0, 0).0, [128, 128, 128, 255]);
        // right of the clip rect nothing is painted
        assert_eq!(img.get_pixel(3, 0).0, [0, 0, 0, 255]);
    }

    #[test]
    fn scale_two_doubles_the_pixels() {
        let r = egui::Rect::from_min_max(egui::pos2(1.0, 1.0), egui::pos2(2.0, 2.0));
        let img = rasterize(
            &[quad(r, Color32::WHITE)],
            &white_texture(),
            2.0,
            [6, 6],
            Color32::BLACK,
        );
        let white = img.pixels().filter(|p| p.0 == [255, 255, 255, 255]).count();
        assert_eq!(white, 4);
        assert_eq!(img.get_pixel(2, 2).0, [255, 255, 255, 255]);
    }

    #[test]
    fn a_real_ui_frame_draws_text_from_the_font_atlas() {
        let img = render_ui([200, 60], 2, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("Catwright")
                        .size(24.0)
                        .color(Color32::WHITE),
                );
            });
        });
        assert_eq!(img.dimensions(), (200, 60));
        // glyphs are textured from the font atlas: bright pixels on the
        // dark background, and anti-aliased edges in between
        let bright = img.pixels().filter(|p| p.0[0] > 200).count();
        let edges = img.pixels().filter(|p| (60..200).contains(&p.0[0])).count();
        assert!(bright > 50, "text renders: {bright} bright pixels");
        assert!(edges > 20, "glyph edges are smooth: {edges}");
    }
}
