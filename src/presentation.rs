//! Pure presentation arbitration. Clock values are supplied by the GUI, never read
//! here, so precedence, expiration, and interruption tests are deterministic.
use crate::navigation::Page;
use crate::services::battery::BatteryActivity;
use crate::{services::notifications::NotificationRecord, state::IslandState};
use std::collections::VecDeque;
#[derive(Clone, Debug, PartialEq)]
pub struct PluginActivity {
    pub plugin_id: String,
    pub id: String,
    pub title: String,
    pub body: String,
    pub accent: String,
}
struct PluginPreview {
    activity: PluginActivity,
    expires: u64,
    visible_until: Option<u64>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Panel {
    AppLauncher,
    Launcher,
    Media,
    Timer,
    Volume,
    Battery,
    Plugins,
    Notification(NotificationRecord),
}
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    AppLauncher,
    Idle,
    CompactMedia,
    CompactTimer,
    CompactBattery(BatteryActivity),
    Volume,
    Launcher,
    Media,
    Timer,
    TimerFinished,
    VolumePanel,
    BatteryPanel,
    PluginPanel,
    CompactPlugin(PluginActivity),
    CompactPluginPage { plugin_id: String, view_id: String },
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
            Content::AppLauncher => IslandState::AppLauncher,
            Content::Idle => IslandState::Idle,
            Content::CompactMedia
            | Content::CompactTimer
            | Content::CompactBattery(_)
            | Content::CompactPlugin(_)
            | Content::CompactPluginPage { .. }
            | Content::Volume
            | Content::Notification(..) => IslandState::Compact,
            Content::Launcher => IslandState::Expanded,
            Content::Media => IslandState::Media,
            Content::Timer | Content::TimerFinished => IslandState::Timer,
            Content::VolumePanel => IslandState::Volume,
            Content::BatteryPanel => IslandState::Battery,
            Content::PluginPanel => IslandState::Plugin,
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
struct BatteryPreview {
    activity: BatteryActivity,
    expires: u64,
    visible_until: Option<u64>,
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
    selected_page: Option<Page>,
    panel: Option<Panel>,
    hover_panel: bool,
    pinned: bool,
    pending: VecDeque<Card>,
    active: Option<Card>,
    volume_until: Option<u64>,
    battery: Option<BatteryPreview>,
    plugins: VecDeque<PluginPreview>,
    plugin_cooldowns: std::collections::HashMap<String, u64>,
    media: bool,
    timer_active: bool,
    timer_finished_until: Option<u64>,
    hover: bool,
    suppressed: bool,
    last: u64,
    dwell: u64,
}
impl Default for PresentationCoordinator {
    fn default() -> Self {
        Self {
            selected_page: None,
            panel: None,
            hover_panel: false,
            pinned: false,
            pending: VecDeque::new(),
            active: None,
            volume_until: None,
            battery: None,
            plugins: VecDeque::new(),
            plugin_cooldowns: std::collections::HashMap::new(),
            media: false,
            timer_active: false,
            timer_finished_until: None,
            hover: false,
            suppressed: false,
            last: 0,
            dwell: 5000,
        }
    }
}
impl PresentationCoordinator {
    pub fn selected_page(&self) -> Option<&Page> {
        self.selected_page.as_ref()
    }
    /// None preserves automatic activity arbitration for existing installations.
    pub fn set_selected_page(&mut self, page: Option<Page>, now: u64) {
        self.advance(now);
        self.selected_page = page.filter(Page::is_valid);
    }
    /// Page navigation changes the remembered destination, not alert ownership.
    /// Like hovering, navigation collapses on leave unless explicitly pinned.
    pub fn navigate_page(&mut self, page: Page, now: u64) {
        if !page.is_valid() {
            return;
        }
        let panel = Self::page_panel(&page);
        self.set_selected_page(Some(page), now);
        self.open(panel, now);
        self.hover_panel = true;
    }
    fn page_panel(page: &Page) -> Panel {
        match page {
            Page::Home => Panel::Launcher,
            Page::Media => Panel::Media,
            Page::Timer => Panel::Timer,
            Page::Volume => Panel::Volume,
            Page::Plugin { .. } => Panel::Plugins,
        }
    }
    fn selected_compact(&self) -> Option<Content> {
        self.selected_page.as_ref().map(|page| match page {
            Page::Home => Content::Idle,
            Page::Media => Content::CompactMedia,
            Page::Timer => Content::CompactTimer,
            Page::Volume => Content::Volume,
            Page::Plugin { id, view } => Content::CompactPluginPage {
                plugin_id: id.clone(),
                view_id: view.clone(),
            },
        })
    }
    fn account(&mut self, now: u64) {
        // Notifications remain visible above feedback, so volume changes must
        // not pause their dwell. User panels, suppression and reading still do.
        if self.panel.is_none() && !self.suppressed && !self.hover {
            let delta = now.saturating_sub(self.last);
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
        self.plugins
            .retain(|p| now < p.visible_until.unwrap_or(p.expires));
        if self.volume_until.is_some_and(|end| now >= end) {
            self.volume_until = None;
        }
        if self.timer_finished_until.is_some_and(|end| now >= end) {
            self.timer_finished_until = None;
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
        if self.active.is_none() && self.panel.is_none() && !self.suppressed {
            self.active = self.pending.pop_front();
        }
        if self
            .battery
            .as_ref()
            .is_some_and(|preview| now >= preview.visible_until.unwrap_or(preview.expires))
        {
            self.battery = None;
        }
        if self.panel.is_none()
            && !self.suppressed
            && self.active.is_none()
            && self.timer_finished_until.is_none()
            && let Some(preview) = &mut self.battery
            && preview.visible_until.is_none()
        {
            preview.visible_until = Some(now.saturating_add(3000));
        }
    }
    pub fn battery(&mut self, activity: BatteryActivity, now: u64) {
        self.advance(now);
        self.battery = Some(BatteryPreview {
            activity,
            expires: now.saturating_add(10_000),
            visible_until: None,
        });
        self.advance(now);
    }
    pub fn plugin_activity(&mut self, activity: PluginActivity, now: u64) -> bool {
        self.advance(now);
        if self
            .plugin_cooldowns
            .get(&activity.plugin_id)
            .is_some_and(|at| now.saturating_sub(*at) < 30_000)
        {
            return false;
        }
        self.plugin_cooldowns
            .insert(activity.plugin_id.clone(), now);
        self.plugins
            .retain(|p| p.activity.plugin_id != activity.plugin_id);
        if self.plugins.len() == 16 {
            self.plugins.pop_front();
        }
        self.plugins.push_back(PluginPreview {
            activity,
            expires: now.saturating_add(10_000),
            visible_until: None,
        });
        true
    }
    pub fn clear_plugin(&mut self, id: &str, now: u64) {
        self.advance(now);
        self.plugins.retain(|p| p.activity.plugin_id != id);
        self.plugin_cooldowns.remove(id);
    }
    pub fn clear_battery(&mut self, now: u64) {
        self.advance(now);
        self.battery = None;
    }
    pub fn set_dwell(&mut self, seconds: u32, now: u64) {
        self.advance(now);
        self.dwell = seconds.clamp(3, 10) as u64 * 1000;
    }
    pub fn set_media(&mut self, media: bool, now: u64) {
        self.advance(now);
        self.media = media;
    }
    pub fn set_timer(&mut self, active: bool, finished: bool, now: u64) {
        self.advance(now);
        self.timer_active = active;
        if finished {
            self.timer_finished_until = Some(now.saturating_add(2500));
        } else if active {
            self.timer_finished_until = None;
        }
    }
    /// Explicit reset/configuration cancels both compact activity and any
    /// outstanding completion preview. Routine idle snapshots do not dismiss it.
    pub fn clear_timer(&mut self, now: u64) {
        self.advance(now);
        self.timer_active = false;
        self.timer_finished_until = None;
    }
    pub fn open(&mut self, panel: Panel, now: u64) {
        self.hover_panel = false;
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
    /// A hover preview is temporary; explicit navigation takes ownership via `open`.
    pub fn open_hover(&mut self, now: u64) {
        if self.panel.is_none() {
            self.open(
                if let Some(page) = &self.selected_page {
                    Self::page_panel(page)
                } else if self
                    .battery
                    .as_ref()
                    .is_some_and(|preview| preview.visible_until.is_some())
                {
                    Panel::Battery
                } else if self.timer_finished_until.is_some() {
                    Panel::Timer
                } else if self.media {
                    Panel::Media
                } else if self.timer_active {
                    Panel::Timer
                } else {
                    Panel::Launcher
                },
                now,
            );
            self.hover_panel = true;
        }
    }
    pub fn close_hover(&mut self, now: u64) {
        if self.hover_panel && !self.pinned() {
            self.close(now);
        }
    }
    pub fn pinned(&self) -> bool {
        self.panel.is_some() && self.pinned
    }
    /// Pins the current panel without discarding whether hover originally
    /// opened it. Unpinning can therefore resume its ordinary hover lifetime.
    pub fn toggle_pin(&mut self, now: u64) {
        self.advance(now);
        if self.panel.is_some() {
            self.pinned = !self.pinned;
        }
    }
    pub fn expand_notification(&mut self, now: u64) {
        self.advance(now);
        if let Some(card) = self.active.take() {
            self.hover_panel = false;
            self.panel = Some(Panel::Notification(card.record));
        }
    }
    pub fn close(&mut self, now: u64) {
        self.advance(now);
        if matches!(self.panel, Some(Panel::Battery)) {
            self.battery = None;
        }
        self.panel = None;
        self.hover_panel = false;
        self.pinned = false;
        self.timer_finished_until = None;
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
            self.pinned = false;
        }
        self.advance(now);
    }
    pub fn clear_notifications(&mut self, now: u64) {
        self.advance(now);
        self.active = None;
        self.pending.clear();
        if matches!(self.panel, Some(Panel::Notification(_))) {
            self.panel = None;
            self.pinned = false;
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
            self.pinned = false;
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
            self.pinned = false;
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
        if self.panel.is_none()
            && !self.suppressed
            && self.active.is_none()
            && self.timer_finished_until.is_none()
            && self.battery.is_none()
            && let Some(preview) = self.plugins.front_mut()
            && preview.visible_until.is_none()
        {
            preview.visible_until = Some(now.saturating_add(3000).min(preview.expires));
        }
        let pending = self.pending.iter().map(|c| c.ids.len()).sum::<usize>()
            + if self.panel.is_some() {
                self.active.as_ref().map_or(0, |c| c.ids.len())
            } else {
                0
            };
        let volume_chip = self.panel.is_some() && self.volume_until.is_some();
        let content = if let Some(panel) = &self.panel {
            match panel {
                Panel::AppLauncher => Content::AppLauncher,
                Panel::Launcher => Content::Launcher,
                Panel::Media => Content::Media,
                Panel::Timer => Content::Timer,
                Panel::Volume => Content::VolumePanel,
                Panel::Battery => Content::BatteryPanel,
                Panel::Plugins => Content::PluginPanel,
                Panel::Notification(record) => Content::ExpandedNotification(record.clone()),
            }
        } else if self.suppressed {
            self.base_content()
        } else if let Some(card) = &self.active {
            Content::Notification(card.record.clone(), card.ids.len() as u32)
        } else if self.timer_finished_until.is_some() {
            Content::TimerFinished
        } else if let Some(preview) = &self.battery {
            Content::CompactBattery(preview.activity)
        } else if let Some(preview) = self.plugins.front() {
            Content::CompactPlugin(preview.activity.clone())
        } else if self.volume_until.is_some() {
            Content::Volume
        } else {
            self.base_content()
        };
        Presentation {
            content,
            pending,
            volume_chip,
        }
    }
    /// Contracted priority below notifications, transient events and feedback.
    /// Activity never mutates the user's remembered expanded destination.
    fn base_content(&self) -> Content {
        if self.media {
            Content::CompactMedia
        } else if self.timer_active {
            Content::CompactTimer
        } else {
            self.selected_compact().unwrap_or(Content::Idle)
        }
    }
    pub fn next_deadline(&self, now: u64) -> Option<u64> {
        let mut deadline = None;
        let mut consider = |candidate| {
            deadline = Some(deadline.map_or(candidate, |previous: u64| previous.min(candidate)));
        };
        for preview in &self.plugins {
            consider(preview.visible_until.unwrap_or(preview.expires));
        }
        if let Some(end) = self.volume_until {
            consider(end);
        }
        if let Some(end) = self.timer_finished_until {
            consider(end);
        }
        if let Some(preview) = &self.battery {
            consider(preview.visible_until.unwrap_or(preview.expires));
        }
        for card in &self.pending {
            consider(card.arrived.saturating_add(60_000));
        }
        if let Some(card) = &self.active {
            if self.suppressed || self.panel.is_some() {
                consider(card.arrived.saturating_add(60_000));
            }
            if self.panel.is_none() && !self.suppressed && !self.hover {
                consider(now.saturating_add(card.remaining));
            }
        }
        deadline.map(|v| v.max(now.saturating_add(1)))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn plugin(id: &str) -> PluginActivity {
        PluginActivity {
            plugin_id: id.into(),
            id: "event".into(),
            title: "Build".into(),
            body: "Done".into(),
            accent: "Green".into(),
        }
    }
    #[test]
    fn plugin_cooldown_expiration_and_restoration() {
        let mut c = PresentationCoordinator::default();
        c.set_media(true, 0);
        assert!(c.plugin_activity(plugin("test"), 10));
        assert!(matches!(c.view(10).content, Content::CompactPlugin(_)));
        assert!(!c.plugin_activity(plugin("test"), 100));
        assert_eq!(c.view(3010).content, Content::CompactMedia);
        assert!(c.plugin_activity(plugin("test"), 30_010));
        c.clear_plugin("test", 30_011);
        assert_eq!(c.view(30_011).content, Content::CompactMedia);
    }
    #[test]
    fn plugin_defers_behind_user_panel_and_expires() {
        let mut c = PresentationCoordinator::default();
        c.open(Panel::Media, 0);
        c.toggle_pin(1);
        c.plugin_activity(plugin("test"), 20);
        assert_eq!(c.view(5000).content, Content::Media);
        c.close(11_000);
        assert_eq!(c.view(11_000).content, Content::Idle);
        c.open(Panel::Volume, 12_000);
        c.plugin_activity(plugin("other"), 12_001);
        c.close(12_500);
        assert!(matches!(c.view(12_500).content, Content::CompactPlugin(_)));
    }
    #[test]
    fn temporary_plugin_event_stays_above_volume_feedback() {
        let mut c = PresentationCoordinator::default();
        c.plugin_activity(plugin("test"), 0);
        c.view(0);
        c.volume(100);
        assert!(matches!(c.view(100).content, Content::CompactPlugin(_)));
        assert!(matches!(c.view(1600).content, Content::CompactPlugin(_)));
        assert_eq!(c.view(3000).content, Content::Idle);
    }
    fn battery_event() -> BatteryActivity {
        BatteryActivity {
            snapshot: crate::services::battery::BatterySnapshot::from_raw(1, 8, 50),
            kind: crate::services::battery::BatteryEventKind::Connected,
        }
    }
    #[test]
    fn battery_preview_restores_media_and_timer_base() {
        let mut c = PresentationCoordinator::default();
        c.set_media(true, 0);
        c.battery(battery_event(), 10);
        assert!(matches!(c.view(10).content, Content::CompactBattery(_)));
        assert_eq!(c.next_deadline(10), Some(3010));
        assert_eq!(c.view(3010).content, Content::CompactMedia);
        c.set_timer(true, false, 3100);
        c.set_media(false, 3100);
        c.battery(battery_event(), 3200);
        assert_eq!(c.view(6200).content, Content::CompactTimer);
    }
    #[test]
    fn battery_defers_behind_panels_and_expires_without_replay() {
        let mut c = PresentationCoordinator::default();
        c.open(Panel::Media, 0);
        c.toggle_pin(1);
        c.battery(battery_event(), 10);
        assert_eq!(c.view(100).content, Content::Media);
        c.close(200);
        assert!(matches!(c.view(200).content, Content::CompactBattery(_)));
        c.open(Panel::Media, 4000);
        c.battery(battery_event(), 4010);
        c.close(14_011);
        assert_eq!(c.view(14_011).content, Content::Idle);
    }
    #[test]
    fn battery_waits_for_timer_completion_and_suppression() {
        let mut c = PresentationCoordinator::default();
        c.set_timer(false, true, 0);
        c.battery(battery_event(), 10);
        assert_eq!(c.view(100).content, Content::TimerFinished);
        assert!(matches!(c.view(2500).content, Content::CompactBattery(_)));
        c.clear_battery(2600);
        assert_eq!(c.view(2600).content, Content::Idle);
        c.suppress(true, 2700);
        c.battery(battery_event(), 2800);
        assert_eq!(c.view(2900).content, Content::Idle);
        c.suppress(false, 3000);
        assert!(matches!(c.view(3000).content, Content::CompactBattery(_)));
    }
    #[test]
    fn battery_hover_opens_details_and_close_dismisses_preview() {
        let mut c = PresentationCoordinator::default();
        c.battery(battery_event(), 0);
        c.open_hover(10);
        assert_eq!(c.view(10).content, Content::BatteryPanel);
        c.close(20);
        assert_eq!(c.view(20).content, Content::Idle);
    }
    #[test]
    fn close_dismisses_completion_without_hiding_active_countdown() {
        let mut c = PresentationCoordinator::default();
        c.set_timer(false, true, 100);
        c.close(200);
        assert_eq!(c.view(200).content, Content::Idle);
        c.set_timer(true, false, 300);
        c.open(Panel::Timer, 400);
        c.close(500);
        assert_eq!(c.view(500).content, Content::CompactTimer);
    }
    #[test]
    fn resetting_timer_clears_its_completion_preview_and_deadline() {
        let mut c = PresentationCoordinator::default();
        c.set_media(true, 0);
        c.set_timer(false, true, 10);
        assert_eq!(c.view(10).content, Content::TimerFinished);
        c.clear_timer(20);
        assert_eq!(c.view(20).content, Content::CompactMedia);
        assert_eq!(c.next_deadline(20), None);
        c.set_timer(true, false, 30);
        c.clear_timer(40);
        assert_eq!(c.view(40).content, Content::CompactMedia);
    }
    #[test]
    fn pinned_hover_panel_only_closes_after_unpin_or_explicit_close() {
        let mut c = PresentationCoordinator::default();
        c.toggle_pin(0);
        assert!(!c.pinned());
        c.open_hover(10);
        c.toggle_pin(20);
        assert!(c.pinned());
        c.close_hover(30);
        assert_eq!(c.view(30).content, Content::Launcher);
        c.toggle_pin(40);
        c.close_hover(50);
        assert_eq!(c.view(50).content, Content::Idle);
        c.open_hover(60);
        c.toggle_pin(70);
        c.close(80);
        assert!(!c.pinned());
        c.open_hover(90);
        c.close_hover(100);
        assert_eq!(c.view(100).content, Content::Idle);
    }

    #[test]
    fn pin_survives_navigation_but_notification_removal_clears_it() {
        let mut c = PresentationCoordinator::default();
        c.open_hover(0);
        c.toggle_pin(10);
        c.open(Panel::Media, 20);
        assert!(c.pinned());
        c.close_hover(30);
        assert_eq!(c.view(30).content, Content::Media);
        c.close(40);
        c.push(note(1, "Messages"), 50);
        c.expand_notification(60);
        c.toggle_pin(70);
        c.remove(1, 80);
        assert!(!c.pinned());
        c.open_hover(90);
        c.close_hover(100);
        assert!(!c.panel_open());
    }

    #[test]
    fn timer_runs_compact_then_completes_and_restores_media() {
        let mut c = PresentationCoordinator::default();
        c.set_media(true, 0);
        c.set_timer(true, false, 10);
        assert_eq!(c.view(20).content, Content::CompactMedia);
        c.open_hover(30);
        assert_eq!(c.view(30).content, Content::Media);
        c.close_hover(40);
        c.set_timer(false, true, 50);
        assert_eq!(c.view(50).content, Content::TimerFinished);
        assert_eq!(c.next_deadline(50), Some(2550));
        assert_eq!(c.view(2549).content, Content::TimerFinished);
        assert_eq!(c.view(2550).content, Content::CompactMedia);
        assert_eq!(c.next_deadline(2550), None);
    }

    #[test]
    fn timer_completion_does_not_preempt_panels_notifications_or_suppression() {
        let mut c = PresentationCoordinator::default();
        c.open(Panel::Media, 0);
        c.set_timer(false, true, 10);
        assert_eq!(c.view(10).content, Content::Media);
        c.close(20);
        c.push(note(1, "Messages"), 30);
        assert!(matches!(c.view(30).content, Content::Notification(..)));
        c.clear_notifications(40);
        c.suppress(true, 50);
        assert_eq!(c.view(50).content, Content::Idle);
        c.set_timer(true, false, 60);
        assert_eq!(c.view(60).content, Content::CompactTimer);
        c.suppress(false, 70);
        c.volume(80);
        assert_eq!(c.view(80).content, Content::Volume);
        assert_eq!(c.view(1580).content, Content::CompactTimer);
    }

    #[test]
    fn hover_preview_returns_to_idle_or_now_playing() {
        let mut c = PresentationCoordinator::default();
        c.open_hover(0);
        assert_eq!(c.view(0).content, Content::Launcher);
        c.close_hover(10);
        assert_eq!(c.view(10).content, Content::Idle);
        c.set_media(true, 20);
        for now in 30..50 {
            c.open_hover(now);
            assert_eq!(c.view(now).content, Content::Media);
            c.close_hover(now);
            assert_eq!(c.view(now).content, Content::CompactMedia);
        }
    }

    #[test]
    fn explicit_navigation_survives_pointer_leaving() {
        let mut c = PresentationCoordinator::default();
        c.open_hover(0);
        c.open(Panel::Volume, 10);
        c.close_hover(20);
        assert_eq!(c.view(20).content, Content::VolumePanel);
        c.close(30);
        c.close_hover(40);
        assert_eq!(c.view(40).content, Content::Idle);
    }

    #[test]
    fn hover_preview_restores_interrupted_notification() {
        let mut c = PresentationCoordinator::default();
        c.push(note(1, "Messages"), 0);
        c.open_hover(100);
        c.close_hover(200);
        assert!(matches!(c.view(200).content, Content::Notification(..)));
    }
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
    fn notification_stays_above_volume_without_extending_dwell() {
        let mut c = PresentationCoordinator::default();
        c.push(note(1, "a"), 0);
        c.volume(1000);
        assert!(matches!(c.view(1000).content, Content::Notification(..)));
        c.advance(2500);
        assert!(matches!(c.view(2500).content, Content::Notification(..)));
        assert!(matches!(c.view(4999).content, Content::Notification(..)));
        assert_eq!(c.view(5000).content, Content::Idle);
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
    fn delayed_volume_callback_charges_all_visible_notification_time() {
        let mut c = PresentationCoordinator::default();
        c.push(note(1, "a"), 0);
        c.volume(1000);
        c.advance(4500);
        assert_eq!(c.active.as_ref().unwrap().remaining, 500);
        assert_eq!(c.next_deadline(4500), Some(5000));
        assert!(matches!(c.view(4999).content, Content::Notification(..)));
        assert_eq!(c.view(5000).content, Content::Idle);
    }
    #[test]
    fn repeated_volume_feedback_does_not_starve_notifications() {
        let mut c = PresentationCoordinator::default();
        c.push(note(1, "a"), 0);
        c.volume(1000);
        c.volume(2000);
        c.advance(4500);
        assert_eq!(c.active.as_ref().unwrap().remaining, 500);
        assert_eq!(c.next_deadline(4500), Some(5000));
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
    fn incoming_notification_immediately_preempts_volume() {
        let mut c = PresentationCoordinator::default();
        c.volume(0);
        c.push(note(1, "a"), 10);
        assert!(matches!(c.view(10).content, Content::Notification(..)));
        c.advance(2500);
        assert_eq!(c.active.as_ref().unwrap().remaining, 2510);
        assert_eq!(c.next_deadline(2500), Some(5010));
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

#[cfg(test)]
mod navigation_tests {
    use super::*;
    fn plugin_page() -> Page {
        Page::Plugin {
            id: "org.stillisland.codex-usage".into(),
            view: "main".into(),
        }
    }
    fn compact_plugin() -> Content {
        Content::CompactPluginPage {
            plugin_id: "org.stillisland.codex-usage".into(),
            view_id: "main".into(),
        }
    }
    fn notification() -> NotificationRecord {
        NotificationRecord {
            id: 1,
            app_id: "fixture".into(),
            app_name: "Fixture".into(),
            title: "Notice".into(),
            body: "Body".into(),
            created_ticks: 0,
            icon: None,
        }
    }
    #[test]
    fn contracted_priority_unwinds_without_changing_remembered_page() {
        let mut c = PresentationCoordinator::default();
        c.set_selected_page(Some(plugin_page()), 0);
        c.set_timer(true, false, 1);
        assert_eq!(c.view(1).content, Content::CompactTimer);
        c.set_media(true, 2);
        assert_eq!(c.view(2).content, Content::CompactMedia);
        c.volume(3);
        assert_eq!(c.view(3).content, Content::Volume);
        c.plugin_activity(
            PluginActivity {
                plugin_id: "fixture".into(),
                id: "finished".into(),
                title: "Completed".into(),
                body: String::new(),
                accent: String::new(),
            },
            4,
        );
        assert!(matches!(c.view(4).content, Content::CompactPlugin(_)));
        c.battery(
            BatteryActivity {
                snapshot: crate::services::battery::BatterySnapshot::from_raw(1, 8, 50),
                kind: crate::services::battery::BatteryEventKind::Connected,
            },
            5,
        );
        c.set_timer(false, true, 6);
        c.push(notification(), 7);
        assert!(matches!(c.view(7).content, Content::Notification(..)));
        c.remove(1, 8);
        assert_eq!(c.view(8).content, Content::TimerFinished);
        c.clear_timer(9);
        assert!(matches!(c.view(9).content, Content::CompactBattery(_)));
        c.clear_battery(10);
        assert!(matches!(c.view(10).content, Content::CompactPlugin(_)));
        c.clear_plugin("fixture", 11);
        assert_eq!(c.view(11).content, Content::Volume);
        assert_eq!(c.view(1503).content, Content::CompactMedia);
        c.set_timer(true, false, 1504);
        c.set_media(false, 1505);
        assert_eq!(c.view(1505).content, Content::CompactTimer);
        c.clear_timer(1506);
        assert_eq!(c.view(1506).content, compact_plugin());
        assert_eq!(c.selected_page(), Some(&plugin_page()));
        assert_eq!(c.next_deadline(1506), None);
        c.set_selected_page(None, 1507);
        assert_eq!(c.view(1507).content, Content::Idle);
    }
    #[test]
    fn selected_plugin_reopens_while_playback_prioritizes_compact_media() {
        let mut coordinator = PresentationCoordinator::default();
        coordinator.set_media(true, 0);
        coordinator.navigate_page(plugin_page(), 10);
        assert_eq!(coordinator.view(10).content, Content::PluginPanel);
        coordinator.close_hover(20);
        assert_eq!(coordinator.view(20).content, Content::CompactMedia);
        assert_eq!(coordinator.next_deadline(20), None);
        coordinator.open_hover(30);
        assert_eq!(coordinator.view(30).content, Content::PluginPanel);
        assert_eq!(coordinator.selected_page(), Some(&plugin_page()));
    }
    #[test]
    fn navigation_preserves_pin_and_unpin_restores_hover_lifetime() {
        let mut coordinator = PresentationCoordinator::default();
        coordinator.open_hover(0);
        coordinator.toggle_pin(1);
        coordinator.navigate_page(Page::Media, 2);
        coordinator.close_hover(3);
        assert_eq!(coordinator.view(3).content, Content::Media);
        assert!(coordinator.pinned());
        coordinator.toggle_pin(4);
        coordinator.close_hover(5);
        assert_eq!(coordinator.view(5).content, Content::CompactMedia);
    }
    #[test]
    fn temporary_volume_and_notification_return_to_selected_compact_page() {
        let mut coordinator = PresentationCoordinator::default();
        coordinator.set_selected_page(Some(plugin_page()), 0);
        coordinator.volume(10);
        assert_eq!(coordinator.view(10).content, Content::Volume);
        assert_eq!(coordinator.view(1510).content, compact_plugin());
        coordinator.push(notification(), 2000);
        assert!(matches!(
            coordinator.view(2000).content,
            Content::Notification(..)
        ));
        assert_eq!(coordinator.view(7000).content, compact_plugin());
        assert_eq!(coordinator.selected_page(), Some(&plugin_page()));
    }
    #[test]
    fn navigating_queues_existing_notification_instead_of_losing_it() {
        let mut coordinator = PresentationCoordinator::default();
        coordinator.push(notification(), 0);
        coordinator.navigate_page(Page::Timer, 100);
        assert_eq!(coordinator.view(100).content, Content::Timer);
        assert_eq!(coordinator.view(100).pending, 1);
        coordinator.close_hover(200);
        assert!(matches!(
            coordinator.view(200).content,
            Content::Notification(..)
        ));
        assert_eq!(coordinator.view(5100).content, Content::CompactTimer);
    }
    #[test]
    fn playback_overrides_explicit_home_without_overwriting_it() {
        let mut coordinator = PresentationCoordinator::default();
        coordinator.set_media(true, 0);
        assert_eq!(coordinator.view(0).content, Content::CompactMedia);
        coordinator.set_selected_page(Some(Page::Home), 1);
        assert_eq!(coordinator.view(1).content, Content::CompactMedia);
        coordinator.set_media(false, 2);
        assert_eq!(coordinator.view(2).content, Content::Idle);
        assert_eq!(coordinator.selected_page(), Some(&Page::Home));
        coordinator.set_media(true, 2);
        coordinator.set_selected_page(None, 2);
        assert_eq!(coordinator.view(2).content, Content::CompactMedia);
    }
    #[test]
    fn battery_and_plugin_activity_restore_selected_page() {
        let mut coordinator = PresentationCoordinator::default();
        coordinator.set_selected_page(Some(Page::Timer), 0);
        coordinator.battery(
            BatteryActivity {
                snapshot: crate::services::battery::BatterySnapshot::from_raw(1, 8, 50),
                kind: crate::services::battery::BatteryEventKind::Connected,
            },
            1,
        );
        assert!(matches!(
            coordinator.view(1).content,
            Content::CompactBattery(_)
        ));
        assert_eq!(coordinator.view(3001).content, Content::CompactTimer);
        coordinator.plugin_activity(
            PluginActivity {
                plugin_id: "fixture".into(),
                id: "update".into(),
                title: "Updated".into(),
                body: "".into(),
                accent: "".into(),
            },
            4000,
        );
        assert!(matches!(
            coordinator.view(4000).content,
            Content::CompactPlugin(_)
        ));
        assert_eq!(coordinator.view(7000).content, Content::CompactTimer);
    }
}
