//! Pointer intent without a renderer or clock dependency. A GUI schedules a
//! single wake at `deadline`; no work is needed while the pointer is stationary.

#[derive(Default, Debug)]
pub struct HoverIntent {
    entered: Option<u64>,
    anchor: Option<(f64, f64)>,
}

impl HoverIntent {
    /// Starts a fresh visit. Leaving and reentering must earn the dwell again.
    pub fn enter(&mut self, now: u64) {
        if self.entered.is_none() {
            self.entered = Some(now);
            self.anchor = None;
        }
    }

    /// Small hand tremors are tolerated, but a pointer moving across the island
    /// must become still before its dwell can trigger an expansion.
    pub fn moved(&mut self, x: f64, y: f64, now: u64) {
        if self.entered.is_none() || !x.is_finite() || !y.is_finite() {
            return;
        }
        if let Some((anchor_x, anchor_y)) = self.anchor {
            if (x - anchor_x).hypot(y - anchor_y) < 6.0 {
                return;
            }
            self.entered = Some(now);
        }
        self.anchor = Some((x, y));
    }

    pub fn leave(&mut self) {
        self.entered = None;
        self.anchor = None;
    }

    pub fn deadline(&self, delay_ms: u32) -> Option<u64> {
        self.entered
            .map(|entered| entered.saturating_add(u64::from(delay_ms)))
    }

    pub fn ready(&self, now: u64, delay_ms: u32) -> bool {
        self.deadline(delay_ms)
            .is_some_and(|deadline| now >= deadline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rapid_crossings_leave_no_stale_dwell_or_child_reentry_reset() {
        let mut intent = HoverIntent::default();
        for now in 0..1000 {
            intent.enter(now);
            intent.moved(40., 16., now);
            intent.leave();
            intent.moved(41., 17., now);
            assert_eq!(intent.deadline(200), None);
        }
        intent.enter(1000);
        intent.moved(40., 16., 1000);
        for now in 1001..1200 {
            intent.enter(now); // child controls report duplicate entry
            intent.moved(41., 17., now);
        }
        assert!(!intent.ready(1199, 200));
        assert!(intent.ready(1200, 200));
    }

    #[test]
    fn physical_movement_normalized_for_dpi_and_zoom_has_the_same_intent() {
        for scale in [0.75, 1., 1.25, 1.5, 2., 3., 6.] {
            let mut intent = HoverIntent::default();
            intent.enter(0);
            intent.moved(100. * scale / scale, 16. * scale / scale, 0);
            intent.moved(105.9 * scale / scale, 16. * scale / scale, 100);
            assert_eq!(intent.deadline(200), Some(200));
            intent.moved(106.1 * scale / scale, 16. * scale / scale, 150);
            assert_eq!(intent.deadline(200), Some(350));
        }
    }

    #[test]
    fn duplicate_enters_do_not_starve_the_same_hover_visit() {
        let mut intent = HoverIntent::default();
        intent.enter(10);
        intent.moved(100., 50., 10);
        intent.enter(200);
        intent.moved(102., 51., 200);
        assert_eq!(intent.deadline(350), Some(360));
        assert!(intent.ready(360, 350));
        intent.leave();
        intent.enter(400);
        assert_eq!(intent.deadline(350), Some(750));
    }

    #[test]
    fn requires_an_uninterrupted_visit() {
        let mut intent = HoverIntent::default();
        assert!(!intent.ready(1000, 350));
        intent.enter(100);
        assert_eq!(intent.deadline(350), Some(450));
        assert!(!intent.ready(449, 350));
        intent.leave();
        assert_eq!(intent.deadline(350), None);
        assert!(!intent.ready(450, 350));
        intent.enter(500);
        assert!(!intent.ready(849, 350));
        assert!(intent.ready(850, 350));
    }

    #[test]
    fn crossing_pointer_restarts_dwell_but_jitter_does_not() {
        let mut intent = HoverIntent::default();
        intent.enter(0);
        intent.moved(0.0, 0.0, 0);
        intent.moved(3.0, 2.0, 100);
        assert_eq!(intent.deadline(350), Some(350));
        intent.moved(6.0, 0.0, 200);
        assert_eq!(intent.deadline(350), Some(550));
        assert!(!intent.ready(350, 350));
        intent.moved(12.0, 0.0, 400);
        assert_eq!(intent.deadline(350), Some(750));
        assert!(!intent.ready(550, 350));
        assert!(intent.ready(750, 350));
    }

    #[test]
    fn ignores_nonfinite_coordinates_and_saturates_extreme_time() {
        let mut intent = HoverIntent::default();
        intent.enter(u64::MAX - 1);
        intent.moved(f64::NAN, 0.0, u64::MAX);
        assert_eq!(intent.deadline(350), Some(u64::MAX));
        assert!(!intent.ready(u64::MAX - 1, 350));
        assert!(intent.ready(u64::MAX, 350));
        intent.leave();
        intent.moved(100.0, 100.0, 0);
        assert_eq!(intent.deadline(350), None);
    }
}
