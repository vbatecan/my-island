//! Local launcher actions. Parsing never performs an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Timer(u64),
    KeepVisible(bool),
}
impl Action {
    pub fn label(&self, replacing: bool) -> String {
        match self {
            Self::KeepVisible(true) => "Keep visible".into(),
            Self::KeepVisible(false) => "Resume auto hide".into(),
            Self::Timer(seconds) => {
                let (amount, unit) = if seconds % 60 == 0 {
                    (seconds / 60, "minute")
                } else {
                    (*seconds, "second")
                };
                if replacing {
                    format!(
                        "Replace timer with {amount} {unit}{}",
                        if amount == 1 { "" } else { "s" }
                    )
                } else {
                    format!("Start {amount}-{unit} timer")
                }
            }
        }
    }
}
pub fn is_incomplete(query: &str) -> bool {
    let q = query.trim().to_ascii_lowercase();
    if q == "timer" {
        return true;
    }
    let explicit = q.starts_with("timer ");
    let q = q.strip_prefix("timer ").unwrap_or(&q);
    let token = q.split_whitespace().last().unwrap_or("");
    let digits = token
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(token.len());
    if digits == 0 {
        return explicit;
    }
    let suffix = &token[digits..];
    if suffix.is_empty() {
        return explicit || q.split_whitespace().count() > 1;
    }
    parse(query).is_none()
        && ["seconds", "minutes", "hours"]
            .iter()
            .any(|unit| unit.starts_with(suffix))
}

pub fn parse(query: &str) -> Option<Action> {
    let query = query.trim().to_ascii_lowercase();
    match query.as_str() {
        "keep visible" => return Some(Action::KeepVisible(true)),
        "resume auto hide" => return Some(Action::KeepVisible(false)),
        _ => {}
    }
    let mut rest = query.strip_prefix("timer ").unwrap_or(&query).trim();
    let mut seconds = 0_u64;
    while !rest.is_empty() {
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        if digits == 0 {
            return None;
        }
        let value = rest[..digits].parse::<u64>().ok()?;
        rest = rest[digits..].trim_start();
        let end = rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(rest.len());
        let multiplier = match &rest[..end] {
            "s" | "sec" | "secs" | "second" | "seconds" => 1,
            "m" | "min" | "mins" | "minute" | "minutes" => 60,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
            _ => return None,
        };
        seconds = seconds.checked_add(value.checked_mul(multiplier)?)?;
        rest = rest[end..].trim_start();
    }
    (1..=86400)
        .contains(&seconds)
        .then_some(Action::Timer(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn durations_are_complete_and_bounded() {
        for (query, seconds) in [
            ("10 minutes", 600),
            ("10m", 600),
            ("1h 30m", 5400),
            ("timer 45s", 45),
            ("24h", 86400),
        ] {
            assert_eq!(parse(query), Some(Action::Timer(seconds)));
        }
        for query in [
            "10",
            "timer",
            "timer 10",
            "0m",
            "25h",
            "1.5m",
            "1h junk",
            "18446744073709551615h",
        ] {
            assert_eq!(parse(query), None, "{query}");
        }
    }
}
