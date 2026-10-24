//! Small formatting helpers shared across the UI.

use std::time::Duration;

/// 1234 -> "1.2K", 5_300_000 -> "5.3M"
pub fn compact_num(n: i64) -> String {
    let a = n.unsigned_abs() as f64;
    let sign = if n < 0 { "-" } else { "" };
    if a < 1_000.0 {
        format!("{n}")
    } else if a < 1_000_000.0 {
        format!("{sign}{:.1}K", a / 1_000.0)
    } else if a < 1_000_000_000.0 {
        format!("{sign}{:.1}M", a / 1_000_000.0)
    } else {
        format!("{sign}{:.1}B", a / 1_000_000_000.0)
    }
}

/// 1234567 -> "1,234,567"
pub fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn duration(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1.0 {
        format!("{:.2} ms", ms)
    } else if ms < 1000.0 {
        format!("{:.0} ms", ms)
    } else if ms < 60_000.0 {
        format!("{:.2} s", ms / 1000.0)
    } else {
        let s = d.as_secs();
        format!("{}m {:02}s", s / 60, s % 60)
    }
}

pub fn bytes(n: usize) -> String {
    let n = n as f64;
    if n < 1024.0 {
        format!("{n} B")
    } else if n < 1024.0 * 1024.0 {
        format!("{:.1} KB", n / 1024.0)
    } else if n < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} MB", n / 1024.0 / 1024.0)
    } else {
        format!("{:.2} GB", n / 1024.0 / 1024.0 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn nums() {
        assert_eq!(super::thousands(1234567), "1,234,567");
        assert_eq!(super::thousands(12), "12");
        assert_eq!(super::compact_num(1500), "1.5K");
    }
}
