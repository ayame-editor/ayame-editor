//! The Ayame flower mark rendered to raw RGBA, shared by the native window
//! icon (`gui.rs`). Pure math with no GUI dependencies so the shape — most
//! importantly its aspect ratio — stays unit-testable in every build.

/// One petal ellipse: (cx, cy, rx, ry, rotation radians, r, g, b).
type Petal = (f32, f32, f32, f32, f32, u8, u8, u8);

/// Canvas edge in pixels.
pub const ICON_SIZE: u32 = 64;

/// The window/taskbar icon mirrors `web/favicon.svg`: the purple Ayame Editor
/// flower mark on a transparent background, drawn to RGBA so the native GUI
/// stays dependency-free.
///
/// The petal table is inherited from the favicon artwork, whose raw silhouette
/// is noticeably taller than wide (~36x52 on the 64px canvas). The favicon
/// corrects that in SVG with a centered `scale(1.1 .9)`; without the same
/// correction the titlebar icon looks vertically stretched at 16x16 (issue
/// #51). Sample points are therefore pulled through the inverse of a centered
/// anisotropic scale, chosen so the painted bounding box comes out ~50x49 —
/// square within a pixel — and bolder, which also survives the shrink to
/// 16x16 much better.
const SCALE_X: f32 = 1.40;
const SCALE_Y: f32 = 0.95;
/// Nudges the widened mark down so top/bottom margins match.
const OFFSET_Y: f32 = 1.5;

pub fn app_icon_rgba() -> Vec<u8> {
    const N: u32 = ICON_SIZE;
    let mut px = vec![0u8; (N * N * 4) as usize];

    // Ordered back-to-front.
    let petals: [Petal; 6] = [
        (32.0, 18.0, 6.0, 13.2, 0.0, 0xA9, 0x92, 0xE0),
        (23.0, 27.0, 5.8, 13.0, -0.58, 0x9B, 0x82, 0xD8),
        (41.0, 27.0, 5.8, 13.0, 0.58, 0x79, 0x5F, 0xC3),
        (24.0, 43.0, 6.2, 13.5, 0.47, 0x8E, 0x73, 0xCF),
        (40.0, 43.0, 6.2, 13.5, -0.47, 0x6F, 0x56, 0xB8),
        (32.0, 38.0, 6.4, 11.8, 0.0, 0x67, 0x4F, 0xAF),
    ];

    let in_ellipse = |x: f32, y: f32, cx: f32, cy: f32, rx: f32, ry: f32, rot: f32| {
        let (sin, cos) = rot.sin_cos();
        let dx0 = x - cx;
        let dy0 = y - cy;
        let dx = (dx0 * cos + dy0 * sin) / rx;
        let dy = (-dx0 * sin + dy0 * cos) / ry;
        dx * dx + dy * dy <= 1.0
    };

    for y in 0..N {
        for x in 0..N {
            let mut dst = [0.0f32; 4];
            for &(cx, cy, rx, ry, rot, r, g, b) in &petals {
                let mut hits = 0.0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let fx = x as f32 + (sx as f32 + 0.5) / 4.0;
                        let fy = y as f32 + (sy as f32 + 0.5) / 4.0;
                        // Inverse of the centered widening transform above.
                        let gx = 32.0 + (fx - 32.0) / SCALE_X;
                        let gy = 32.0 + (fy - 32.0 - OFFSET_Y) / SCALE_Y;
                        if in_ellipse(gx, gy, cx, cy, rx, ry, rot) {
                            hits += 1.0;
                        }
                    }
                }
                let src_a = hits / 16.0;
                if src_a <= 0.0 {
                    continue;
                }
                let src = [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0];
                let out_a = src_a + dst[3] * (1.0 - src_a);
                for c in 0..3 {
                    dst[c] = (src[c] * src_a + dst[c] * dst[3] * (1.0 - src_a)) / out_a;
                }
                dst[3] = out_a;
            }

            let i = ((y * N + x) * 4) as usize;
            px[i] = (dst[0] * 255.0).round() as u8;
            px[i + 1] = (dst[1] * 255.0).round() as u8;
            px[i + 2] = (dst[2] * 255.0).round() as u8;
            px[i + 3] = (dst[3] * 255.0).round() as u8;
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bounding box of pixels with non-negligible alpha.
    fn painted_bbox(px: &[u8]) -> (u32, u32, u32, u32) {
        let n = ICON_SIZE;
        let (mut x0, mut x1, mut y0, mut y1) = (n, 0, n, 0);
        for y in 0..n {
            for x in 0..n {
                if px[((y * n + x) * 4 + 3) as usize] > 8 {
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                    y0 = y0.min(y);
                    y1 = y1.max(y);
                }
            }
        }
        assert!(x0 <= x1 && y0 <= y1, "icon is fully transparent");
        (x0, x1, y0, y1)
    }

    /// Issue #51: the mark must stay near-square so the 16x16 titlebar
    /// rendering does not look vertically stretched, and it must keep filling
    /// most of the canvas so it stays legible when shrunk.
    #[test]
    fn mark_is_square_and_fills_canvas() {
        let px = app_icon_rgba();
        let (x0, x1, y0, y1) = painted_bbox(&px);
        let w = (x1 - x0 + 1) as f32;
        let h = (y1 - y0 + 1) as f32;
        let aspect = w / h;
        assert!(
            (0.9..=1.12).contains(&aspect),
            "flower mark aspect drifted from square: {w}x{h} = {aspect:.2}"
        );
        assert!(
            w >= ICON_SIZE as f32 * 0.7 && h >= ICON_SIZE as f32 * 0.7,
            "flower mark shrank too small for titlebar sizes: {w}x{h}"
        );
    }
}
