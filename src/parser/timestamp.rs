//! Timestamp parsing utilities for `WoW` combat log lines.

// ── Timestamp Parsing ───────────────────────────────────────────────────────

/// Fast manual timestamp parsing — no regex, no allocation.
///
/// Format: `MM/DD HH:MM:SS.mmm  ...`
///
/// Returns `(seconds_value, end_of_timestamp_index)` or `None`.
#[inline]
pub(crate) fn parse_timestamp_fast(line: &[u8]) -> Option<(f64, usize)> {
    // minimum: "1/1 0:0:0.0"
    if line.len() < 11 || !line[0].is_ascii_digit() {
        return None;
    }

    let slash = memchr(b'/', line, 0, 5)?;
    let month = f64::from(parse_int_fast(line, 0, slash)?);

    let space = memchr(b' ', line, slash + 1, slash + 4)?;
    let day = f64::from(parse_int_fast(line, slash + 1, space)?);

    let c1 = memchr(b':', line, space + 1, space + 4)?;
    let hour = f64::from(parse_int_fast(line, space + 1, c1)?);

    let c2 = memchr(b':', line, c1 + 1, c1 + 4)?;
    let min = f64::from(parse_int_fast(line, c1 + 1, c2)?);

    let dot = memchr(b'.', line, c2 + 1, c2 + 4)?;
    let sec = f64::from(parse_int_fast(line, c2 + 1, dot)?);

    // Parse ms digits (variable length, typically 3)
    let mut ms_end = dot + 1;
    while ms_end < line.len() && line[ms_end].is_ascii_digit() {
        ms_end += 1;
    }
    if ms_end == dot + 1 {
        return None;
    }
    let ms = f64::from(parse_int_fast(line, dot + 1, ms_end)?);

    let secs =
        (month * 31.0 + day).mul_add(86400.0, hour * 3600.0) + min * 60.0 + sec + ms / 1000.0;
    Some((secs, ms_end))
}

/// Extract the timestamp substring from a line (for string comparison in formatter).
#[inline]
pub(super) fn extract_timestamp_str(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    parse_timestamp_fast(bytes).map(|(_, end)| &line[..end])
}

#[inline]
fn memchr(needle: u8, haystack: &[u8], start: usize, max_end: usize) -> Option<usize> {
    let end = max_end.min(haystack.len());
    (start..end).find(|&i| haystack[i] == needle)
}

#[inline]
fn parse_int_fast(bytes: &[u8], start: usize, end: usize) -> Option<u32> {
    if start >= end {
        return None;
    }
    let mut val: u32 = 0;
    for &b in &bytes[start..end] {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val * 10 + u32::from(b - b'0');
    }
    Some(val)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn format_duration(seconds: f64) -> String {
    if seconds < 60.0 {
        format!("{}s", seconds as u64)
    } else if seconds < 3600.0 {
        format!("{}m {}s", (seconds / 60.0) as u64, (seconds % 60.0) as u64)
    } else {
        let hours = (seconds / 3600.0) as u64;
        let mins = ((seconds % 3600.0) / 60.0) as u64;
        format!("{hours}h {mins}m")
    }
}
