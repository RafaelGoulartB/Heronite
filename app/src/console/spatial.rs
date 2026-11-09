//! Spatial navigation over rectangles: from the focused one, the nearest
//! in a direction. Layouts that place tiles freely (the Dashboard's mosaic)
//! use it instead of an index; the units are the caller's (cells or px).

use super::Pad;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    /// The direction of a d-pad press.
    pub fn from_pad(pad: Pad) -> Option<Self> {
        match pad {
            Pad::Left => Some(Self::Left),
            Pad::Right => Some(Self::Right),
            Pad::Up => Some(Self::Up),
            Pad::Down => Some(Self::Down),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    fn centre(&self) -> (f32, f32) {
        (self.x + self.w / 2., self.y + self.h / 2.)
    }
}

/// How much a step sideways costs against a step along the move: a tile a
/// little further straight ahead wins over a nearer one off to the side.
const SIDE_WEIGHT: f32 = 1.6;
/// Rectangles that touch are not overlapping.
const EPS: f32 = 1e-3;

/// The rectangle to move to from `rects[from]` in `dir`: among those that
/// lie wholly beyond its edge, the one with the least `along + 1.6 * across`
/// between the centres. The first wins a tie; `None` when nothing lies that way.
pub fn nearest(rects: &[Rect], from: usize, dir: Dir) -> Option<usize> {
    let cur = rects.get(from)?;
    let (cx, cy) = cur.centre();
    let mut best: Option<(usize, f32)> = None;
    for (i, t) in rects.iter().enumerate() {
        if i == from {
            continue;
        }
        let beyond = match dir {
            Dir::Left => t.x + t.w <= cur.x + EPS,
            Dir::Right => t.x >= cur.x + cur.w - EPS,
            Dir::Up => t.y + t.h <= cur.y + EPS,
            Dir::Down => t.y >= cur.y + cur.h - EPS,
        };
        if !beyond {
            continue;
        }
        let (tx, ty) = t.centre();
        let (dx, dy) = (tx - cx, ty - cy);
        let (along, across) = match dir {
            Dir::Left => (-dx, dy.abs()),
            Dir::Right => (dx, dy.abs()),
            Dir::Up => (-dy, dx.abs()),
            Dir::Down => (dy, dx.abs()),
        };
        let score = along + across * SIDE_WEIGHT;
        if best.is_none_or(|(_, s)| score < s - EPS) {
            best = Some((i, score));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3x2 grid in reading order, then a tile spanning the two columns on the right.
    fn mosaic() -> Vec<Rect> {
        vec![
            Rect::new(0., 0., 1., 1.), // 0
            Rect::new(1., 0., 1., 1.), // 1
            Rect::new(0., 1., 1., 1.), // 2
            Rect::new(1., 1., 1., 1.), // 3
            Rect::new(2., 0., 1., 2.), // 4: tall, beside 1 and 3
        ]
    }

    #[test]
    fn moves_to_the_neighbour_in_each_direction() {
        let r = mosaic();
        assert_eq!(nearest(&r, 0, Dir::Right), Some(1));
        assert_eq!(nearest(&r, 0, Dir::Down), Some(2));
        assert_eq!(nearest(&r, 3, Dir::Left), Some(2));
        assert_eq!(nearest(&r, 3, Dir::Up), Some(1));
    }

    #[test]
    fn nothing_beyond_the_edge_is_none() {
        let r = mosaic();
        assert_eq!(nearest(&r, 0, Dir::Left), None);
        assert_eq!(nearest(&r, 0, Dir::Up), None);
        assert_eq!(nearest(&r, 4, Dir::Right), None);
        assert_eq!(nearest(&r, 2, Dir::Down), None);
    }

    #[test]
    fn a_tall_tile_is_reached_from_either_side() {
        let r = mosaic();
        assert_eq!(nearest(&r, 1, Dir::Right), Some(4));
        assert_eq!(nearest(&r, 3, Dir::Right), Some(4));
        // Back: the tile on the same row as the tall one's centre is a tie, the first wins.
        assert_eq!(nearest(&r, 4, Dir::Left), Some(1));
    }

    #[test]
    fn straight_ahead_beats_a_nearer_tile_off_to_the_side() {
        let r = vec![
            Rect::new(0., 0., 2., 2.),  // 0: big
            Rect::new(2., 0., 1., 1.),  // 1: beside its top half
            Rect::new(2., 1.5, 1., 1.), // 2: lower, off its centre line
            Rect::new(3., 0.5, 1., 1.), // 3: farther, level with the centre
        ];
        assert_eq!(nearest(&r, 0, Dir::Right), Some(1));
        assert_eq!(nearest(&r, 3, Dir::Left), Some(1));
    }

    #[test]
    fn touching_edges_count_as_beyond_and_overlaps_do_not() {
        let r = vec![Rect::new(0., 0., 2., 1.), Rect::new(1., 0., 2., 1.), Rect::new(2., 0., 1., 1.)];
        // 1 overlaps 0 (it starts inside), so only 2 lies to the right of 0.
        assert_eq!(nearest(&r, 0, Dir::Right), Some(2));
    }

    #[test]
    fn a_bad_start_is_none() {
        assert_eq!(nearest(&[], 0, Dir::Left), None);
        assert_eq!(nearest(&mosaic(), 9, Dir::Left), None);
    }

    #[test]
    fn d_pad_presses_map_to_directions() {
        assert_eq!(Dir::from_pad(Pad::Up), Some(Dir::Up));
        assert_eq!(Dir::from_pad(Pad::A), None);
    }
}
