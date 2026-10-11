//! The "river" of the month-by-month strip: a streamgraph of the period's
//! buckets (months), one band per chart slot plus "Other", centred on a
//! line so a month without play pinches the river shut. Bands are smooth
//! (monotone cubic through the bucket centres, so a band never goes below
//! zero) and sampled into polygons painted with GPUI paths.

use gpui_kit::*;

use crate::theme;

/// Samples per bucket.
const STEP: usize = 16;
/// Thinner than this (px) a band is not drawn.
const MIN_TH: f32 = 0.6;

/// Bottom-to-top order of the layers: "Other" at the bottom, the biggest
/// game in the middle.
pub(super) const ORDER: [Option<usize>; 7] = [None, Some(5), Some(3), Some(1), Some(0), Some(2), Some(4)];

/// Monotone cubic (Fritsch–Carlson) through `(xs, ys)`.
struct Mono {
    xs: Vec<f32>,
    ys: Vec<f32>,
    m: Vec<f32>,
}

impl Mono {
    fn new(xs: Vec<f32>, ys: Vec<f32>) -> Self {
        let n = xs.len();
        let d: Vec<f32> = (0..n - 1).map(|i| (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i])).collect();
        let mut m = vec![0f32; n];
        m[0] = d[0];
        m[n - 1] = d[n - 2];
        for i in 1..n - 1 {
            m[i] = if d[i - 1] * d[i] <= 0. { 0. } else { (d[i - 1] + d[i]) / 2. };
        }
        for i in 0..n - 1 {
            if d[i] == 0. {
                m[i] = 0.;
                m[i + 1] = 0.;
                continue;
            }
            let (a, b) = (m[i] / d[i], m[i + 1] / d[i]);
            let s = a * a + b * b;
            if s > 9. {
                let t = 3. / s.sqrt();
                m[i] = t * a * d[i];
                m[i + 1] = t * b * d[i];
            }
        }
        Self { xs, ys, m }
    }

    fn at(&self, x: f32) -> f32 {
        let n = self.xs.len();
        let x = x.clamp(self.xs[0], self.xs[n - 1]);
        let mut i = 0;
        while i < n - 2 && x > self.xs[i + 1] {
            i += 1;
        }
        let h = self.xs[i + 1] - self.xs[i];
        let t = (x - self.xs[i]) / h;
        let (t2, t3) = (t * t, t * t * t);
        (2. * t3 - 3. * t2 + 1.) * self.ys[i] + (t3 - 2. * t2 + t) * h * self.m[i] + (-2. * t3 + 3. * t2) * self.ys[i + 1] + (t3 - t2) * h * self.m[i + 1]
    }
}

/// One band: its slot (`None` = Other) and the polygons of its runs, in px
/// of the plot (x from 0 to the plot width).
pub(super) struct Band {
    pub slot: Option<usize>,
    pub polys: Vec<Vec<Point<Pixels>>>,
}

/// The river of `layers` (seconds per bucket, in [`ORDER`]), `w` px wide,
/// `k` px per second, centred on `y0`.
pub(super) struct River {
    curves: Vec<Mono>,
    slots: Vec<Option<usize>>,
    buckets: usize,
    k: f32,
    y0: f32,
}

impl River {
    pub fn new(layers: Vec<(Option<usize>, Vec<f64>)>, k: f32, y0: f32) -> Self {
        let buckets = layers.first().map_or(0, |l| l.1.len());
        let mut xs = vec![0f32];
        xs.extend((0..buckets).map(|b| b as f32 + 0.5));
        xs.push(buckets as f32);
        let (slots, curves) = layers
            .into_iter()
            .map(|(slot, v)| {
                let mut ys = vec![v.first().copied().unwrap_or(0.) as f32];
                ys.extend(v.iter().map(|s| *s as f32));
                ys.push(v.last().copied().unwrap_or(0.) as f32);
                (slot, Mono::new(xs.clone(), ys))
            })
            .unzip();
        Self { curves, slots, buckets, k, y0 }
    }

    /// `(top, bottom)` of each layer at bucket position `t` (0..=buckets).
    pub fn layout(&self, t: f32) -> Vec<(f32, f32)> {
        let ths: Vec<f32> = self.curves.iter().map(|c| c.at(t).max(0.) * self.k).collect();
        let total: f32 = ths.iter().sum();
        let mut y = self.y0 + total / 2.;
        ths.into_iter()
            .map(|th| {
                let out = (y - th, y);
                y -= th;
                out
            })
            .collect()
    }

    pub fn bands(&self, w: f32) -> Vec<Band> {
        if self.buckets == 0 {
            return Vec::new();
        }
        let n = self.buckets * STEP;
        let col = w / self.buckets as f32;
        let samples: Vec<Vec<(f32, f32)>> = (0..=n).map(|j| self.layout(j as f32 / STEP as f32)).collect();
        let x = |j: usize| px(j as f32 / STEP as f32 * col);
        self.slots
            .iter()
            .enumerate()
            .map(|(li, slot)| {
                let mut polys = Vec::new();
                let mut run: Vec<usize> = Vec::new();
                for j in 0..=n + 1 {
                    let on = j <= n && samples[j][li].1 - samples[j][li].0 > MIN_TH;
                    if on {
                        run.push(j);
                    } else if !run.is_empty() {
                        let mut poly: Vec<Point<Pixels>> = run.iter().map(|&j| point(x(j), px(samples[j][li].0))).collect();
                        poly.extend(run.iter().rev().map(|&j| point(x(j), px(samples[j][li].1))));
                        polys.push(poly);
                        run.clear();
                    }
                }
                Band { slot: *slot, polys }
            })
            .collect()
    }
}

/// Paints `bands` at `origin`; `dim` fades every band but `focus` (hovered
/// legend row or band).
pub(super) fn paint(bands: &[Band], origin: Point<Pixels>, focus: Option<Option<usize>>, window: &mut Window) {
    for band in bands {
        let color: Hsla = match band.slot {
            Some(s) => theme::chart_series(s).into(),
            None => theme::chart_other(),
        };
        let faded = focus.is_some_and(|f| f != band.slot);
        let color = if faded { Hsla { a: color.a * 0.2, ..color } } else { color };
        for poly in &band.polys {
            let mut fill = PathBuilder::fill();
            let mut edge = PathBuilder::stroke(px(1.5));
            for (i, p) in poly.iter().enumerate() {
                let p = *p + origin;
                if i == 0 {
                    fill.move_to(p);
                    edge.move_to(p);
                } else {
                    fill.line_to(p);
                    edge.line_to(p);
                }
            }
            fill.close();
            edge.close();
            if let Ok(path) = fill.build() {
                window.paint_path(path, color);
            }
            // The surface colour between bands (the 2px gap of the charts).
            if let Ok(path) = edge.build() {
                window.paint_path(path, theme::river_gap());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::River;

    #[test]
    fn a_month_without_play_pinches_the_river() {
        let r = River::new(vec![(None, vec![10., 0., 10.]), (Some(0), vec![20., 0., 5.])], 1., 50.);
        let at = |t: f32| r.layout(t).iter().map(|(a, b)| b - a).sum::<f32>();
        assert!((at(0.5) - 30.).abs() < 1e-3);
        assert!(at(1.5).abs() < 1e-3);
        assert!((at(2.5) - 15.).abs() < 1e-3);
        // Centred on y0, stacked bottom to top.
        let l = r.layout(0.5);
        assert_eq!(l[0], (55., 65.));
        assert_eq!(l[1], (35., 55.));
        // Never below zero between a full and an empty month.
        assert!((0..=30).all(|i| r.layout(i as f32 / 10.).iter().all(|(a, b)| b >= a)));
    }

    #[test]
    fn bands_split_into_runs_around_gaps() {
        let r = River::new(vec![(Some(0), vec![10., 0., 0., 10.])], 1., 0.);
        let bands = r.bands(400.);
        assert_eq!(bands.len(), 1);
        assert_eq!(bands[0].polys.len(), 2);
    }
}
