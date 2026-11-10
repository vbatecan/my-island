//! Bounded vector geometry; visual phase never changes playback time.
pub(super) fn wave(width: f32, value: f32, phase: f32, amplitude: f32) -> String {
    if !width.is_finite() || !value.is_finite() || !phase.is_finite() || !amplitude.is_finite() {
        return String::new();
    }
    let end = width.clamp(0., 4096.) * value.clamp(0., 1.);
    if end <= 0. {
        return String::new();
    }
    let mut path = String::from("M 0 5");
    let samples = (end / 3.).ceil().max(1.) as usize;
    use std::fmt::Write;
    for i in 1..=samples {
        let x = end * i as f32 / samples as f32;
        let taper = (x / 8.).min((end - x) / 8.).clamp(0., 1.);
        let y =
            5. + amplitude.clamp(0., 2.) * taper * (x * std::f32::consts::TAU / 24. - phase).sin();
        let _ = write!(path, " L {x:.2} {y:.2}");
    }
    path
}
pub(super) fn digits(previous: &str, current: &str, cells: usize) -> Vec<crate::DigitFrame> {
    let old = format!("{previous:>cells$}");
    let new = format!("{current:>cells$}");
    old.chars()
        .zip(new.chars())
        .map(|(a, b)| crate::DigitFrame {
            previous: a.to_string().into(),
            current: b.to_string().into(),
            changing: a != b,
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::{digits, wave};
    #[test]
    fn endpoints_are_exact_and_invalid_input_is_empty() {
        assert!(wave(100., 0.5, 2., 2.).ends_with("50.00 5.00"));
        assert!(wave(100., 1., 0., 2.).ends_with("100.00 5.00"));
        assert!(wave(100., 0., 0., 2.).is_empty());
        assert!(wave(f32::NAN, 1., 0., 2.).is_empty());
    }
    #[test]
    fn minute_carry_preserves_cells_and_colon() {
        let rows = digits("10:00", "9:59", 5);
        assert_eq!(rows.len(), 5);
        assert!(!rows[2].changing);
        assert_eq!(rows[0].current, " ");
        assert_eq!(rows[4].current, "9");
    }
}
