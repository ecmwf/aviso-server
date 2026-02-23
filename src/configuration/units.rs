use std::time::Duration;

/// Parses duration literals used in configuration.
///
/// Accepted examples: `50s`, `10m`, `1h`, `10d`, `1w`.
/// Rejected examples: `10` (no unit), `1mo` (unsupported unit), `abc`.
/// Note: leading/trailing whitespace is ignored (for example `" 10m "`).
pub fn parse_duration_spec(input: &str) -> Result<Duration, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("duration must not be empty".to_string());
    }
    let split_at = trimmed
        .find(|ch: char| !ch.is_ascii_digit())
        .ok_or_else(|| "duration must include a unit suffix".to_string())?;
    let (value_str, unit) = trimmed.split_at(split_at);
    if unit.is_empty() {
        return Err("duration must include a unit suffix".to_string());
    }
    let value = value_str
        .parse::<u64>()
        .map_err(|_| "duration value must be an integer".to_string())?;
    let seconds = match unit.to_ascii_lowercase().as_str() {
        "s" => value,
        "m" => value
            .checked_mul(60)
            .ok_or_else(|| "duration overflow".to_string())?,
        "h" => value
            .checked_mul(60 * 60)
            .ok_or_else(|| "duration overflow".to_string())?,
        "d" => value
            .checked_mul(24 * 60 * 60)
            .ok_or_else(|| "duration overflow".to_string())?,
        "w" => value
            .checked_mul(7 * 24 * 60 * 60)
            .ok_or_else(|| "duration overflow".to_string())?,
        _ => {
            return Err("duration unit must be one of: s, m, h, d, w".to_string());
        }
    };
    Ok(Duration::from_secs(seconds))
}

/// Parses retention window literals for JetStream defaults and per-schema policies.
///
/// This wrapper keeps retention parsing on the same code path everywhere.
pub fn parse_retention_time_spec(input: &str) -> Result<Duration, String> {
    parse_duration_spec(input)
}

/// Parses byte-size literals used in configuration.
///
/// Accepted examples: `100M`, `100Mi`, `64Ki`, `1G`, `1Ti`.
/// Rejected examples: `10` (no unit), `10m` (wrong case), `0Mi`, `abc`.
/// Note: units are case-sensitive (`M` valid, `m` invalid).
pub fn parse_size_spec(input: &str) -> Result<i64, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("size must not be empty".to_string());
    }
    let split_at = trimmed
        .find(|ch: char| !ch.is_ascii_digit())
        .ok_or_else(|| "size must include a unit suffix".to_string())?;
    let (value_str, unit) = trimmed.split_at(split_at);
    if unit.is_empty() {
        return Err("size must include a unit suffix".to_string());
    }
    let value = value_str
        .parse::<i64>()
        .map_err(|_| "size value must be an integer".to_string())?;
    if value <= 0 {
        return Err("size value must be greater than zero".to_string());
    }
    let multiplier = match unit {
        "K" => 1_000_i64,
        "M" => 1_000_000_i64,
        "G" => 1_000_000_000_i64,
        "T" => 1_000_000_000_000_i64,
        "Ki" => 1_024_i64,
        "Mi" => 1_048_576_i64,
        "Gi" => 1_073_741_824_i64,
        "Ti" => 1_099_511_627_776_i64,
        _ => {
            return Err("size unit must be one of: K, Ki, M, Mi, G, Gi, T, Ti".to_string());
        }
    };
    value
        .checked_mul(multiplier)
        .ok_or_else(|| "size overflow".to_string())
}

#[cfg(test)]
mod tests {
    use super::{parse_duration_spec, parse_retention_time_spec, parse_size_spec};
    use std::time::Duration;

    #[test]
    fn duration_parser_accepts_supported_units() {
        assert_eq!(parse_duration_spec("50s").unwrap(), Duration::from_secs(50));
        assert_eq!(
            parse_duration_spec("10m").unwrap(),
            Duration::from_secs(600)
        );
        assert_eq!(
            parse_duration_spec("1h").unwrap(),
            Duration::from_secs(3600)
        );
        assert_eq!(
            parse_duration_spec("2d").unwrap(),
            Duration::from_secs(172_800)
        );
        assert_eq!(
            parse_duration_spec("1w").unwrap(),
            Duration::from_secs(604_800)
        );
    }

    #[test]
    fn duration_parser_rejects_invalid_values() {
        assert!(parse_duration_spec("").is_err());
        assert!(parse_duration_spec("10").is_err());
        assert!(parse_duration_spec("abc").is_err());
        assert!(parse_duration_spec("10x").is_err());
    }

    #[test]
    fn retention_parser_reuses_duration_rules() {
        assert_eq!(
            parse_retention_time_spec("7d").unwrap(),
            Duration::from_secs(604_800)
        );
        assert!(parse_retention_time_spec("7x").is_err());
    }

    #[test]
    fn size_parser_accepts_supported_units() {
        assert_eq!(parse_size_spec("100K").unwrap(), 100_000);
        assert_eq!(parse_size_spec("100Ki").unwrap(), 102_400);
        assert_eq!(parse_size_spec("2M").unwrap(), 2_000_000);
        assert_eq!(parse_size_spec("2Mi").unwrap(), 2_097_152);
        assert_eq!(parse_size_spec("1G").unwrap(), 1_000_000_000);
    }

    #[test]
    fn size_parser_rejects_invalid_values() {
        assert!(parse_size_spec("").is_err());
        assert!(parse_size_spec("10").is_err());
        assert!(parse_size_spec("0Mi").is_err());
        assert!(parse_size_spec("10m").is_err());
        assert!(parse_size_spec("abc").is_err());
    }
}
