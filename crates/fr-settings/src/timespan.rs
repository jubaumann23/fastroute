//! `TextManager.parseTimespanString`: "HH:mm:ss", "mm:ss" or "ss" to seconds,
//! via an ISO-8601 `PT..H..M..S` string and `java.time.Duration.parse`.

use crate::jutil::java_split;

/// Returns `None` for blank or unparsable input (Java returns `null`, i.e. no limit).
/// Note "5m" or "300s" are *not* accepted (they become `PT5mS` / `PT300sS`).
pub fn parse_timespan_string(s: &str) -> Option<i64> {
    if s.chars().all(char::is_whitespace) {
        return None;
    }
    let parts = java_split(s, |c| c == ':');
    let iso = match parts.as_slice() {
        [h, m, sec] => format!("PT{h}H{m}M{sec}S"),
        [m, sec] => format!("PT{m}M{sec}S"),
        [sec] => format!("PT{sec}S"),
        _ => "PT".to_string(),
    };
    parse_iso_time_duration(&iso)
}

/// Subset of `Duration.parse` for strings starting with "PT" (hours, minutes,
/// seconds with optional fraction). Returns `Duration.getSeconds()`.
fn parse_iso_time_duration(text: &str) -> Option<i64> {
    let rest = text.strip_prefix("PT")?;
    if rest.is_empty() {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut pos = 0;
    let mut total_nanos: i128 = 0;
    let mut stage = 0; // 0: expect H/M/S, 1: after H, 2: after M, 3: after S
    while pos < bytes.len() {
        let start = pos;
        if matches!(bytes[pos], b'+' | b'-') {
            pos += 1;
        }
        let digits_start = pos;
        while pos < bytes.len() && bytes[pos].is_ascii_digit() {
            pos += 1;
        }
        if pos == digits_start {
            return None;
        }
        let whole: i128 = rest[start..pos].parse().ok()?;
        let negative = bytes[start] == b'-';
        let mut frac_nanos: i128 = 0;
        if pos < bytes.len() && matches!(bytes[pos], b'.' | b',') {
            pos += 1;
            let fs = pos;
            while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                pos += 1;
            }
            let frac = &rest[fs..pos];
            if frac.len() > 9 {
                return None;
            }
            if !frac.is_empty() {
                let padded = format!("{frac:0<9}");
                frac_nanos = padded.parse().ok()?;
            }
            if pos >= bytes.len() || !bytes[pos].eq_ignore_ascii_case(&b'S') {
                return None;
            }
        }
        let unit = *bytes.get(pos)?;
        pos += 1;
        let (new_stage, secs_per_unit) = match unit.to_ascii_uppercase() {
            b'H' => (1, 3600),
            b'M' => (2, 60),
            b'S' => (3, 1),
            _ => return None,
        };
        if new_stage <= stage {
            return None;
        }
        stage = new_stage;
        let signed_frac = if negative { -frac_nanos } else { frac_nanos };
        total_nanos += whole * secs_per_unit * 1_000_000_000 + signed_frac;
    }
    let secs = total_nanos.div_euclid(1_000_000_000);
    i64::try_from(secs).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timespans() {
        assert_eq!(parse_timespan_string("12:00:00"), Some(43200));
        assert_eq!(parse_timespan_string("05:30"), Some(330));
        assert_eq!(parse_timespan_string("90"), Some(90));
        assert_eq!(parse_timespan_string("1.5"), Some(1));
        assert_eq!(parse_timespan_string("-1.5"), Some(-2));
        assert_eq!(parse_timespan_string("5m"), None);
        assert_eq!(parse_timespan_string(""), None);
        assert_eq!(parse_timespan_string("1:2:3:4"), None);
    }
}
