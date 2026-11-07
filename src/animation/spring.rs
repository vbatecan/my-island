//! Exact integration of a damped harmonic oscillator.
//!
//! Unlike Euler integration this remains stable when a frame is delayed. Retargeting
//! changes only the equilibrium, so an interrupted morph retains its momentum.

#[derive(Clone, Copy, Debug)]
pub struct SpringConfig {
    pub stiffness: f64,
    pub damping: f64,
    pub mass: f64,
    pub position_threshold: f64,
    pub velocity_threshold: f64,
}

impl Default for SpringConfig {
    fn default() -> Self {
        Self {
            stiffness: 700.0,
            damping: 42.0,
            mass: 1.0,
            position_threshold: 0.015,
            velocity_threshold: 0.08,
        }
    }
}

impl SpringConfig {
    fn validated(self) -> Self {
        // Public tuning parameters must never poison an entire UI with NaNs.
        let positive = |v: f64| v.is_finite() && (1e-8..=1e8).contains(&v);
        if !positive(self.stiffness)
            || !positive(self.mass)
            || !self.damping.is_finite()
            || !(0.0..=1e8).contains(&self.damping)
            || !positive(self.position_threshold)
            || !positive(self.velocity_threshold)
        {
            Self::default()
        } else {
            self
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub position: f64,
    pub velocity: f64,
    pub target: f64,
    pub config: SpringConfig,
}

impl Spring {
    pub fn new(value: f64) -> Self {
        let value = if value.is_finite() { value } else { 0.0 };
        Self {
            position: value,
            target: value,
            velocity: 0.0,
            config: SpringConfig::default(),
        }
    }

    pub fn set_target(&mut self, target: f64) {
        if target.is_finite() {
            self.target = target;
        }
    }

    pub fn snap(&mut self, target: f64) {
        if target.is_finite() {
            self.position = target;
            self.target = target;
            self.velocity = 0.0;
        }
    }

    pub fn is_settled(&self) -> bool {
        let config = self.config.validated();
        (self.position - self.target).abs() <= config.position_threshold
            && self.velocity.abs() <= config.velocity_threshold
    }

    /// Advance by real elapsed seconds; returns whether another frame is needed.
    /// Nonpositive and nonfinite deltas are ignored. A large finite delay is
    /// integrated exactly, rather than split into a potentially unbounded loop.
    pub fn advance(&mut self, dt: f64) -> bool {
        if !self.position.is_finite() || !self.velocity.is_finite() {
            self.snap(if self.target.is_finite() {
                self.target
            } else {
                0.0
            });
        }
        if !self.target.is_finite() {
            self.target = self.position;
        }
        if self.is_settled() {
            self.snap(self.target);
            return false;
        }
        if !dt.is_finite() || dt <= 0.0 {
            return true;
        }

        let c = self.config.validated();
        let omega = (c.stiffness / c.mass).sqrt();
        let alpha = c.damping / (2.0 * c.mass);
        let y = self.position - self.target;
        let v = self.velocity;
        let discriminant = alpha * alpha - omega * omega;

        let (next_y, next_v) = if discriminant.abs() <= omega * omega * 1e-10 {
            let decay = (-alpha * dt).exp();
            if decay == 0.0 {
                (0.0, 0.0)
            } else {
                let b = v + alpha * y;
                let next_y = (y + b * dt) * decay;
                (next_y, (v - alpha * b * dt) * decay)
            }
        } else if discriminant < 0.0 {
            let wd = (-discriminant).sqrt();
            let decay = (-alpha * dt).exp();
            if decay == 0.0 {
                (0.0, 0.0)
            } else {
                // Bound the argument before multiplication, including enormous
                // elapsed times for undamped oscillators.
                let phase = wd * (dt % (std::f64::consts::TAU / wd));
                let (sin, cos) = phase.sin_cos();
                (
                    decay * (y * cos + (v + alpha * y) / wd * sin),
                    decay * (v * cos - (alpha * v + omega * omega * y) / wd * sin),
                )
            }
        } else {
            let root = discriminant.sqrt();
            // This avoids cancellation for highly overdamped springs.
            let r1 = -omega * omega / (alpha + root);
            let r2 = -alpha - root;
            let a = (v - r2 * y) / (r1 - r2);
            let b = y - a;
            let e1 = (r1 * dt).exp();
            let e2 = (r2 * dt).exp();
            (a * e1 + b * e2, r1 * a * e1 + r2 * b * e2)
        };
        self.position = self.target + next_y;
        self.velocity = next_v;
        if !self.position.is_finite() || !self.velocity.is_finite() || self.is_settled() {
            self.snap(self.target);
            false
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converges_and_stops_exactly() {
        for damping in [42.0, 2.0 * 700.0_f64.sqrt(), 90.0] {
            let mut s = Spring::new(126.0);
            s.config.damping = damping;
            s.set_target(360.0);
            for _ in 0..1200 {
                if !s.advance(1.0 / 120.0) {
                    break;
                }
            }
            assert!(s.is_settled());
            assert_eq!(s.position, 360.0);
            assert_eq!(s.velocity, 0.0);
            assert!(!s.advance(1.0 / 60.0));
        }
    }

    #[test]
    fn retargeting_preserves_position_and_momentum() {
        let mut s = Spring::new(126.0);
        s.set_target(360.0);
        s.advance(0.08);
        let before = (s.position, s.velocity);
        assert!(before.1 > 0.0);
        s.set_target(126.0);
        assert_eq!((s.position, s.velocity), before);
        s.advance(0.001);
        assert!(s.position > before.0, "momentum survives the reversal");
        for i in 0..100 {
            s.set_target(if i % 2 == 0 { 360.0 } else { 126.0 });
            s.advance(0.013);
            assert!(s.position.is_finite() && s.velocity.is_finite());
        }
        s.advance(10.0);
        assert_eq!(s.position, 126.0);
    }

    #[test]
    fn identical_elapsed_time_is_frame_rate_independent() {
        for damping in [42.0, 2.0 * 700.0_f64.sqrt(), 90.0] {
            let mut reference = Spring::new(0.0);
            reference.config.damping = damping;
            reference.set_target(234.0);
            reference.advance(0.2);
            for fps in [30, 60, 120, 144, 240] {
                let mut s = Spring::new(0.0);
                s.config.damping = damping;
                s.set_target(234.0);
                let count = fps / 5;
                for _ in 0..count {
                    s.advance(0.2 / count as f64);
                }
                assert!((s.position - reference.position).abs() < 1e-9);
                assert!((s.velocity - reference.velocity).abs() < 1e-8);
            }
        }
    }

    #[test]
    fn settling_requires_both_velocity_and_position() {
        let mut s = Spring::new(0.0);
        assert!(s.is_settled());
        s.velocity = 5.0;
        assert!(!s.is_settled());
        s.velocity = 0.0;
        s.position = 0.02;
        assert!(!s.is_settled());
        s.position = 0.001;
        assert!(!s.advance(0.01));
        assert_eq!(s.position, 0.0);
    }

    #[test]
    fn invalid_and_extreme_time_steps_are_safe() {
        let mut s = Spring::new(0.0);
        s.set_target(360.0);
        for dt in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(s.advance(dt));
            assert_eq!(s.position, 0.0);
            assert_eq!(s.velocity, 0.0);
        }
        assert!(s.advance(f64::MIN_POSITIVE));
        assert!(!s.advance(f64::MAX));
        assert_eq!(s.position, 360.0);
        s.set_target(f64::NAN);
        assert_eq!(s.target, 360.0);
    }

    #[test]
    fn invalid_tuning_and_state_cannot_propagate_nan() {
        let mut s = Spring::new(f64::NAN);
        s.config.mass = 0.0;
        s.set_target(100.0);
        s.advance(0.1);
        assert!(s.position.is_finite());
        s.velocity = f64::INFINITY;
        assert!(!s.advance(0.1));
        assert_eq!(s.position, 100.0);
    }

    #[test]
    fn default_has_restrained_overshoot() {
        let mut s = Spring::new(0.0);
        s.set_target(1.0);
        let mut peak: f64 = 0.0;
        for _ in 0..120 {
            s.advance(1.0 / 240.0);
            peak = peak.max(s.position);
        }
        assert!(peak > 1.0 && peak < 1.03, "peak: {peak}");
        assert!(s.is_settled());
    }
}
