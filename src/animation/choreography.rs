//! Continuous visibility envelopes based on available physical space.
//! These also constrain the rendered opacity during interrupted contractions,
//! when a visibility spring can still carry positive velocity.
fn smoothstep(value: f64) -> f64 {
    let t = value.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn expanded_space(width: f64, height: f64) -> f64 {
    smoothstep((width - 290.0) / 60.0) * smoothstep((height - 112.0) / 42.0)
}

pub fn compact_space(width: f64, height: f64) -> f64 {
    smoothstep((width - 164.0) / 46.0) * smoothstep((80.0 - height) / 26.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_requires_space_in_both_dimensions() {
        assert_eq!(expanded_space(126.0, 32.0), 0.0);
        assert_eq!(expanded_space(360.0, 100.0), 0.0);
        assert_eq!(expanded_space(280.0, 160.0), 0.0);
        assert_eq!(expanded_space(360.0, 160.0), 1.0);
        assert_eq!(compact_space(126.0, 32.0), 0.0);
        assert_eq!(compact_space(360.0, 160.0), 0.0);
        assert_eq!(compact_space(220.0, 48.0), 1.0);
    }

    #[test]
    fn expansion_envelope_is_continuous_and_monotonic() {
        let mut previous = 0.0;
        for step in 0..=1000 {
            let t = step as f64 / 1000.0;
            let current = expanded_space(126.0 + 234.0 * t, 32.0 + 128.0 * t);
            assert!(current >= previous && current <= 1.0);
            assert!(current - previous < 0.02);
            previous = current;
        }
    }
}
