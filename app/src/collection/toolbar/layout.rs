//! A prefix of the quick controls, after the header's fixed controls and
//! filter chips have taken their space. All dimensions are logical pixels.

pub(in crate::collection) const GAP: f32 = 6.;
pub(super) const DRAG_SPACE: f32 = 48.;

/// The widths already include each control's internal padding and gaps.
/// Reserve a gap after the last quick control, before the chips/drag area.
pub(super) fn visible_count(enabled: bool, lane: f32, chips: f32, widths: &[f32]) -> usize {
    if !enabled {
        return 0;
    }
    let mut remaining = (lane - chips - DRAG_SPACE).max(0.);
    widths
        .iter()
        .take_while(|width| {
            let need = **width + GAP;
            if need > remaining {
                return false;
            }
            remaining -= need;
            true
        })
        .count()
}

/// The prefix to show when a shown control hides its own filter chip
/// (`chips(shown)` is the chips' lane width then). Starting from every
/// control, each pass gives back the chips of the controls that did not fit,
/// which can only shrink the prefix, so it settles within `widths.len()` passes.
pub(super) fn fit(enabled: bool, lane: f32, widths: &[f32], chips: impl Fn(usize) -> f32) -> usize {
    let mut shown = if enabled { widths.len() } else { 0 };
    loop {
        let next = visible_count(enabled, lane, chips(shown), widths);
        if next >= shown {
            return shown;
        }
        shown = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_thresholds_preserve_priority_and_never_overflow() {
        // Shaped control widths: ordinary and long localized labels.
        for widths in [[166., 120., 32., 135., 156.], [260., 183., 32., 245., 192.]] {
            for chips in [0., 87., 452.] {
                let mut threshold = DRAG_SPACE + chips;
                for count in 1..=widths.len() {
                    threshold += widths[count - 1] + GAP;
                    assert_eq!(visible_count(true, threshold - 0.25, chips, &widths), count - 1);
                    assert_eq!(visible_count(true, threshold, chips, &widths), count);
                }
                for lane in 0..1800 {
                    let lane = lane as f32;
                    let count = visible_count(true, lane, chips, &widths);
                    let used = widths[..count].iter().map(|w| w + GAP).sum::<f32>();
                    assert!(used <= (lane - chips - DRAG_SPACE).max(0.));
                }
            }
        }
    }

    #[test]
    fn no_space_disabled_and_no_presets() {
        let widths = [180., 130., 32., 160.];
        assert_eq!(visible_count(true, 0., 0., &widths), 0);
        assert_eq!(visible_count(true, 200., 240., &widths), 0);
        assert_eq!(visible_count(false, 4000., 0., &widths), 0);
        assert_eq!(visible_count(true, 4000., 0., &widths), 4);
        // A favourite button must not leapfrog the higher-priority selects.
        assert_eq!(visible_count(true, 100., 0., &widths), 0);
    }

    #[test]
    fn a_control_that_fits_takes_its_chips_place_and_gives_it_back_when_it_does_not() {
        let widths = [100., 80., 26.];
        // The install chip (40px with its gap) leaves while the install control shows.
        let chips = |shown: usize| if shown >= 2 { 0. } else { 40. };
        let two = DRAG_SPACE + (100. + GAP) + (80. + GAP);
        assert_eq!(fit(true, two, &widths, chips), 2);
        // Install no longer fits: its chip returns, and sort still fits beside it.
        assert_eq!(fit(true, two - 1., &widths, chips), 1);
        // Sort would fit beside no chip, but install does not fit, so its chip returns and sort gives way.
        assert_eq!(fit(true, DRAG_SPACE + 40. + 100. + GAP - 1., &widths, chips), 0);
        assert_eq!(fit(false, 4000., &widths, chips), 0);
        assert_eq!(fit(true, 4000., &widths, chips), 3);
    }

    #[test]
    fn pills_reduce_the_lane_and_chips_have_first_claim() {
        let widths = [180., 130., 32., 160., 140.];
        let lane = DRAG_SPACE + widths.iter().map(|w| w + GAP).sum::<f32>();
        assert_eq!(visible_count(true, lane, 0., &widths), 5);
        for occupied in [40., 140., 310., 800.] {
            assert_eq!(visible_count(true, lane - occupied, 0., &widths), visible_count(true, lane, occupied, &widths));
        }
    }
}
