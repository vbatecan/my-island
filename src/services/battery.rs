//! Pure battery normalization and event detection. Windows delivery lives in
//! platform::power; presentation owns preview timing and precedence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BatterySnapshot {
    pub present: bool,
    pub percent: Option<u8>,
    pub external_power: Option<bool>,
    pub charging: Option<bool>,
}
impl BatterySnapshot {
    pub fn from_raw(ac: u8, flags: u8, percent: u8) -> Self {
        let present = flags != 128 && (flags != 255 || percent <= 100);
        Self {
            present,
            percent: (present && percent <= 100).then_some(percent),
            external_power: match ac {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            },
            charging: (present && flags != 255).then_some(flags & 8 != 0),
        }
    }
    /// AC connection remains meaningful when a full battery stops charging.
    pub fn power_connected(self) -> bool {
        self.present
            && match self.external_power {
                Some(connected) => connected,
                None => self.charging == Some(true),
            }
    }
    pub fn label(self) -> &'static str {
        if !self.present {
            "No system battery"
        } else if self.charging == Some(true) {
            "Charging"
        } else {
            match self.external_power {
                Some(true) => "Plugged in",
                Some(false) => "On battery",
                None => "Power status unavailable",
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatteryEventKind {
    Connected,
    Disconnected,
    ChargingChanged,
    Low,
    Critical,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryActivity {
    pub snapshot: BatterySnapshot,
    pub kind: BatteryEventKind,
}
impl BatteryActivity {
    pub fn label(self) -> &'static str {
        match self.kind {
            BatteryEventKind::Low => "Low battery",
            BatteryEventKind::Critical => "Battery very low",
            _ => self.snapshot.label(),
        }
    }
}
#[derive(Default)]
pub struct BatteryDetector {
    previous: Option<BatterySnapshot>,
}
impl BatteryDetector {
    /// Establishing or replacing a battery is silent. Ordinary percentage
    /// updates do not generate activities; threshold recovery naturally rearms.
    pub fn update(&mut self, current: BatterySnapshot) -> Option<BatteryActivity> {
        let previous = self.previous.replace(current)?;
        if !previous.present || !current.present || previous == current {
            return None;
        }
        let crossed = |threshold| {
            current.external_power == Some(false)
                && previous.percent.is_some_and(|p| p > threshold)
                && current.percent.is_some_and(|p| p <= threshold)
        };
        let kind = if crossed(10) {
            BatteryEventKind::Critical
        } else if crossed(20) {
            BatteryEventKind::Low
        } else if let (Some(old), Some(new)) = (previous.external_power, current.external_power) {
            if old != new {
                if new {
                    BatteryEventKind::Connected
                } else {
                    BatteryEventKind::Disconnected
                }
            } else if previous.charging.is_some()
                && current.charging.is_some()
                && previous.charging != current.charging
            {
                BatteryEventKind::ChargingChanged
            } else {
                return None;
            }
        } else {
            return None;
        };
        Some(BatteryActivity {
            snapshot: current,
            kind,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn battery(p: u8) -> BatterySnapshot {
        BatterySnapshot::from_raw(0, 0, p)
    }
    #[test]
    fn full_battery_connection_drives_effect_without_claiming_charging() {
        let unplugged = BatterySnapshot::from_raw(0, 1, 100);
        let plugged = BatterySnapshot::from_raw(1, 1, 100);
        assert!(!unplugged.power_connected());
        assert!(plugged.power_connected());
        assert_eq!(plugged.charging, Some(false));
        assert_eq!(plugged.label(), "Plugged in");
        assert!(!BatterySnapshot::from_raw(1, 128, 100).power_connected());
        let mut detector = BatteryDetector::default();
        detector.update(unplugged);
        assert_eq!(
            detector.update(plugged).unwrap().kind,
            BatteryEventKind::Connected
        );
        assert_eq!(detector.update(plugged), None);
        assert_eq!(
            detector.update(unplugged).unwrap().kind,
            BatteryEventKind::Disconnected
        );
    }
    #[test]
    fn baseline_and_duplicates_are_silent() {
        let mut d = BatteryDetector::default();
        assert_eq!(d.update(battery(20)), None);
        assert_eq!(d.update(battery(20)), None);
        assert_eq!(d.update(battery(19)), None);
    }
    #[test]
    fn threshold_crossings_rearm_after_recovery() {
        let mut d = BatteryDetector::default();
        d.update(battery(21));
        assert_eq!(d.update(battery(20)).unwrap().kind, BatteryEventKind::Low);
        assert_eq!(d.update(battery(19)), None);
        assert_eq!(
            d.update(battery(9)).unwrap().kind,
            BatteryEventKind::Critical
        );
        d.update(battery(25));
        assert_eq!(d.update(battery(18)).unwrap().kind, BatteryEventKind::Low);
    }
    #[test]
    fn unknown_values_and_absence_do_not_invent_percentages() {
        let s = BatterySnapshot::from_raw(255, 255, 255);
        assert!(!s.present);
        assert_eq!(s.percent, None);
        let s = BatterySnapshot::from_raw(1, 0, 255);
        assert!(s.present);
        assert_eq!(s.percent, None);
        assert_eq!(s.label(), "Plugged in");
        assert!(!BatterySnapshot::from_raw(1, 128, 100).present);
    }
    #[test]
    fn power_changes_are_deduplicated_and_ac_never_warns_low() {
        let mut d = BatteryDetector::default();
        d.update(battery(50));
        let charging = BatterySnapshot::from_raw(1, 8, 49);
        assert_eq!(
            d.update(charging).unwrap().kind,
            BatteryEventKind::Connected
        );
        assert_eq!(d.update(charging), None);
        assert_eq!(d.update(BatterySnapshot::from_raw(1, 8, 9)), None);
        assert_eq!(
            d.update(BatterySnapshot::from_raw(1, 0, 9)).unwrap().kind,
            BatteryEventKind::ChargingChanged
        );
        assert_eq!(
            d.update(battery(9)).unwrap().kind,
            BatteryEventKind::Disconnected
        );
    }
}
