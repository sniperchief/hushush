//! Money is always an integer number of zatoshis.

/// 1 ZEC in zatoshis.
pub const COIN: i64 = 100_000_000;

/// Total Zcash supply (21,000,000 ZEC) in zatoshis; upper bound for any amount.
pub const MAX_MONEY_ZATS: i64 = 21_000_000 * COIN;

/// Parses a decimal ZEC string ("1.5", "0.00000001") into zatoshis exactly.
/// Rejects more than 8 decimal places, signs, exponents and out-of-range values.
pub fn parse_zec(s: &str) -> Option<i64> {
    let s = s.trim();
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    if whole.is_empty() && frac.is_empty()
        || frac.len() > 8
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let frac: i64 = if frac.is_empty() {
        0
    } else {
        format!("{frac:0<8}").parse().ok()?
    };
    let zats = whole.checked_mul(COIN)?.checked_add(frac)?;
    (zats <= MAX_MONEY_ZATS).then_some(zats)
}

/// Formats zatoshis as a decimal ZEC string with 8 places.
pub fn format_zec(zats: i64) -> String {
    let sign = if zats < 0 { "-" } else { "" };
    let a = zats.unsigned_abs();
    format!("{sign}{}.{:08}", a / COIN as u64, a % COIN as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exactly() {
        assert_eq!(parse_zec("1"), Some(COIN));
        assert_eq!(parse_zec("0.00000001"), Some(1));
        assert_eq!(parse_zec("1.5"), Some(150_000_000));
        assert_eq!(parse_zec(".5"), Some(50_000_000));
        assert_eq!(parse_zec("21000000"), Some(MAX_MONEY_ZATS));
        // 0.1 + 0.2 style float error is impossible here.
        assert_eq!(
            parse_zec("0.1").unwrap() + parse_zec("0.2").unwrap(),
            parse_zec("0.3").unwrap()
        );
    }

    #[test]
    fn rejects_bad_input() {
        for bad in [
            "",
            ".",
            "-1",
            "+1",
            "1e8",
            "0.000000001",
            "21000000.00000001",
            "1.2.3",
            "abc",
        ] {
            assert_eq!(parse_zec(bad), None, "{bad}");
        }
    }

    #[test]
    fn formats() {
        assert_eq!(format_zec(150_000_000), "1.50000000");
        assert_eq!(format_zec(1), "0.00000001");
        assert_eq!(format_zec(-5), "-0.00000005");
    }
}
