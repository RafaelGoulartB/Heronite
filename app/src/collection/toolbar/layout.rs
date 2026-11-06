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
    fn pills_reduce_the_lane_and_chips_have_first_claim() {
        let widths = [180., 130., 32., 160., 140.];
        let lane = DRAG_SPACE + widths.iter().map(|w| w + GAP).sum::<f32>();
        assert_eq!(visible_count(true, lane, 0., &widths), 5);
        for occupied in [40., 140., 310., 800.] {
            assert_eq!(visible_count(true, lane - occupied, 0., &widths), visible_count(true, lane, occupied, &widths));
        }
    }
}
