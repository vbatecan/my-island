//! Decide when a timeline change is continuous enough for a one-line scroll.
//! Rendering owns interpolation; this model neither schedules frames nor polls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineChange {
    Unchanged,
    Snap,
    Advance,
}

#[derive(Default)]
pub struct LyricCursor {
    index: Option<usize>,
    position: Option<f64>,
    initialized: bool,
}

impl LyricCursor {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn update(&mut self, index: Option<usize>, position: f64, motion: bool) -> LineChange {
        let adjacent = match (self.index, index) {
            (None, Some(0)) => true,
            (Some(previous), Some(next)) => previous.checked_add(1) == Some(next),
            _ => false,
        };
        let continuous = self.position.is_some_and(|previous| {
            position.is_finite() && (0.0..=0.75).contains(&(position - previous))
        });
        let change = if self.initialized && self.index == index {
            LineChange::Unchanged
        } else if self.initialized && adjacent && continuous && motion {
            LineChange::Advance
        } else {
            LineChange::Snap
        };
        self.initialized = true;
        self.index = index;
        self.position = position.is_finite().then_some(position);
        change
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_playback_moves_each_line_once_and_initial_load_does_not_scroll() {
        let mut cursor = LyricCursor::default();
        assert_eq!(cursor.update(Some(40), 100., true), LineChange::Snap);
        assert_eq!(cursor.update(Some(40), 100.25, true), LineChange::Unchanged);
        assert_eq!(cursor.update(Some(41), 100.5, true), LineChange::Advance);
        assert_eq!(cursor.update(Some(41), 100.75, true), LineChange::Unchanged);
        assert_eq!(cursor.update(Some(42), 101., true), LineChange::Advance);
        cursor.reset();
        assert_eq!(cursor.update(None, 0., true), LineChange::Snap);
        assert_eq!(cursor.update(None, 0.25, true), LineChange::Unchanged);
        assert_eq!(cursor.update(Some(0), 0.5, true), LineChange::Advance);
    }

    #[test]
    fn seeks_delayed_updates_and_disabled_motion_rebase_to_current_line() {
        let mut cursor = LyricCursor::default();
        cursor.update(Some(1), 10., true);
        assert_eq!(cursor.update(Some(2), 30., true), LineChange::Snap);
        assert_eq!(cursor.update(Some(1), 10., true), LineChange::Snap);
        assert_eq!(cursor.update(Some(8), 10.25, true), LineChange::Snap);
        assert_eq!(cursor.update(Some(9), 10.5, false), LineChange::Snap);
        assert_eq!(cursor.update(Some(10), 12., true), LineChange::Snap);
        assert_eq!(cursor.update(Some(10), 12., false), LineChange::Unchanged);
    }

    #[test]
    fn blank_lines_and_invalid_positions_do_not_start_repeated_work() {
        let mut cursor = LyricCursor::default();
        cursor.update(None, f64::NAN, true);
        assert_eq!(
            cursor.update(None, f64::INFINITY, true),
            LineChange::Unchanged
        );
        assert_eq!(cursor.update(Some(0), 1., true), LineChange::Snap);
        assert_eq!(cursor.update(Some(1), 1.25, true), LineChange::Advance);
        assert_eq!(cursor.update(Some(1), 1.5, true), LineChange::Unchanged);
        cursor.update(Some(usize::MAX), 2., true);
        assert_eq!(cursor.update(Some(0), 2.25, true), LineChange::Snap);
    }
}
