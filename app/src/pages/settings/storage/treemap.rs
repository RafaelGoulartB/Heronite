//! The installed games as a treemap: one tile per game, its area the space
//! the game takes, laid out with the squarified algorithm (Bruls, Huizing,
//! van Wijk) so tiles stay close to squares and the art reads well.

/// A tile: `index` into the values given to [`squarify`], in px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Tile {
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Lays `values` (sorted largest first, all > 0) out in a `w` x `h` box.
pub(super) fn squarify(values: &[f64], w: f32, h: f32) -> Vec<Tile> {
    let total: f64 = values.iter().sum();
    if values.is_empty() || total <= 0. || w <= 0. || h <= 0. {
        return Vec::new();
    }
    let scale = (w as f64 * h as f64) / total;
    let areas: Vec<f64> = values.iter().map(|v| v * scale).collect();
    let (mut x, mut y, mut rw, mut rh) = (0f64, 0f64, w as f64, h as f64);
    let mut out = Vec::with_capacity(values.len());
    let mut i = 0;
    while i < areas.len() {
        let side = rw.min(rh);
        let mut end = i + 1;
        let mut best = worst(&areas[i..end], side);
        while end < areas.len() {
            let next = worst(&areas[i..=end], side);
            if next > best {
                break;
            }
            best = next;
            end += 1;
        }
        let row: f64 = areas[i..end].iter().sum();
        if rw >= rh {
            // A column along the left edge.
            let cw = if end == areas.len() { rw } else { row / rh };
            let mut ty = y;
            for (k, a) in areas[i..end].iter().enumerate() {
                let th = if i + k + 1 == end { y + rh - ty } else { a / cw };
                out.push(Tile { index: i + k, x: x as f32, y: ty as f32, w: cw as f32, h: th as f32 });
                ty += th;
            }
            x += cw;
            rw -= cw;
        } else {
            // A row along the top edge.
            let rhh = if end == areas.len() { rh } else { row / rw };
            let mut tx = x;
            for (k, a) in areas[i..end].iter().enumerate() {
                let tw = if i + k + 1 == end { x + rw - tx } else { a / rhh };
                out.push(Tile { index: i + k, x: tx as f32, y: y as f32, w: tw as f32, h: rhh as f32 });
                tx += tw;
            }
            y += rhh;
            rh -= rhh;
        }
        i = end;
    }
    out
}

/// The worst aspect ratio of a row of `areas` laid along `side`.
fn worst(areas: &[f64], side: f64) -> f64 {
    let sum: f64 = areas.iter().sum();
    let max = areas.iter().cloned().fold(f64::MIN, f64::max);
    let min = areas.iter().cloned().fold(f64::MAX, f64::min);
    let (s2, w2) = (sum * sum, side * side);
    (w2 * max / s2).max(s2 / (w2 * min))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_fill_the_box_in_proportion() {
        let values = [6., 6., 4., 3., 2., 2., 1.];
        let tiles = squarify(&values, 600., 400.);
        assert_eq!(tiles.len(), values.len());
        let area: f32 = tiles.iter().map(|t| t.w * t.h).sum();
        assert!((area - 600. * 400.).abs() < 1., "{area}");
        let total: f64 = values.iter().sum();
        for t in &tiles {
            assert!(t.x >= -0.01 && t.y >= -0.01 && t.x + t.w <= 600.01 && t.y + t.h <= 400.01, "{t:?}");
            let want = values[t.index] / total * 240_000.;
            assert!(((t.w * t.h) as f64 - want).abs() < 1., "{t:?} {want}");
        }
        // Squarified: no sliver for the biggest games.
        assert!(tiles.iter().take(2).all(|t| t.w.max(t.h) / t.w.min(t.h) < 3.));
    }

    #[test]
    fn nothing_to_lay_out() {
        assert!(squarify(&[], 100., 100.).is_empty());
        assert!(squarify(&[1.], 0., 100.).is_empty());
        let one = squarify(&[5.], 300., 200.);
        assert_eq!(one, [Tile { index: 0, x: 0., y: 0., w: 300., h: 200. }]);
    }
}
