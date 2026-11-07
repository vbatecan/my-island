//! Pure presentation arbitration. Clock values are supplied by the GUI, never read
//! here, so precedence, expiration, and interruption tests are deterministic.
use crate::{services::notifications::NotificationRecord, state::IslandState};
use std::collections::VecDeque;
#[derive(Clone, Debug, PartialEq)]
pub enum Panel {
    Launcher,
    Media,
    Volume,
    Notification(NotificationRecord),
}
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Idle,
    CompactMedia,
    Volume,
    Launcher,
    Media,
    VolumePanel,
    Notification(NotificationRecord, u32),
    ExpandedNotification(NotificationRecord),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Presentation {
    pub content: Content,
    pub pending: usize,
    pub volume_chip: bool,
}
impl Presentation {
    pub fn state(&self) -> IslandState {
        match self.content {
            Content::Idle => IslandState::Idle,
            Content::CompactMedia | Content::Volume | Content::Notification(..) => {
                IslandState::Compact
            }
            Content::Launcher => IslandState::Expanded,
            Content::Media => IslandState::Media,
            Content::VolumePanel => IslandState::Volume,
            Content::ExpandedNotification(..) => IslandState::Notification,
        }
    }
}
#[derive(Clone, Debug)]
struct Card {
    record: NotificationRecord,
    ids: VecDeque<NotificationRecord>,
    arrived: u64,
    latest: u64,
    remaining: u64,
    shown: u64,
}
impl Card {
    fn new(record: NotificationRecord, now: u64, dwell: u64) -> Self {
        Self {
            ids: VecDeque::from([record.clone()]),
            record,
            arrived: now,
            latest: now,
            remaining: dwell,
            shown: 0,
        }
    }
    fn contains_id(&self, id: u32) -> bool {
        self.ids.iter().any(|record| record.id == id)
    }
    fn update(&mut self, record: NotificationRecord, now: u64) {
        self.ids.retain(|member| member.id != record.id);
        self.ids.push_back(record.clone());
        self.record = record;
        self.latest = now;
    }
    fn remove(&mut self, id: u32) {
        self.ids.retain(|record| record.id != id);
        if let Some(record) = self.ids.back() {
            self.record = record.clone();
        }
    }
}
pub struct PresentationCoordinator {
    panel: Option<Panel>,
    pending: VecDeque<Card>,
    active: Option<Card>,
    volume_until: Option<u64>,
    media: bool,
    hover: bool,
    suppressed: bool,
    last: u64,
    dwell: u64,
}
impl Default for PresentationCoordinator {
    fn default() -> Self {
        Self {
            panel: None,
            pending: VecDeque::new(),
            active: None,
            volume_until: None,
            media: false,
            hover: false,
            suppressed: false,
            last: 0,
            dwell: 5000,
        }
    }
}
impl PresentationCoordinator {
    fn account(&mut self, now: u64) {
        // A delayed GUI wake can span the end of the volume chip. Only its
        // actual 1500 ms lifetime pauses the existing notification's dwell.
        if self.panel.is_none() && !self.suppressed && !self.hover {
            let visible_from = self.last.max(self.volume_until.unwrap_or(self.last));
            let delta = now.saturating_sub(visible_from);
            if let Some(card) = &mut self.active {
                let consumed = delta.min(card.remaining);
                card.remaining -= consumed;
                card.shown = card.shown.saturating_add(consumed);
            }
        }
        self.last = self.last.max(now);
    }
    pub fn advance(&mut self, now: u64) {
        self.account(now);
        if self.volume_until.is_some_and(|end| now >= end) {
            self.volume_until = None;
        }
        self.pending
            .retain(|card| now.saturating_sub(card.arrived) < 60_000);
        if self.active.as_ref().is_some_and(|card| {
            card.remaining == 0
                || ((self.panel.is_some() || self.suppressed)
                    && now.saturating_sub(card.arrived) >= 60_000)
        }) {
            self.active = None;
        }
        if self.active.is_none()
            && self.panel.is_none()
            && !self.suppressed
            && self.volume_until.is_none()
        {
            self.active = self.pending.pop_front();
        }
    }
    pub fn set_dwell(&mut self, seconds: u32, now: u64) {
        self.advance(now);
        self.dwell = seconds.clamp(3, 10) as u64 * 1000;
    }
    pub fn set_media(&mut self, media: bool, now: u64) {
        self.advance(now);
        self.media = media;
    }
    pub fn open(&mut self, panel: Panel, now: u64) {
        self.advance(now);
        if !matches!(panel, Panel::Notification(_)) {
            if let Some(card) = self.active.take() {
                self.pending.push_front(card);
            }
        } else {
            self.active = None;
        }
        self.panel = Some(panel);
    }
    pub fn expand_notification(&mut self, now: u64) {
        self.advance(now);
        if let Some(card) = self.active.take() {
            self.panel = Some(Panel::Notification(card.record));
        }
    }
    pub fn close(&mut self, now: u64) {
        self.advance(now);
        self.panel = None;
        self.advance(now);
    }
    pub fn panel_open(&self) -> bool {
        self.panel.is_some()
    }
    pub fn set_hover(&mut self, value: bool, now: u64) {
        self.advance(now);
        self.hover = value;
    }
    pub fn suppress(&mut self, value: bool, now: u64) {
        self.advance(now);
        self.suppressed = value;
        self.last = now;
    }
    pub fn volume(&mut self, now: u64) {
        self.advance(now);
        self.volume_until = Some(now.saturating_add(1500));
    }
    pub fn hide_notification(&mut self, now: u64) {
        self.advance(now);
        self.active = None;
        if matches!(self.panel, Some(Panel::Notification(_))) {
            self.panel = None;
        }
        self.advance(now);
    }
    pub fn clear_notifications(&mut self, now: u64) {
        self.advance(now);
        self.active = None;
        self.pending.clear();
        if matches!(self.panel, Some(Panel::Notification(_))) {
            self.panel = None;
        }
    }
    pub fn retain_apps(&mut self, blocked: &[String], now: u64) {
        self.advance(now);
        self.pending
            .retain(|card| !blocked.contains(&card.record.app_id));
        if self
            .active
            .as_ref()
            .is_some_and(|card| blocked.contains(&card.record.app_id))
        {
            self.active = None;
        }
        if matches!(&self.panel,Some(Panel::Notification(record))if blocked.contains(&record.app_id))
        {
            self.panel = None;
        }
        self.advance(now);
    }
    pub fn remove(&mut self, id: u32, now: u64) {
        self.advance(now);
        self.pending.retain_mut(|card| {
            card.remove(id);
            !card.ids.is_empty()
        });
        if let Some(card) = &mut self.active {
            card.remove(id);
            if card.ids.is_empty() {
                self.active = None;
            }
        }
        if matches!(&self.panel,Some(Panel::Notification(record))if record.id==id) {
            self.panel = None;
        }
        self.advance(now);
    }
    fn bound_ids(&mut self) {
        // Limit notification identities, not just coalesced cards: a busy app
        // must not accumulate an unbounded ID set inside a single queue entry.
        while self.pending.iter().map(|c| c.ids.len()).sum::<usize>()
            + self.active.as_ref().map_or(0, |c| c.ids.len())
            > 20
        {
            // Keep the currently visible latest record stable. Older members
            // of its group can be retired before rejecting a new app entirely.
            if let Some(card) = self.active.as_mut().filter(|card| card.ids.len() > 1) {
                card.ids.pop_front();
            } else if let Some(card) = self.pending.front_mut() {
                card.ids.pop_front();
                if let Some(record) = card.ids.back() {
                    card.record = record.clone();
                }
                if card.ids.is_empty() {
                    self.pending.pop_front();
                }
            } else if let Some(card) = &mut self.active {
                card.ids.pop_front();
                if let Some(record) = card.ids.back() {
                    card.record = record.clone();
                }
                if card.ids.is_empty() {
                    self.active = None;
                }
            } else {
                break;
            }
        }
    }
    pub fn push(&mut self, record: NotificationRecord, now: u64) {
        self.advance(now);
        if matches!(&self.panel,Some(Panel::Notification(pinned))if pinned.id==record.id) {
            return;
        }
        // Identity lookup precedes grouping. Otherwise an update could be merged
        // into the active app group while its old ID remains queued elsewhere.
        if self
            .active
            .as_ref()
            .is_some_and(|c| c.contains_id(record.id))
        {
            let card = self.active.as_mut().unwrap();
            card.update(record, now);
            card.remaining = self.dwell.min(15_000u64.saturating_sub(card.shown));
        } else if let Some(card) = self.pending.iter_mut().find(|c| c.contains_id(record.id)) {
            card.update(record, now);
        } else if let Some(card) = self
            .active
            .as_mut()
            .filter(|c| c.record.app_id == record.app_id && now.saturating_sub(c.latest) <= 3000)
        {
            card.update(record, now);
            card.remaining = self.dwell.min(15_000u64.saturating_sub(card.shown));
        } else if let Some(card) = self
            .pending
            .iter_mut()
            .find(|c| c.record.app_id == record.app_id && now.saturating_sub(c.latest) <= 3000)
        {
            card.update(record, now);
        } else {
            self.pending.push_back(Card::new(record, now, self.dwell));
        }
        self.bound_ids();
        self.advance(now);
    }
    pub fn view(&mut self, now: u64) -> Presentation {
        self.advance(now);
        let pending = self.pending.iter().map(|c| c.ids.len()).sum::<usize>()
            + if self.panel.is_some() {
                self.active.as_ref().map_or(0, |c| c.ids.len())
            } else {
                0
            };
        let volume_chip = self.panel.is_some() && self.volume_until.is_some();
        let content = if let Some(panel) = &self.panel {
            match panel {
                Panel::Launcher => Content::Launcher,
                Panel::Media => Content::Media,
                Panel::Volume => Content::VolumePanel,
                Panel::Notification(record) => Content::ExpandedNotification(record.clone()),
            }
        } else if self.suppressed {
            if self.media {
                Content::CompactMedia
            } else {
                Content::Idle
            }
        } else if self.volume_until.is_some() {
            Content::Volume
        } else if let Some(card) = &self.active {
            Content::Notification(card.record.clone(), card.ids.len() as u32)
        } else if self.media {
            Content::CompactMedia
        } else {
            Content::Idle
        };
        Presentation {
            content,
            pending,
            volume_chip,
        }
    }
    pub fn next_deadline(&self, now: u64) -> Option<u64> {
        let mut deadlines = Vec::new();
        if let Some(end) = self.volume_until {
            deadlines.push(end);
        }
        for card in &self.pending {
            deadlines.push(card.arrived.saturating_add(60_000));
        }
        if let Some(card) = &self.active {
            if self.suppressed || self.panel.is_some() {
                deadlines.push(card.arrived.saturating_add(60_000));
            }
            if self.panel.is_none()
                && !self.suppressed
                && !self.hover
                && self.volume_until.is_none()
            {
                deadlines.push(now.saturating_add(card.remaining));
            }
        }
        deadlines
            .into_iter()
            .min()
            .map(|v| v.max(now.saturating_add(1)))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn note(id: u32, app: &str) -> NotificationRecord {
        NotificationRecord {
            id,
            app_id: app.into(),
            app_name: app.into(),
            title: format!("Notice{id}"),
            body: "body".into(),
            created_ticks: 0,
            icon: None,
        }
    }
    #[test]
    fn protects_panel_and_restores_base() {
        let mut c = PresentationCoordinator::default();
        c.set_media(true, 0);
        c.open(Panel::Media, 0);
        c.push(note(1, "a"), 10);
        assert!(matches!(c.view(10).content, Content::Media));
        assert_eq!(c.view(10).pending, 1);
        c.close(100);
        assert!(matches!(c.view(100).content, Content::Notification(..)));
        assert!(matches!(c.view(5100).content, Content::CompactMedia));
    }
    #[test]
    fn volume_interrupts_and_preserves_notification_time() {
        let mut c = PresentationCoordinator::default();
        c.push(note(1, "a"), 0);
        c.volume(1000);
        assert_eq!(c.view(1000).content, Content::Volume);
        c.advance(2500);
        assert!(matches!(c.view(2500).content, Content::Notification(..)));
        assert!(matches!(c.view(6499).content, Content::Notification(..)));
        assert_eq!(c.view(6500).content, Content::Idle);
    }
    #[test]
    fn hover_pauses_and_queue_expires() {
        let mut c = PresentationCoordinator::default();
        c.push(note(1, "a"), 0);
        c.set_hover(true, 1000);
        assert!(c.next_deadline(1000).is_none());
        c.set_hover(false, 100_000);
        assert!(matches!(c.view(103999).content, Content::Notification(..)));
        assert_eq!(c.view(104000).content, Content::Idle);
        c.open(Panel::Volume, 104001);
        c.push(note(2, "b"), 104002);
        assert_eq!(c.view(164002).pending, 0);
    }
    #[test]
    fn grouping_updates_and_removal() {
        let mut c = PresentationCoordinator::default();
        c.push(note(1, "a"), 0);
        c.push(note(2, "a"), 100);
        assert!(matches!(c.view(100).content, Content::Notification(_, 2)));
        c.push(note(2, "a"), 200);
        assert!(matches!(c.view(200).content, Content::Notification(_, 2)));
        c.remove(1, 300);
        c.remove(2, 400);
        assert_eq!(c.view(400).content, Content::Idle);
    }
    #[test]
    fn bounded_queue_and_permission_clear() {
        let mut c = PresentationCoordinator::default();
        c.open(Panel::Launcher, 0);
        for id in 0..100 {
            c.push(note(id, &id.to_string()), id as u64);
        }
        assert_eq!(c.view(100).pending, 20);
        c.clear_notifications(100);
        assert_eq!(c.view(100).pending, 0);
    }
    #[test]
    fn pinned_notification_is_a_stable_reading_snapshot() {
        let mut c = PresentationCoordinator::default();
        c.push(note(1, "a"), 0);
        c.expand_notification(1);
        let old = c.view(1);
        let mut update = note(1, "a");
        update.title = "changed".into();
        c.push(update, 2);
        assert_eq!(old.content, c.view(2).content);
        c.hide_notification(3);
        assert_eq!(c.view(3).content, Content::Idle);
    }
}
#[cfg(test)]
mod arbitration_regressions {
    use super::*;
    fn note(id: u32, app: &str) -> NotificationRecord {
        NotificationRecord {
            id,
            app_id: app.into(),
            app_name: app.into(),
            title: format!("Notice{id}"),
            body: "body".into(),
            created_ticks: 0,
            icon: None,
        }
    }
    fn stored(c: &PresentationCoordinator) -> usize {
        c.pending.iter().map(|card| card.ids.len()).sum::<usize>()
            + c.active.as_ref().map_or(0, |card| card.ids.len())
    }
    #[test]
    fn delayed_volume_callback_charges_only_post_chip_time() {
        let mut c = PresentationCoordinator::default();
        c.push(note(1, "a"), 0);
        c.volume(1000);
        c.advance(4500);
        assert_eq!(c.active.as_ref().unwrap().remaining, 2000);
        assert_eq!(c.next_deadline(4500), Some(6500));
        assert!(matches!(c.view(6499).content, Content::Notification(..)));
        assert_eq!(c.view(6500).content, Content::Idle);
    }
    #[test]
    fn volume_retrigger_extends_pause_without_losing_dwell() {
        let mut c = PresentationCoordinator::default();
        c.push(note(1, "a"), 0);
        c.volume(1000);
        c.volume(2000);
        c.advance(4500);
        assert_eq!(c.active.as_ref().unwrap().remaining, 3000);
        assert_eq!(c.next_deadline(4500), Some(7500));
    }
    #[test]
    fn coalesced_identity_count_is_globally_bounded() {
        let mut c = PresentationCoordinator::default();
        c.open(Panel::Launcher, 0);
        for id in 0..200 {
            c.push(note(id, "busy"), id as u64);
            assert!(stored(&c) <= 20);
        }
        assert_eq!(c.view(200).pending, 20);
        assert_eq!(c.pending.len(), 1);
        assert!(!c.pending[0].contains_id(0));
        assert!(c.pending[0].contains_id(199));
        c.close(201);
        assert!(matches!(c.view(201).content, Content::Notification(_, 20)));
    }
    #[test]
    fn updates_do_not_duplicate_identity_between_groups() {
        let mut c = PresentationCoordinator::default();
        c.push(note(1, "active"), 0);
        c.push(note(2, "queued"), 1);
        c.push(note(2, "active"), 2);
        assert_eq!(stored(&c), 2);
        assert_eq!(c.active.as_ref().unwrap().ids.len(), 1);
        assert_eq!(c.pending.len(), 1);
        c.remove(2, 3);
        assert_eq!(stored(&c), 1);
    }
    #[test]
    fn removal_of_latest_group_member_restores_surviving_record() {
        let mut c = PresentationCoordinator::default();
        c.push(note(1, "a"), 0);
        c.push(note(2, "a"), 1);
        c.remove(2, 2);
        assert!(matches!(c.view(2).content,Content::Notification(record,1)if record.id==1));
    }
    #[test]
    fn suppression_expires_active_and_queued_but_hover_preserves_reading() {
        let mut c = PresentationCoordinator::default();
        c.push(note(1, "a"), 0);
        c.set_hover(true, 1000);
        c.suppress(true, 2000);
        c.push(note(2, "b"), 3000);
        assert_eq!(c.next_deadline(3000), Some(60000));
        c.advance(60000);
        assert!(c.active.is_none());
        assert_eq!(c.next_deadline(60000), Some(63000));
        c.advance(63000);
        assert_eq!(stored(&c), 0);
        c.suppress(false, 64000);
        assert_eq!(c.view(64000).content, Content::Idle);
    }
    #[test]
    fn pinned_snapshot_survives_ttl_and_volume_while_incoming_expires() {
        let mut c = PresentationCoordinator::default();
        c.push(note(1, "a"), 0);
        c.expand_notification(1);
        c.volume(10);
        c.push(note(2, "a"), 20);
        c.set_hover(true, 30);
        assert!(c.view(30).volume_chip);
        assert_eq!(c.view(60020).pending, 0);
        assert!(
            matches!(c.view(60020).content,Content::ExpandedNotification(record)if record.id==1)
        );
        c.remove(1, 60021);
        assert_eq!(c.view(60021).content, Content::Idle);
    }
    #[test]
    fn block_list_removes_active_queued_and_pinned_records() {
        let mut c = PresentationCoordinator::default();
        c.push(note(1, "blocked"), 0);
        c.push(note(2, "other"), 1);
        c.retain_apps(&["blocked".into()], 2);
        assert!(matches!(c.view(2).content,Content::Notification(record,_)if record.id==2));
        c.expand_notification(3);
        c.push(note(3, "other"), 4);
        c.retain_apps(&["other".into()], 5);
        assert_eq!(c.view(5).content, Content::Idle);
        assert_eq!(stored(&c), 0);
    }
    #[test]
    fn incoming_during_volume_starts_dwell_when_first_presented() {
        let mut c = PresentationCoordinator::default();
        c.volume(0);
        c.push(note(1, "a"), 10);
        assert_eq!(c.view(10).content, Content::Volume);
        c.advance(2500);
        assert_eq!(c.active.as_ref().unwrap().remaining, 5000);
        assert_eq!(c.next_deadline(2500), Some(7500));
    }
}
#[cfg(test)]
mod capacity_regression {
    use super::*;
    #[test]
    fn full_active_group_leaves_room_for_new_app_without_preempting() {
        let note = |id, app: &str| NotificationRecord {
            id,
            app_id: app.into(),
            app_name: app.into(),
            title: id.to_string(),
            body: String::new(),
            created_ticks: 0,
            icon: None,
        };
        let mut c = PresentationCoordinator::default();
        for id in 0..20 {
            c.push(note(id, "a"), id as u64);
        }
        c.push(note(20, "b"), 20);
        assert!(matches!(c.view(20).content,Content::Notification(record,19)if record.id==19));
        assert_eq!(c.view(20).pending, 1);
        assert_eq!(c.pending[0].record.id, 20);
    }
}
