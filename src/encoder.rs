//! Frame scaling, tile based delta detection and JPEG encoding.
//!
//! The host used to send a full JPEG of the whole screen for every frame.
//! Most of a desktop does not change between two frames, so we now split the
//! (downscaled) frame into a grid of tiles, compare each tile with the
//! previous frame, merge the changed tiles into rectangles and only encode
//! those. A full frame ("keyframe") is only sent when the session starts, when
//! the resolution changes or when so much of the screen changed that sending
//! everything is cheaper than sending many rectangles.

use crate::proto::{Msg, Tile};

/// Edge length of one comparison tile in (downscaled) pixels.
pub const TILE: u32 = 64;
/// JPEG quality for full frames.
pub const FULL_QUALITY: u8 = 62;
/// Tiles are small, so we can afford a bit more quality for crisp text.
pub const TILE_QUALITY: u8 = 72;
/// If more than this fraction of the frame changed, send a full frame.
pub const KEYFRAME_RATIO: f32 = 0.6;
/// Safety valve: never send more rectangles than this, fall back to a keyframe.
pub const MAX_RECTS: usize = 96;

/// Box-filter downscale straight from RGBA into RGB.
#[allow(dead_code)]
pub fn scale_to_rgb(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    scale_to_rgb_ex(src, sw, sh, dw, dh, false)
}

/// Box-filter downscale straight from RGBA/BGRA into RGB.
///
/// Doing scaling and the conversion in a single pass avoids one full
/// intermediate buffer per frame (the old path did `thumbnail()` and then a
/// separate `rgba_to_rgb()`). `bgra = true` additionally swaps red and blue,
/// which is the pixel order the DXGI duplication API hands us.
pub fn scale_to_rgb_ex(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32, bgra: bool) -> Vec<u8> {
    let mut out = vec![0u8; dw as usize * dh as usize * 3];
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return out;
    }
    let (ri, bi) = if bgra { (2usize, 0usize) } else { (0usize, 2usize) };
    if dw == sw && dh == sh {
        for (o, px) in out.chunks_exact_mut(3).zip(src.chunks_exact(4)) {
            o[0] = px[ri];
            o[1] = px[1];
            o[2] = px[bi];
        }
        return out;
    }

    let sstride = sw as usize * 4;
    let mut cols: Vec<(usize, usize)> = Vec::with_capacity(dw as usize);
    for dx in 0..dw as u64 {
        let x0 = (dx * sw as u64 / dw as u64) as usize;
        let mut x1 = ((dx + 1) * sw as u64 / dw as u64) as usize;
        if x1 <= x0 {
            x1 = x0 + 1;
        }
        if x1 > sw as usize {
            x1 = sw as usize;
        }
        cols.push((x0, x1));
    }

    for dy in 0..dh as u64 {
        let y0 = (dy * sh as u64 / dh as u64) as usize;
        let mut y1 = ((dy + 1) * sh as u64 / dh as u64) as usize;
        if y1 <= y0 {
            y1 = y0 + 1;
        }
        if y1 > sh as usize {
            y1 = sh as usize;
        }
        let orow = dy as usize * dw as usize * 3;
        for (dx, &(x0, x1)) in cols.iter().enumerate() {
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for y in y0..y1 {
                let a = y * sstride + x0 * 4;
                let e = y * sstride + x1 * 4;
                if e > src.len() {
                    break;
                }
                for px in src[a..e].chunks_exact(4) {
                    r += px[ri] as u32;
                    g += px[1] as u32;
                    b += px[bi] as u32;
                    n += 1;
                }
            }
            if n == 0 {
                n = 1;
            }
            let o = orow + dx * 3;
            out[o] = (r / n) as u8;
            out[o + 1] = (g / n) as u8;
            out[o + 2] = (b / n) as u8;
        }
    }
    out
}

/// Tiles that differ between `cur` and `prev`, merged into pixel rectangles.
pub fn dirty_rects(cur: &[u8], prev: &[u8], w: u32, h: u32) -> Vec<(u32, u32, u32, u32)> {
    let stride = w as usize * 3;
    if cur.len() != prev.len() || cur.len() < stride * h as usize {
        return vec![(0, 0, w, h)];
    }
    let cols = w.div_ceil(TILE);
    let rows = h.div_ceil(TILE);
    let mut grid = vec![false; (cols * rows) as usize];

    for ty in 0..rows {
        let y0 = ty * TILE;
        let y1 = ((ty + 1) * TILE).min(h);
        for tx in 0..cols {
            let x0 = tx * TILE;
            let x1 = ((tx + 1) * TILE).min(w);
            let n = (x1 - x0) as usize * 3;
            let mut changed = false;
            for y in y0..y1 {
                let a = y as usize * stride + x0 as usize * 3;
                // slice equality compiles to memcmp
                if cur[a..a + n] != prev[a..a + n] {
                    changed = true;
                    break;
                }
            }
            grid[(ty * cols + tx) as usize] = changed;
        }
    }
    merge_grid(&grid, cols, rows, w, h)
}

/// Merges the boolean dirty grid into as few rectangles as possible:
/// horizontal runs per tile row, then vertically stacked runs with an
/// identical column span.
fn merge_grid(grid: &[bool], cols: u32, rows: u32, w: u32, h: u32) -> Vec<(u32, u32, u32, u32)> {
    let mut done: Vec<(u32, u32, u32, u32)> = Vec::new();
    let mut open: Vec<(u32, u32, u32, u32)> = Vec::new();

    for ry in 0..rows {
        let mut runs: Vec<(u32, u32)> = Vec::new();
        let mut cx = 0;
        while cx < cols {
            if grid[(ry * cols + cx) as usize] {
                let start = cx;
                while cx < cols && grid[(ry * cols + cx) as usize] {
                    cx += 1;
                }
                runs.push((start, cx - start));
            } else {
                cx += 1;
            }
        }
        let mut next_open: Vec<(u32, u32, u32, u32)> = Vec::new();
        for (start, len) in runs {
            match open
                .iter()
                .position(|r| r.0 == start && r.2 == len && r.1 + r.3 == ry)
            {
                Some(pos) => {
                    let mut r = open.remove(pos);
                    r.3 += 1;
                    next_open.push(r);
                }
                None => next_open.push((start, ry, len, 1)),
            }
        }
        done.append(&mut open);
        open = next_open;
    }
    done.append(&mut open);

    done.into_iter()
        .map(|(tx, ty, tw, th)| {
            let x = tx * TILE;
            let y = ty * TILE;
            let ww = (tw * TILE).min(w.saturating_sub(x));
            let hh = (th * TILE).min(h.saturating_sub(y));
            (x, y, ww, hh)
        })
        .filter(|r| r.2 > 0 && r.3 > 0)
        .collect()
}

/// Copies a rectangle out of a RGB buffer into its own contiguous buffer.
pub fn crop_rgb(src: &[u8], w: u32, x: u32, y: u32, cw: u32, ch: u32) -> Vec<u8> {
    let stride = w as usize * 3;
    let mut out = Vec::with_capacity(cw as usize * ch as usize * 3);
    for yy in y..y + ch {
        let a = yy as usize * stride + x as usize * 3;
        let e = a + cw as usize * 3;
        if e > src.len() {
            break;
        }
        out.extend_from_slice(&src[a..e]);
    }
    out
}

pub fn jpeg_rgb(rgb: &[u8], w: u32, h: u32, quality: u8) -> Option<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::with_capacity(rgb.len() / 8 + 512);
    {
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
        enc.encode(rgb, w, h, image::ExtendedColorType::Rgb8).ok()?;
    }
    Some(buf)
}

/// Draws a decoded RGB rectangle into the viewer's RGBA canvas.
pub fn blit_rgb_to_rgba(
    dst: &mut [u8],
    dw: u32,
    dh: u32,
    x: u32,
    y: u32,
    src: &[u8],
    sw: u32,
    sh: u32,
) -> bool {
    if sw == 0 || sh == 0 || x + sw > dw || y + sh > dh {
        return false;
    }
    if src.len() < sw as usize * sh as usize * 3 || dst.len() < dw as usize * dh as usize * 4 {
        return false;
    }
    let dstride = dw as usize * 4;
    for row in 0..sh as usize {
        let d0 = (y as usize + row) * dstride + x as usize * 4;
        let s0 = row * sw as usize * 3;
        for i in 0..sw as usize {
            let d = d0 + i * 4;
            let s = s0 + i * 3;
            dst[d] = src[s];
            dst[d + 1] = src[s + 1];
            dst[d + 2] = src[s + 2];
            dst[d + 3] = 255;
        }
    }
    true
}

pub struct EncodeResult {
    pub msg: Option<Msg>,
    pub keyframe: bool,
    #[allow(dead_code)]
    pub rects: usize,
    pub dirty: f32,
    pub bytes: usize,
}

impl EncodeResult {
    fn nothing() -> Self {
        Self {
            msg: None,
            keyframe: false,
            rects: 0,
            dirty: 0.0,
            bytes: 0,
        }
    }
}

/// Keeps the previously sent frame so the next one can be diffed against it.
pub struct Delta {
    prev: Vec<u8>,
    w: u32,
    h: u32,
    full_q: u8,
    tile_q: u8,
}

impl Default for Delta {
    fn default() -> Self {
        Self {
            prev: Vec::new(),
            w: 0,
            h: 0,
            full_q: FULL_QUALITY,
            tile_q: TILE_QUALITY,
        }
    }
}

impl Delta {
    pub fn new() -> Self {
        Self::default()
    }

    /// Game mode trades sharpness for bandwidth, remote maintenance does the
    /// opposite - so the quality is switchable at runtime.
    pub fn set_quality(&mut self, full: u8, tile: u8) {
        if full != self.full_q || tile != self.tile_q {
            self.full_q = full;
            self.tile_q = tile;
            self.reset();
        }
    }

    pub fn reset(&mut self) {
        self.prev.clear();
        self.w = 0;
        self.h = 0;
    }

    /// Always emits a complete frame (used for the first frame and as fallback).
    pub fn encode_full(&mut self, rgb: &[u8], w: u32, h: u32) -> EncodeResult {
        let jpeg = match jpeg_rgb(rgb, w, h, self.full_q) {
            Some(j) => j,
            None => return EncodeResult::nothing(),
        };
        self.prev.clear();
        self.prev.extend_from_slice(rgb);
        self.w = w;
        self.h = h;
        let bytes = jpeg.len();
        EncodeResult {
            msg: Some(Msg::Frame {
                width: w,
                height: h,
                jpeg,
            }),
            keyframe: true,
            rects: 1,
            dirty: 1.0,
            bytes,
        }
    }

    pub fn encode(&mut self, rgb: &[u8], w: u32, h: u32) -> EncodeResult {
        if self.w != w || self.h != h || self.prev.len() != rgb.len() {
            return self.encode_full(rgb, w, h);
        }
        let rects = dirty_rects(rgb, &self.prev, w, h);
        if rects.is_empty() {
            return EncodeResult::nothing();
        }
        let area: u64 = rects.iter().map(|r| r.2 as u64 * r.3 as u64).sum();
        let dirty = area as f32 / (w as f32 * h as f32).max(1.0);
        if dirty > KEYFRAME_RATIO || rects.len() > MAX_RECTS {
            let mut res = self.encode_full(rgb, w, h);
            res.dirty = dirty;
            return res;
        }

        let mut tiles: Vec<Tile> = Vec::with_capacity(rects.len());
        let mut bytes = 0usize;
        for (x, y, cw, ch) in rects {
            let crop = crop_rgb(rgb, w, x, y, cw, ch);
            if let Some(jpeg) = jpeg_rgb(&crop, cw, ch, self.tile_q) {
                bytes += jpeg.len();
                tiles.push(Tile {
                    x,
                    y,
                    w: cw,
                    h: ch,
                    jpeg,
                });
            }
        }
        if tiles.is_empty() {
            return EncodeResult::nothing();
        }
        self.prev.copy_from_slice(rgb);
        let n = tiles.len();
        EncodeResult {
            msg: Some(Msg::Tiles {
                width: w,
                height: h,
                tiles,
            }),
            keyframe: false,
            rects: n,
            dirty,
            bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_rgb(w: u32, h: u32, c: u8) -> Vec<u8> {
        vec![c; (w * h * 3) as usize]
    }

    #[test]
    fn scale_identity_drops_alpha() {
        let rgba = vec![1, 2, 3, 255, 4, 5, 6, 255];
        let out = scale_to_rgb(&rgba, 2, 1, 2, 1);
        assert_eq!(out, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn bgra_channels_are_swapped() {
        // one blue pixel in BGRA (B=255) must come out as RGB 0,0,255
        let bgra = vec![255u8, 0, 0, 255];
        let out = scale_to_rgb_ex(&bgra, 1, 1, 1, 1, true);
        assert_eq!(out, vec![0, 0, 255]);
        // and the downscaling path must swap as well
        let bgra4 = vec![255u8, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255];
        let out2 = scale_to_rgb_ex(&bgra4, 2, 2, 1, 1, true);
        assert_eq!(out2, vec![0, 0, 255]);
    }

    #[test]
    fn scale_averages_a_block() {
        // 2x2 white/black checkerboard -> one grey pixel
        let mut rgba = Vec::new();
        for v in [255u8, 0, 0, 255] {
            rgba.extend_from_slice(&[v, v, v, 255]);
        }
        let out = scale_to_rgb(&rgba, 2, 2, 1, 1);
        assert_eq!(out.len(), 3);
        assert!((127..=128).contains(&out[0]), "got {}", out[0]);
    }

    #[test]
    fn no_change_means_no_rects() {
        let a = solid_rgb(256, 128, 9);
        assert!(dirty_rects(&a, &a.clone(), 256, 128).is_empty());
    }

    #[test]
    fn single_pixel_change_yields_one_tile() {
        let a = solid_rgb(256, 128, 9);
        let mut b = a.clone();
        // pixel (70, 70) -> tile column 1, tile row 1
        let idx = (70 * 256 + 70) * 3;
        b[idx] = 200;
        let r = dirty_rects(&b, &a, 256, 128);
        assert_eq!(r, vec![(64, 64, 64, 64)]);
    }

    #[test]
    fn adjacent_tiles_merge_into_one_rect() {
        let a = solid_rgb(256, 128, 9);
        let mut b = a.clone();
        for y in 0..128usize {
            for x in 0..256usize {
                b[(y * 256 + x) * 3 + 1] = 77;
            }
        }
        let r = dirty_rects(&b, &a, 256, 128);
        assert_eq!(r, vec![(0, 0, 256, 128)]);
    }

    #[test]
    fn edge_tiles_are_clipped_to_the_frame() {
        // 100x70 -> 2x2 tiles, the outer ones are partial
        let a = solid_rgb(100, 70, 0);
        let b = solid_rgb(100, 70, 255);
        let r = dirty_rects(&b, &a, 100, 70);
        assert_eq!(r, vec![(0, 0, 100, 70)]);
    }

    #[test]
    fn crop_and_blit_roundtrip() {
        let mut src = solid_rgb(8, 8, 0);
        for y in 0..8usize {
            for x in 0..8usize {
                src[(y * 8 + x) * 3] = (y * 8 + x) as u8;
            }
        }
        let crop = crop_rgb(&src, 8, 2, 3, 4, 2);
        assert_eq!(crop.len(), 4 * 2 * 3);
        assert_eq!(crop[0], (3 * 8 + 2) as u8);

        let mut canvas = vec![0u8; 8 * 8 * 4];
        assert!(blit_rgb_to_rgba(&mut canvas, 8, 8, 2, 3, &crop, 4, 2));
        assert_eq!(canvas[((3 * 8) + 2) * 4], (3 * 8 + 2) as u8);
        assert_eq!(canvas[((3 * 8) + 2) * 4 + 3], 255);
        // out of bounds must be rejected, not panic
        assert!(!blit_rgb_to_rgba(&mut canvas, 8, 8, 6, 3, &crop, 4, 2));
    }

    #[test]
    fn delta_first_frame_is_a_keyframe_then_quiet() {
        let mut d = Delta::new();
        let a = solid_rgb(128, 128, 40);
        let r1 = d.encode(&a, 128, 128);
        assert!(r1.keyframe && r1.msg.is_some());
        let r2 = d.encode(&a, 128, 128);
        assert!(r2.msg.is_none(), "identical frame must produce nothing");
    }

    #[test]
    fn delta_small_change_sends_tiles_not_a_keyframe() {
        let mut d = Delta::new();
        let a = solid_rgb(256, 256, 40);
        d.encode(&a, 256, 256);
        let mut b = a.clone();
        for y in 0..20usize {
            for x in 0..20usize {
                b[(y * 256 + x) * 3] = 200;
            }
        }
        let r = d.encode(&b, 256, 256);
        assert!(!r.keyframe);
        assert_eq!(r.rects, 1);
        match r.msg {
            Some(Msg::Tiles { tiles, .. }) => {
                assert_eq!(tiles.len(), 1);
                assert_eq!((tiles[0].x, tiles[0].y, tiles[0].w, tiles[0].h), (0, 0, 64, 64));
            }
            _ => panic!("expected tiles"),
        }
    }

    #[test]
    fn delta_big_change_falls_back_to_keyframe() {
        let mut d = Delta::new();
        let a = solid_rgb(256, 256, 40);
        d.encode(&a, 256, 256);
        let b = solid_rgb(256, 256, 200);
        let r = d.encode(&b, 256, 256);
        assert!(r.keyframe);
        assert!(matches!(r.msg, Some(Msg::Frame { .. })));
    }

    #[test]
    fn delta_survives_a_resolution_change() {
        let mut d = Delta::new();
        d.encode(&solid_rgb(128, 128, 1), 128, 128);
        let r = d.encode(&solid_rgb(64, 64, 1), 64, 64);
        assert!(r.keyframe);
    }

    #[test]
    fn quality_switch_forces_a_fresh_keyframe() {
        let mut d = Delta::new();
        let a = solid_rgb(128, 128, 40);
        assert!(d.encode(&a, 128, 128).keyframe);
        assert!(d.encode(&a, 128, 128).msg.is_none());
        d.set_quality(40, 45);
        assert!(d.encode(&a, 128, 128).keyframe, "quality change must resend");
    }
}
