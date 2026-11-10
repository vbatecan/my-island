//! Fullscreen policy and finite activity preview eligibility.
#[derive(Default)]
pub(super) struct Visibility {
    pub covered: bool,
    pub locked: bool,
    pub hiding: bool,
    pub edge_hover: bool,
    pub until: Option<u64>,
    pub remaining: Option<u64>,
    pub preview: Option<super::Content>,
}
impl Visibility {
    pub fn pause(&mut self, now: u64, engaged: bool) {
        if engaged {
            if let Some(end) = self.until.take() {
                self.remaining = Some(end.saturating_sub(now).max(1));
            }
        } else if let Some(remaining) = self.remaining.take() {
            self.until = Some(now.saturating_add(remaining));
        }
    }
    pub fn should_hide(&self, now: u64, engaged: bool) -> bool {
        self.locked || (self.covered && !engaged && self.until.is_none_or(|end| now >= end))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edge_reveal_holds_then_expires_after_exit_and_lock_wins() {
        let mut v = Visibility {
            covered: true,
            edge_hover: true,
            until: Some(600),
            ..Default::default()
        };
        v.pause(0, v.edge_hover);
        assert!(!v.should_hide(1000, v.edge_hover));
        assert_eq!(v.until, None);
        v.edge_hover = false;
        v.pause(1100, v.edge_hover);
        assert_eq!(v.until, Some(1700));
        assert!(!v.should_hide(1699, false));
        assert!(v.should_hide(1700, false));
        v.locked = true;
        assert!(v.should_hide(1200, true));
    }
    #[test]
    fn hover_suspends_the_remaining_dwell_without_periodic_deadlines() {
        let mut v = Visibility {
            covered: true,
            until: Some(3000),
            ..Default::default()
        };
        v.pause(1000, true);
        assert_eq!(v.until, None);
        assert_eq!(v.remaining, Some(2000));
        v.pause(10000, true);
        v.pause(11000, false);
        assert_eq!(v.until, Some(13000));
        assert!(!v.should_hide(12999, false));
        assert!(v.should_hide(13000, false));
    }
    #[test]
    fn fullscreen_preview_expires_but_interaction_and_desktop_remain_visible() {
        let mut v = Visibility {
            covered: true,
            ..Default::default()
        };
        assert!(v.should_hide(0, false));
        v.until = Some(3000);
        assert!(!v.should_hide(2999, false));
        assert!(v.should_hide(3000, false));
        assert!(!v.should_hide(3000, true));
        v.locked = true;
        assert!(v.should_hide(1, true));
        v.locked = false;
        v.covered = false;
        assert!(!v.should_hide(4000, false));
    }
}
