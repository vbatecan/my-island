//! Bounded local evaluation and on-demand reference exchange rates.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Calculator,
    Currency,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub value: String,
    pub detail: String,
}
#[derive(Debug, PartialEq, Eq)]
struct Pair {
    amount: String,
    base: String,
    quote: String,
}
fn number(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 128
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || "+-.eE".contains(c))
        && text.parse::<f64>().is_ok_and(f64::is_finite)
}
fn pair(query: &str) -> Option<Pair> {
    if query.len() > 512 {
        return None;
    }
    let words = query.split_whitespace().collect::<Vec<_>>();
    if words.len() != 4
        || !number(words[0])
        || !(words[2].eq_ignore_ascii_case("to") || words[2].eq_ignore_ascii_case("in"))
        || ![words[1], words[3]]
            .iter()
            .all(|v| v.len() == 3 && v.bytes().all(|c| c.is_ascii_alphabetic()))
    {
        return None;
    }
    Some(Pair {
        amount: words[0].into(),
        base: words[1].to_ascii_uppercase(),
        quote: words[3].to_ascii_uppercase(),
    })
}
pub fn currency_pair_key(query: &str) -> Option<String> {
    pair(query).map(|p| format!("{}/{}", p.base, p.quote))
}
pub fn is_incomplete(query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() || q.len() > 512 {
        return false;
    }
    let lower = q.to_ascii_lowercase();
    let mathlike = q.starts_with(|c: char| c.is_ascii_digit() || "+-.(".contains(c))
        || ["sin(", "cos(", "tan(", "sqrt(", "log(", "ln(", "abs("]
            .iter()
            .any(|f| lower.starts_with(f));
    let balance = q.chars().fold(0_i32, |n, c| {
        n + if c == '(' {
            1
        } else if c == ')' {
            -1
        } else {
            0
        }
    });
    let words: Vec<_> = q.split_whitespace().collect();
    mathlike
        && (q.ends_with(['+', '-', '*', '/', '^', '(', '='])
            || balance > 0
            || lower.ends_with(" to")
            || lower.ends_with(" in")
            || (words.len() == 4
                && ["to", "in"].contains(&words[2].to_ascii_lowercase().as_str())
                && words[3].len() < 3))
}
pub fn classify(query: &str) -> Option<Kind> {
    if query.len() > 512 {
        return query
            .trim_start()
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || "+-(.".contains(c))
            .then_some(Kind::Calculator);
    }
    let q = query.trim();
    if pair(q).is_some() {
        return Some(Kind::Currency);
    }
    if q.is_empty() {
        return None;
    }
    // An unfinished expression is an editing state, not work or an error.
    let balance = q.chars().fold(0_i32, |n, c| {
        n + if c == '(' {
            1
        } else if c == ')' {
            -1
        } else {
            0
        }
    });
    if q.ends_with(['+', '-', '*', '/', '^', '(', '=']) || balance > 0 {
        return None;
    }
    let lower = q.to_lowercase();
    let function = [
        "sqrt", "sin", "cos", "tan", "log", "ln", "abs", "exp", "floor", "ceil", "round",
    ]
    .iter()
    .any(|f| {
        lower
            .strip_prefix(f)
            .is_some_and(|r| r.trim_start().starts_with('('))
    });
    let numeric = q.chars().any(|c| c.is_ascii_digit())
        && q.chars()
            .all(|c| c.is_ascii_digit() || c.is_whitespace() || "+-*/().^%!×÷−eE".contains(c));
    let units = q.split_whitespace().next().is_some_and(number)
        && (lower.contains(" to ") || lower.contains(" in "));
    (numeric || function || units || ["pi", "e", "i"].contains(&lower.as_str()))
        .then_some(Kind::Calculator)
}
struct Deadline<'a> {
    start: Instant,
    cancel: &'a AtomicBool,
}
impl fend_core::Interrupt for Deadline<'_> {
    fn should_interrupt(&self) -> bool {
        self.cancel.load(Ordering::Acquire) || self.start.elapsed() > Duration::from_millis(150)
    }
}
fn math(text: &str, cancel: &AtomicBool) -> Result<String, String> {
    let text = text.replace('×', "*").replace('÷', "/").replace('−', "-");
    let mut context = fend_core::Context::new();
    let result = fend_core::evaluate_with_interrupt(
        &text,
        &mut context,
        &Deadline {
            start: Instant::now(),
            cancel,
        },
    )?
    .get_main_result()
    .to_owned();
    if result.len() > 4096 {
        Err("Result is too large to display.".into())
    } else {
        Ok(result)
    }
}
pub fn evaluate(query: &str, cancel: &AtomicBool) -> Result<Option<Answer>, String> {
    if cancel.load(Ordering::Acquire) {
        return Ok(None);
    }
    let kind = classify(query);
    if kind.is_none() {
        return Ok(None);
    }
    if query.len() > 512 {
        return Err("Keep expressions under 512 bytes.".into());
    }
    let result = if kind == Some(Kind::Currency) {
        convert(&pair(query).unwrap(), cancel)
    } else {
        math(query, cancel).map(|value| Answer {
            value,
            detail: String::new(),
        })
    };
    if cancel.load(Ordering::Acquire) {
        Ok(None)
    } else {
        result.map(Some)
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Rate {
    rate: String,
    date: String,
    fetched: u64,
}
#[derive(Default, Serialize, Deserialize)]
struct Cache {
    rates: BTreeMap<String, Rate>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn valid_date(date: &str) -> bool {
    if date.len() != 10
        || !date.bytes().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
    {
        return false;
    }
    let values = date
        .split('-')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>();
    let Ok(v) = values else { return false };
    if date.len() != 10
        || v.len() != 3
        || !(1900..=9999).contains(&v[0])
        || !(1..=12).contains(&v[1])
    {
        return false;
    }
    let leap = v[0] % 4 == 0 && (v[0] % 100 != 0 || v[0] % 400 == 0);
    let days = match v[1] {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=days).contains(&v[2])
}
fn valid_rate(rate: &Rate, time: u64) -> bool {
    rate.rate.len() <= 128
        && rate
            .rate
            .parse::<f64>()
            .is_ok_and(|r| r.is_finite() && r > 0.)
        && rate
            .rate
            .chars()
            .all(|c| c.is_ascii_digit() || ".eE+-".contains(c))
        && valid_date(&rate.date)
        && rate.fetched <= time
}
fn load(path: &std::path::Path, time: u64) -> Cache {
    let read = || -> Option<Cache> {
        let file = fs::File::open(path).ok()?;
        if file.metadata().ok()?.len() > 65536 {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(65537).read_to_end(&mut bytes).ok()?;
        let mut cache: Cache = serde_json::from_slice(&bytes).ok()?;
        cache.rates.retain(|key, rate| {
            key.len() == 7
                && key.as_bytes()[3] == b'/'
                && key
                    .bytes()
                    .enumerate()
                    .all(|(i, c)| i == 3 || c.is_ascii_uppercase())
                && valid_rate(rate, time)
        });
        while cache.rates.len() > 128 {
            let k = cache.rates.keys().next()?.clone();
            cache.rates.remove(&k);
        }
        Some(cache)
    };
    read().unwrap_or_default()
}
fn choose_rate(
    cached: Option<Rate>,
    time: u64,
    fetch: impl FnOnce() -> Result<Rate, String>,
) -> Result<(Rate, bool), String> {
    if let Some(rate) = cached
        .as_ref()
        .filter(|r| time.saturating_sub(r.fetched) < 21600)
    {
        return Ok((rate.clone(), false));
    }
    match fetch() {
        Ok(rate) if valid_rate(&rate, time) => Ok((rate, false)),
        Ok(_) => cached
            .map(|r| (r, true))
            .ok_or("Invalid exchange rate response.".into()),
        Err(e) => cached.map(|r| (r, true)).ok_or(e),
    }
}
fn convert(pair: &Pair, cancel: &AtomicBool) -> Result<Answer, String> {
    if pair.base == pair.quote {
        return Ok(Answer {
            value: format!("{} {}", math(&pair.amount, cancel)?, pair.quote),
            detail: "Same currency · no conversion needed".into(),
        });
    }
    let path = crate::settings::Settings::path().with_file_name("currency-rates.json");
    let time = now();
    let mut cache = load(&path, time);
    let key = format!("{}/{}", pair.base, pair.quote);
    let (rate, offline) = choose_rate(cache.rates.get(&key).cloned(), time, || {
        fetch_rate(&pair.base, &pair.quote, cancel, time)
    })?;
    if cancel.load(Ordering::Acquire) {
        return Err("Conversion cancelled.".into());
    }
    cache.rates.insert(key, rate.clone());
    while cache.rates.len() > 128 {
        if let Some(key) = cache
            .rates
            .iter()
            .min_by_key(|(_, r)| r.fetched)
            .map(|(k, _)| k.clone())
        {
            cache.rates.remove(&key);
        }
    }
    if let Ok(bytes) = serde_json::to_vec(&cache)
        && bytes.len() <= 65536
        && let Some(parent) = path.parent()
    {
        let _ = fs::create_dir_all(parent);
        let temp = path.with_extension("tmp");
        if fs::write(&temp, bytes).is_ok() {
            let _ = fs::rename(temp, &path);
        }
    }
    let value = math(&format!("({}) * ({})", pair.amount, rate.rate), cancel)?;
    Ok(Answer {
        value: format!("{value} {}", pair.quote),
        detail: format!(
            "Frankfurter reference rate · {} · {} · not a live quote",
            rate.date,
            if offline {
                "offline / stale cache"
            } else {
                "cached locally"
            }
        ),
    })
}
fn fetch_rate(base: &str, quote: &str, cancel: &AtomicBool, time: u64) -> Result<Rate, String> {
    use std::{ffi::c_void, ptr};
    use windows::{
        Win32::Networking::WinHttp::*,
        core::{PCWSTR, w},
    };
    struct Handle(*mut c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
    fn own(raw: *mut c_void) -> Result<Handle, String> {
        if raw.is_null() {
            Err("Cannot open exchange-rate connection.".into())
        } else {
            Ok(Handle(raw))
        }
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    let check = || {
        if cancel.load(Ordering::Acquire) || Instant::now() >= deadline {
            Err("Exchange-rate request cancelled or timed out.".to_string())
        } else {
            Ok(())
        }
    };
    let path = format!(
        "/v2/rate/{}/{}",
        base.to_ascii_lowercase(),
        quote.to_ascii_lowercase()
    )
    .encode_utf16()
    .chain(Some(0))
    .collect::<Vec<_>>();
    // SAFETY: request strings live through each synchronous call. Handles are
    // owned solely on this worker and drop request -> connection -> session.
    let bytes = unsafe {
        let session = own(WinHttpOpen(
            w!("StillIsland/0.2"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, 2000, 2000, 2000, 2000).map_err(|e| e.to_string())?;
        let connection = own(WinHttpConnect(session.0, w!("api.frankfurter.dev"), 443, 0))?;
        let request = own(WinHttpOpenRequest(
            connection.0,
            w!("GET"),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        WinHttpSetOption(
            Some(request.0.cast_const()),
            WINHTTP_OPTION_REDIRECT_POLICY,
            Some(&WINHTTP_OPTION_REDIRECT_POLICY_NEVER.to_ne_bytes()),
        )
        .map_err(|e| e.to_string())?;
        check()?;
        // Send can perform resolve, connect and send phases. Divide the
        // remaining overall budget across those operations; later phases and
        // every body read recompute their timeout from the same deadline.
        let budget = timeout_budget(deadline.saturating_duration_since(Instant::now()), 4)?;
        WinHttpSetTimeouts(request.0, budget, budget, budget, budget).map_err(|e| e.to_string())?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0)
            .map_err(|_| "Exchange-rate request failed.")?;
        check()?;
        let budget = timeout_budget(deadline.saturating_duration_since(Instant::now()), 1)?;
        WinHttpSetTimeouts(request.0, budget, budget, budget, budget).map_err(|e| e.to_string())?;
        WinHttpReceiveResponse(request.0, ptr::null_mut())
            .map_err(|_| "Exchange-rate response failed.")?;
        let mut status = 0u32;
        let mut length = 4u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut length,
            ptr::null_mut(),
        )
        .map_err(|e| e.to_string())?;
        if status != 200 {
            return Err(format!(
                "No exchange rate available for {base}/{quote} (HTTP {status})."
            ));
        }
        let mut body = Vec::new();
        loop {
            check()?;
            let budget = timeout_budget(deadline.saturating_duration_since(Instant::now()), 1)?;
            WinHttpSetTimeouts(request.0, budget, budget, budget, budget)
                .map_err(|e| e.to_string())?;
            let mut buffer = [0u8; 4096];
            let mut read = 0;
            WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut read,
            )
            .map_err(|_| "Cannot read exchange rate.")?;
            if read == 0 {
                break;
            }
            if body.len() + read as usize > 65536 {
                return Err("Exchange-rate response is too large.".into());
            }
            body.extend_from_slice(&buffer[..read as usize]);
        }
        body
    };
    check()?;
    parse_response(&bytes, base, quote, time)
}
fn timeout_budget(remaining: Duration, phases: u128) -> Result<i32, String> {
    if remaining.is_zero() {
        return Err("Exchange-rate request timed out.".into());
    }
    Ok((remaining.as_millis() / phases).clamp(1, 2000) as i32)
}
fn parse_response(bytes: &[u8], base: &str, quote: &str, time: u64) -> Result<Rate, String> {
    let data: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| "Invalid exchange-rate JSON.")?;
    let rate = data
        .get("rate")
        .and_then(|v| v.as_number())
        .ok_or("Missing exchange rate.")?
        .to_string();
    let date = data
        .get("date")
        .and_then(|v| v.as_str())
        .ok_or("Missing exchange-rate date.")?
        .to_owned();
    if data
        .get("base")
        .and_then(|v| v.as_str())
        .is_none_or(|v| !v.eq_ignore_ascii_case(base))
        || data
            .get("quote")
            .and_then(|v| v.as_str())
            .is_none_or(|v| !v.eq_ignore_ascii_case(quote))
    {
        return Err("Mismatched exchange-rate pair.".into());
    }
    let rate = Rate {
        rate,
        date,
        fetched: time,
    };
    if valid_rate(&rate, time) {
        Ok(rate)
    } else {
        Err("Invalid exchange rate response.".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_response_deadline_and_cache() {
        assert_eq!(timeout_budget(Duration::from_millis(80), 4).unwrap(), 20);
        assert!(timeout_budget(Duration::ZERO, 1).is_err());
        assert!(
            parse_response(
                br#"{"base":"USD","quote":"SGD","date":"2026-11-08","rate":1.3}"#,
                "USD",
                "SGD",
                30
            )
            .is_ok()
        );
        for bytes in [
            br#"{}"#.as_slice(),
            br#"{"base":"EUR","quote":"SGD","date":"2026-11-08","rate":1.3}"#,
            br#"{"base":"USD","quote":"SGD","date":"2026-02-30","rate":1.3}"#,
            br#"{"base":"USD","quote":"SGD","date":"2026-11-08","rate":-1}"#,
        ] {
            assert!(parse_response(bytes, "USD", "SGD", 30).is_err());
        }
        let path = std::env::temp_dir().join(format!(
            "still-island-rate-test-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, b"broken json").unwrap();
        assert!(load(&path, 30).rates.is_empty());
        fs::write(&path,br#"{"rates":{"USD/SGD":{"rate":"1.3","date":"2026-11-08","fetched":10},"USD/EUR":{"rate":"1.2","date":"2026-11-08","fetched":100}}}"#).unwrap();
        let cached = load(&path, 30);
        assert_eq!(cached.rates.len(), 1);
        assert_eq!(cached.rates["USD/SGD"].rate, "1.3");
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn detection() {
        for input in ["3+", "3 *", "sin(", "(2 + 3", "1e-"] {
            assert_eq!(classify(input), None, "{input}");
            assert!(is_incomplete(input), "{input}");
        }
        assert_eq!(classify("7-Zip"), None);
        assert_eq!(classify("Calculator"), None);
        for q in ["(50 + 20) / 2", "sin(pi / 2)", "1.5e6 / 3", "5 ft to cm"] {
            assert_eq!(classify(q), Some(Kind::Calculator));
        }
        assert_eq!(classify("100 usd in sgd"), Some(Kind::Currency));
        assert_eq!(classify("100 USD to SG"), Some(Kind::Calculator));
    }
    #[test]
    fn local_math() {
        let c = AtomicBool::new(false);
        assert_eq!(evaluate("(50+20)/2", &c).unwrap().unwrap().value, "35");
        assert_eq!(
            evaluate("5 ft to cm", &c).unwrap().unwrap().value,
            "152.4 cm"
        );
        assert_eq!(evaluate("1+", &c), Ok(None));
        assert!(evaluate("1 + * 2", &c).is_err());
        c.store(true, Ordering::Release);
        assert_eq!(evaluate("1+2", &c).unwrap(), None);
    }
    #[test]
    fn decimal_currency_math() {
        let c = AtomicBool::new(false);
        assert_eq!(math("0.1 * 1.3", &c).unwrap(), "0.13");
        assert_eq!(
            evaluate("100 usd to USD", &c).unwrap().unwrap().value,
            "100 USD"
        );
    }
    #[test]
    fn cache_refresh_and_fallback() {
        let r = Rate {
            rate: "1.3".into(),
            date: "2026-11-08".into(),
            fetched: 10,
        };
        assert!(
            !choose_rate(Some(r.clone()), 20, || panic!("fresh cache must not fetch"))
                .unwrap()
                .1
        );
        assert!(
            choose_rate(Some(r.clone()), 30000, || Err("offline".into()))
                .unwrap()
                .1
        );
        assert!(choose_rate(None, 30000, || Err("offline".into())).is_err());
        assert!(!valid_date("2026-02-29"));
        assert!(valid_date("2024-02-29"));
        assert!(!valid_rate(
            &Rate {
                rate: "NaN".into(),
                ..r
            },
            20
        ));
    }
}
