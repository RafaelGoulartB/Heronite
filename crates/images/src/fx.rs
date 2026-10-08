//! Pixel effects on tightly packed 4-channel u8 buffers: box-blur approximated
//! Gaussian, CSS `saturate()`, channel swap, alpha flatten.

/// RGBA <-> BGRA in place.
pub fn swap_rb(buf: &mut [u8]) {
    for p in buf.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
    }
}

/// Composite straight-alpha pixels over black and make them opaque.
pub fn flatten_over_black(buf: &mut [u8]) {
    for p in buf.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        if a != 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
            p[3] = 255;
        }
    }
}

/// CSS `saturate(s)` (feColorMatrix type=saturate, in sRGB space like CSS
/// filter shorthand). Works for RGBA or BGRA alike when `bgra` is set.
pub fn saturate(buf: &mut [u8], s: f32, bgra: bool) {
    if (s - 1.0).abs() < 1e-4 {
        return;
    }
    let s = s.max(0.0) as f64;
    let m = [
        [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s],
        [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s],
        [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s],
    ];
    // 16.16 fixed point
    let f = |v: f64| (v * 65536.0).round() as i32;
    let mi = [
        [f(m[0][0]), f(m[0][1]), f(m[0][2])],
        [f(m[1][0]), f(m[1][1]), f(m[1][2])],
        [f(m[2][0]), f(m[2][1]), f(m[2][2])],
    ];
    let (ri, bi) = if bgra { (2, 0) } else { (0, 2) };
    for p in buf.as_chunks_mut::<4>().0 {
        let (r, g, b) = (p[ri] as i32, p[1] as i32, p[bi] as i32);
        let o = |row: &[i32; 3]| {
            let v = (row[0] * r + row[1] * g + row[2] * b + 32768) >> 16;
            v.clamp(0, 255) as u8
        };
        p[ri] = o(&mi[0]);
        p[1] = o(&mi[1]);
        p[bi] = o(&mi[2]);
    }
}

/// Box sizes whose successive application approximates a Gaussian of `sigma`
/// (Kutskir's "boxes for gauss"). Returns box *radii*.
pub fn box_radii(sigma: f32, n: usize) -> Vec<usize> {
    let sigma = sigma as f64;
    let nf = n as f64;
    let w_ideal = (12.0 * sigma * sigma / nf + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let wu = wl + 2;
    let m_ideal = (12.0 * sigma * sigma
        - nf * (wl * wl) as f64
        - 4.0 * nf * wl as f64
        - 3.0 * nf)
        / (-4.0 * wl as f64 - 4.0);
    let m = m_ideal.round() as i64;
    (0..n as i64)
        .map(|i| (if i < m { wl } else { wu } as usize - 1) / 2)
        .collect()
}

#[inline(always)]
fn div_recip(n: usize) -> u64 {
    // round(2^24 / n); sum*recip + 2^23 >> 24 keeps constant images exact.
    ((1u64 << 24) + n as u64 / 2) / n as u64
}

/// Horizontal clamp-to-edge box blur of radius `r`, `src -> dst`.
fn box_h(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    let n = 2 * r + 1;
    let recip = div_recip(n);
    for y in 0..h {
        let row = &src[y * w * 4..(y + 1) * w * 4];
        let out = &mut dst[y * w * 4..(y + 1) * w * 4];
        let px = |i: usize| -> [u32; 4] {
            let o = i * 4;
            [row[o] as u32, row[o + 1] as u32, row[o + 2] as u32, row[o + 3] as u32]
        };
        // initial window for x = 0: indices -r..=r clamped
        let first = px(0);
        let mut sum = [first[0] * (r as u32 + 1), first[1] * (r as u32 + 1), first[2] * (r as u32 + 1), first[3] * (r as u32 + 1)];
        for k in 1..=r {
            let p = px(k.min(w - 1));
            for c in 0..4 {
                sum[c] += p[c];
            }
        }
        for x in 0..w {
            let o = x * 4;
            for c in 0..4 {
                out[o + c] = ((sum[c] as u64 * recip + (1 << 23)) >> 24) as u8;
            }
            let add = px((x + r + 1).min(w - 1));
            let sub = px(x.saturating_sub(r));
            for c in 0..4 {
                sum[c] = sum[c] + add[c] - sub[c];
            }
        }
    }
}

/// Vertical clamp-to-edge box blur of radius `r`, `src -> dst`, cache-friendly
/// (one running sum per column, rows processed in order).
fn box_v(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, sums: &mut Vec<u32>) {
    let n = 2 * r + 1;
    let recip = div_recip(n);
    let stride = w * 4;
    sums.clear();
    sums.resize(stride, 0);
    // window for y = 0: rows -r..=r clamped
    for (i, s) in sums.iter_mut().enumerate() {
        *s = src[i] as u32 * (r as u32 + 1);
    }
    for k in 1..=r {
        let row = &src[k.min(h - 1) * stride..][..stride];
        for (s, &v) in sums.iter_mut().zip(row) {
            *s += v as u32;
        }
    }
    for y in 0..h {
        let out = &mut dst[y * stride..(y + 1) * stride];
        for (o, &s) in out.iter_mut().zip(sums.iter()) {
            *o = ((s as u64 * recip + (1 << 23)) >> 24) as u8;
        }
        let add = &src[(y + r + 1).min(h - 1) * stride..][..stride];
        let sub = &src[y.saturating_sub(r) * stride..][..stride];
        for ((s, &a), &b) in sums.iter_mut().zip(add).zip(sub) {
            *s = *s + a as u32 - b as u32;
        }
    }
}

/// Gaussian blur approximation (3 box passes per axis, clamp-to-edge).
/// `sigma` is in pixels of `buf`. All four channels are blurred identically.
pub fn gaussian_blur(buf: &mut Vec<u8>, w: usize, h: usize, sigma: f32) {
    if sigma < 0.5 || w == 0 || h == 0 {
        return;
    }
    assert_eq!(buf.len(), w * h * 4);
    let radii = box_radii(sigma, 3);
    let mut tmp = vec![0u8; buf.len()];
    let mut sums = Vec::new();
    // Ping-pong between the two buffers; 6 passes => result ends up in `tmp`.
    let mut a: &mut Vec<u8> = buf;
    let mut b: &mut Vec<u8> = &mut tmp;
    for &r in &radii {
        box_h(a, b, w, h, r);
        std::mem::swap(&mut a, &mut b);
    }
    for &r in &radii {
        box_v(a, b, w, h, r, &mut sums);
        std::mem::swap(&mut a, &mut b);
    }
    // After 6 swaps `a` points at the buffer holding the final data, which is
    // `buf` again (even number of passes).
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_radii_variance_matches_sigma() {
        for sigma in [2.0f32, 5.0, 10.0, 20.0, 40.0] {
            let radii = box_radii(sigma, 3);
            // variance of a box of width n is (n^2-1)/12
            let var: f64 = radii.iter().map(|&r| { let n = (2 * r + 1) as f64; (n * n - 1.0) / 12.0 }).sum();
            let got = var.sqrt();
            assert!((got - sigma as f64).abs() / (sigma as f64) < (0.08f64).max(0.5 / sigma as f64), "sigma {sigma} -> {got}");
        }
    }

    #[test]
    fn blur_keeps_constant_image_exact() {
        let (w, h) = (37, 23);
        let mut buf: Vec<u8> = (0..w * h).flat_map(|_| [200u8, 100, 7, 255]).collect();
        let orig = buf.clone();
        gaussian_blur(&mut buf, w, h, 6.0);
        assert_eq!(buf, orig);
    }

    #[test]
    fn blur_impulse_response_is_gaussian() {
        let (w, h) = (201usize, 201usize);
        let sigma = 8.0f32;
        let mut buf = vec![0u8; w * h * 4];
        // Bright square impulse is too coarse for u8; use a 255 block and
        // measure the second moment of the response along x through the center.
        for y in 98..103 {
            for x in 98..103 {
                for c in 0..4 {
                    buf[(y * w + x) * 4 + c] = 255;
                }
            }
        }
        gaussian_blur(&mut buf, w, h, sigma);
        let y = 100;
        let (mut m0, mut m2) = (0.0f64, 0.0f64);
        for x in 0..w {
            let v = buf[(y * w + x) * 4] as f64;
            m0 += v;
            let d = x as f64 - 100.0;
            m2 += v * d * d;
        }
        // subtract the variance of the 5px block (=2)
        let var = m2 / m0 - 2.0;
        let got = var.sqrt();
        assert!((got - sigma as f64).abs() / (sigma as f64) < 0.12, "measured sigma {got}");
        // symmetric and peaked at center
        assert!(buf[(y * w + 100) * 4] >= buf[(y * w + 110) * 4]);
        assert_eq!(buf[(y * w + 90) * 4], buf[(y * w + 110) * 4]);
        assert_eq!(buf[(90 * w + 100) * 4], buf[(110 * w + 100) * 4]);
    }

    #[test]
    fn blur_radius_larger_than_image() {
        let (w, h) = (4, 3);
        let mut buf: Vec<u8> = (0..w * h * 4).map(|i| (i * 5) as u8).collect();
        gaussian_blur(&mut buf, w, h, 50.0);
        // everything converges close to the mean, no panic
        let first = buf[0] as i32;
        assert!(buf.chunks(4).all(|p| (p[0] as i32 - first).abs() < 40));
    }

    #[test]
    fn saturate_matches_css_matrix() {
        // Pure red at s=0 -> luma 0.213 * 255 = 54.3
        let mut px = [255u8, 0, 0, 255];
        saturate(&mut px, 0.0, false);
        assert_eq!(px, [54, 54, 54, 255]);
        // s=1 is identity
        let mut px = [10u8, 200, 30, 128];
        saturate(&mut px, 1.0, false);
        assert_eq!(px, [10, 200, 30, 128]);
        // s=2 on a muted color boosts chroma and clamps
        let mut px = [200u8, 100, 100, 255];
        saturate(&mut px, 2.0, false);
        // R' = 1.787*200 - 0.715*100 - 0.072*100 = 278.7 -> clamp 255
        // G' = -0.213*200 + 1.285*100 - 0.072*100 = 78.7 -> 79
        assert_eq!(px[0], 255);
        assert_eq!(px[1], 79);
        assert_eq!(px[2], 79);
        assert_eq!(px[3], 255);
    }

    #[test]
    fn saturate_bgra_order() {
        let mut a = [200u8, 100, 100, 255];
        let mut b = [100u8, 100, 200, 255]; // same color, BGRA
        saturate(&mut a, 2.0, false);
        saturate(&mut b, 2.0, true);
        assert_eq!([a[0], a[1], a[2]], [b[2], b[1], b[0]]);
    }

    #[test]
    fn flatten_and_swap() {
        let mut px = [200u8, 100, 50, 128];
        flatten_over_black(&mut px);
        assert_eq!(px, [100, 50, 25, 255]);
        swap_rb(&mut px);
        assert_eq!(px, [25, 50, 100, 255]);
    }
}
