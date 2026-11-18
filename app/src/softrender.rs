//! Headless rendering: run the UI in a bare egui context (no window, no
//! GPU) and rasterize the tessellated shapes into pixels, so screenshots
//! work without a display (CI, cloud sandboxes). Used by
//! `catwright --screenshot out.png --headless` and by tests
//! (`softrender::render_ui`). See AGENTS.md "Seeing the UI".
//!
//! Every egui mesh samples the font texture atlas (`TextureId::default()`);
//! solid shapes point at its top-left white pixel (`WHITE_UV`), text glyphs
//! at their coverage value. So one texture lookup covers everything:
//! fragment = vertex color (premultiplied) scaled by the atlas coverage.

use egui::epaint::{ClippedPrimitive, Primitive, Vertex};

/// Run `update` for `frames` frames in a headless context of `width`x`height`
/// points (pixels per point = 1) and rasterize the last frame.
pub fn render(
    mut update: impl FnMut(&egui::Context),
    width: u32,
    height: u32,
    frames: usize,
) -> image::RgbImage {
    let ctx = egui::Context::default();
    crate::fonts::install_fallback_fonts(&ctx);
    crate::theme::apply(&ctx);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(width as f32, height as f32),
        )),
        ..Default::default()
    };
    let mut out = None;
    for _ in 0..frames.max(1) {
        out = Some(ctx.run(input.clone(), |ctx| update(ctx)));
    }
    let out = out.expect("at least one frame ran");
    let ppp = out.pixels_per_point;
    let prims = ctx.tessellate(out.shapes, ppp);
    let atlas = ctx.fonts(|f| f.image().clone());
    rasterize(&prims, width, height, ppp, &atlas)
}

/// Render the harness UI (sidebar, transcript, panels, dialogs) headless.
#[cfg_attr(not(test), allow(dead_code))] // used by the screenshot tests
pub fn render_ui(
    h: &mut crate::harness::Harness,
    width: u32,
    height: u32,
    frames: usize,
) -> image::RgbImage {
    render(
        |ctx| {
            let _ = crate::harness::ui::show(h, ctx);
            crate::harness::ui::show_dialogs(h, ctx);
        },
        width,
        height,
        frames,
    )
}

/// Save an RGB image as PNG.
pub fn save_png(img: &image::RgbImage, path: &std::path::Path) -> Result<(), String> {
    img.save(path).map_err(|e| e.to_string())
}

/// Rasterize tessellated primitives into an RGB framebuffer. Vertex colors
/// are premultiplied sRGBA (epaint), so the blend is `src + dst * (1 - a)`.
fn rasterize(
    prims: &[ClippedPrimitive],
    width: u32,
    height: u32,
    ppp: f32,
    atlas: &egui::FontImage,
) -> image::RgbImage {
    let (aw, ah) = (atlas.size[0], atlas.size[1]);
    let bg = crate::theme::BG;
    let mut fb = vec![[bg.r(), bg.g(), bg.b()]; (width * height) as usize];
    for prim in prims {
        let mesh = match &prim.primitive {
            Primitive::Mesh(m) => m,
            // paint callbacks need a GPU; the app does not use them
            Primitive::Callback(_) => continue,
        };
        let clip = prim.clip_rect * ppp;
        for tri in mesh.indices.as_chunks::<3>().0 {
            let (a, b, c) = (
                &mesh.vertices[tri[0] as usize],
                &mesh.vertices[tri[1] as usize],
                &mesh.vertices[tri[2] as usize],
            );
            fill_triangle(&mut fb, width, height, a, b, c, ppp, atlas, aw, ah, clip);
        }
    }
    let mut flat = Vec::with_capacity(fb.len() * 3);
    for px in &fb {
        flat.extend_from_slice(px);
    }
    image::RgbImage::from_raw(width, height, flat).expect("framebuffer has the right size")
}

/// One triangle: bounding box, barycentric weights, nearest atlas sample.
#[allow(clippy::too_many_arguments)]
fn fill_triangle(
    fb: &mut [[u8; 3]],
    width: u32,
    height: u32,
    a: &Vertex,
    b: &Vertex,
    c: &Vertex,
    ppp: f32,
    atlas: &egui::FontImage,
    aw: usize,
    ah: usize,
    clip: egui::Rect,
) {
    let pa = a.pos * ppp;
    let pb = b.pos * ppp;
    let pc = c.pos * ppp;
    let min_x = pa.x.min(pb.x).min(pc.x).floor().max(clip.left()) as i32;
    let max_x = pa.x.max(pb.x).max(pc.x).ceil().min(clip.right()) as i32;
    let min_y = pa.y.min(pb.y).min(pc.y).floor().max(clip.top()) as i32;
    let max_y = pa.y.max(pb.y).max(pc.y).ceil().min(clip.bottom()) as i32;
    let cross = |u: egui::Vec2, v: egui::Vec2| u.x * v.y - u.y * v.x;
    let d = cross(pb - pa, pc - pa);
    if d.abs() < 1e-9 {
        return;
    }
    for y in min_y..=max_y {
        if y < 0 || y >= height as i32 {
            continue;
        }
        for x in min_x..=max_x {
            if x < 0 || x >= width as i32 {
                continue;
            }
            let p = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
            let w0 = cross(pb - p, pc - p) / d;
            let w1 = cross(pc - p, pa - p) / d;
            let w2 = 1.0 - w0 - w1;
            if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                continue;
            }
            let uv = egui::pos2(
                a.uv.x * w0 + b.uv.x * w1 + c.uv.x * w2,
                a.uv.y * w0 + b.uv.y * w1 + c.uv.y * w2,
            );
            let col = lerp_color(a.color, b.color, c.color, w0, w1, w2);
            // nearest atlas sample; the top-left pixel is fully white, so
            // solid shapes (WHITE_UV) get coverage 1.0
            let tx = ((uv.x * aw as f32) as usize).min(aw - 1);
            let ty = ((uv.y * ah as f32) as usize).min(ah - 1);
            let cov = atlas.pixels[ty * aw + tx];
            let sa = (col.a() as f32 / 255.0) * cov;
            let px = &mut fb[(y as u32 * width + x as u32) as usize];
            let inv = 1.0 - sa;
            // premultiplied over: dst = src + dst * (1 - src.a)
            px[0] = (col.r() as f32 * cov + px[0] as f32 * inv) as u8;
            px[1] = (col.g() as f32 * cov + px[1] as f32 * inv) as u8;
            px[2] = (col.b() as f32 * cov + px[2] as f32 * inv) as u8;
        }
    }
}

/// Barycentric interpolation of a premultiplied color.
fn lerp_color(
    a: egui::Color32,
    b: egui::Color32,
    c: egui::Color32,
    w0: f32,
    w1: f32,
    w2: f32,
) -> egui::Color32 {
    let ch = |get: fn(&egui::Color32) -> u8| {
        (get(&a) as f32 * w0 + get(&b) as f32 * w1 + get(&c) as f32 * w2).clamp(0.0, 255.0) as u8
    };
    egui::Color32::from_rgba_premultiplied(
        ch(|c| c.r()),
        ch(|c| c.g()),
        ch(|c| c.b()),
        ch(|c| c.a()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterizes_text_and_rects() {
        let img = render(
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("Hello Catwright");
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(40.0, 20.0)),
                        0.0,
                        egui::Color32::RED,
                    );
                });
            },
            200,
            100,
            2,
        );
        let red = img.get_pixel(20, 15);
        assert!(red[0] > 200 && red[1] < 60, "the rect is red: {red:?}");
        // some pixel in the label area is darker than the background
        let dark = (0..100).any(|y| {
            (0..200).any(|x| {
                let p = img.get_pixel(x, y);
                p[0] < 100 && p[1] < 100 && p[2] < 100
            })
        });
        assert!(dark, "text glyphs were rasterized");
    }

    #[test]
    fn png_roundtrip() {
        let img = render(
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("png");
                });
            },
            64,
            32,
            1,
        );
        let dir = std::env::temp_dir().join(format!("od-softrender-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.png");
        save_png(&img, &path).unwrap();
        let back = image::open(&path).unwrap().to_rgb8();
        assert_eq!(back.dimensions(), (64, 32));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
