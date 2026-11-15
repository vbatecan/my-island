//! Fullscreen policy and finite activity preview eligibility.
#[derive(Default)]
pub(super) struct Visibility {
    pub surface: crate::platform::SurfaceState,
    pub keep_visible: bool,
    pub hover_hidden: bool,
    pub protected_visit: bool,
    pub edge_armed: bool,
    pub edge_deadline: Option<u64>,
    pub covered: bool,
    pub locked: bool,
    pub hiding: bool,
    pub edge_hover: bool,
    pub until: Option<u64>,
    pub remaining: Option<u64>,
    pub preview: Option<super::Content>,
}
impl Visibility {
    pub fn hover_enabled(&self, settings: &crate::settings::Settings) -> bool {
        settings.hide_on_hover
            && !self.covered
            && !self.locked
            && !self.keep_visible
            && (!settings.hover_only_maximized
                || self.surface == crate::platform::SurfaceState::Maximized)
    }
    /// Returns true only when a fresh, uninterrupted edge hold completes.
    pub fn observe_edge(&mut self, now: u64, edge: bool, pressed: bool) -> bool {
        if !self.hover_hidden {
            return false;
        }
        if !edge || pressed {
            self.edge_deadline = None;
            if !edge {
                self.edge_armed = true;
            }
        } else if self.edge_armed {
            if let Some(end) = self.edge_deadline {
                if now >= end {
                    self.reveal_hover();
                    return true;
                }
            } else {
                self.edge_deadline = Some(now.saturating_add(350));
            }
        }
        false
    }
    pub fn reset_hover(&mut self) {
        self.hover_hidden = false;
        self.protected_visit = true;
        self.edge_armed = false;
        self.edge_deadline = None;
    }
    pub fn reveal_hover(&mut self) {
        self.reset_hover();
    }
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
        self.locked
            || (!self.covered && self.hover_hidden)
            || (self.covered && !engaged && self.until.is_none_or(|end| now >= end))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scoped_hover_requires_maximized_surface_and_respects_holds() {
        let mut settings = crate::settings::Settings::default();
        settings.hide_on_hover = true;
        settings.hover_only_maximized = true;
        let mut v = Visibility::default();
        assert!(!v.hover_enabled(&settings));
        v.surface = crate::platform::SurfaceState::Maximized;
        assert!(v.hover_enabled(&settings));
        v.keep_visible = true;
        assert!(!v.hover_enabled(&settings));
        v.keep_visible = false;
        v.surface = crate::platform::SurfaceState::Fullscreen;
        assert!(!v.hover_enabled(&settings));
        settings.hover_only_maximized = false;
        v.surface = crate::platform::SurfaceState::Ordinary;
        assert!(v.hover_enabled(&settings));
        v.locked = true;
        assert!(!v.hover_enabled(&settings));
        assert!(!crate::settings::Settings::parse("hide_on_hover=true\n").hover_only_maximized);
        settings.hover_only_maximized = true;
        assert!(crate::settings::Settings::parse(&settings.encode()).hover_only_maximized);
    }
    #[test]
    fn hover_hide_requires_fresh_uninterrupted_edge_hold() {
        let mut v = Visibility {
            hover_hidden: true,
            ..Default::default()
        };
        assert!(!v.observe_edge(0, true, false));
        assert_eq!(v.edge_deadline, None);
        v.observe_edge(10, false, false);
        v.observe_edge(20, true, false);
        assert_eq!(v.edge_deadline, Some(370));
        assert!(!v.observe_edge(369, true, false));
        assert!(v.observe_edge(370, true, false));
        assert!(!v.hover_hidden);
        assert!(v.protected_visit);
        assert!(!v.should_hide(400, false));
    }
    #[test]
    fn edge_exit_buttons_and_reset_cancel_pending_reveal() {
        let mut v = Visibility {
            hover_hidden: true,
            edge_armed: true,
            ..Default::default()
        };
        v.observe_edge(0, true, false);
        v.observe_edge(100, false, false);
        assert_eq!(v.edge_deadline, None);
        v.observe_edge(200, true, false);
        v.observe_edge(550, true, true);
        assert!(v.hover_hidden);
        assert_eq!(v.edge_deadline, None);
        v.reset_hover();
        assert!(!v.observe_edge(900, true, false));
        v.locked = true;
        assert!(v.should_hide(900, true));
    }
    #[test]
    fn preference_migrates_and_round_trips() {
        use crate::settings::Settings;
        assert!(!Settings::parse("hover_expand=true\n").hide_on_hover);
        let s = Settings::parse("hide_on_hover=true\nhover_expand=true\n");
        assert!(s.hide_on_hover && s.hover_expand);
        assert!(Settings::parse(&s.encode()).hide_on_hover);
    }
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
