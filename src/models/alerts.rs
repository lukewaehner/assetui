use chrono::{DateTime, Utc};

use super::ticker::split_ticker_entry;

/// Which side of the threshold a price alert fires on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertDirection {
    /// Fire when the price rises to or above the threshold.
    Above,
    /// Fire when the price falls to or below the threshold.
    Below,
}

impl AlertDirection {
    pub fn label(&self) -> &'static str {
        match self {
            AlertDirection::Above => "above",
            AlertDirection::Below => "below",
        }
    }
}

/// Why a user's alert spec (`>150` / `<100`) could not be parsed.
#[derive(Debug, Clone, PartialEq)]
pub enum AlertParseError {
    /// No leading `>` or `<`, so the direction is ambiguous.
    MissingOperator,
    /// The text after the operator wasn't a number.
    InvalidNumber(String),
    /// The threshold parsed but wasn't a finite, strictly-positive price.
    NonPositive(f64),
}

impl std::fmt::Display for AlertParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertParseError::MissingOperator => {
                write!(f, "start with > (above) or < (below), e.g. >150")
            }
            AlertParseError::InvalidNumber(s) => write!(f, "not a number: {s}"),
            AlertParseError::NonPositive(v) => write!(f, "threshold must be positive (got {v})"),
        }
    }
}

impl std::error::Error for AlertParseError {}

/// Parses a compact alert spec into a direction and threshold.
pub fn parse_alert_input(input: &str) -> Result<(AlertDirection, f64), AlertParseError> {
    let s = input.trim();
    let (direction, rest) = if let Some(rest) = s.strip_prefix('>') {
        (AlertDirection::Above, rest)
    } else if let Some(rest) = s.strip_prefix('<') {
        (AlertDirection::Below, rest)
    } else {
        return Err(AlertParseError::MissingOperator);
    };

    let rest = rest.trim();
    let value: f64 = rest
        .parse()
        .map_err(|_| AlertParseError::InvalidNumber(rest.to_string()))?;
    if !value.is_finite() || value <= 0.0 {
        return Err(AlertParseError::NonPositive(value));
    }
    Ok((direction, value))
}

pub fn parse_alert_entry(
    input: &str,
) -> Result<(String, AlertDirection, f64), AlertParseError> {
    let (ticker, rest) =
        split_ticker_entry(input).ok_or(AlertParseError::MissingOperator)?;
    let (direction, threshold) = parse_alert_input(rest)?;
    Ok((ticker, direction, threshold))
}

pub fn crosses(direction: AlertDirection, threshold: f64, price: f64) -> bool {
    match direction {
        AlertDirection::Above => price >= threshold,
        AlertDirection::Below => price <= threshold,
    }
}

/// A persisted price alert: fire once when `ticker` crosses `threshold` in `direction`.
#[derive(Debug, Clone)]
pub struct Alert {
    /// Database-assigned row id.
    pub id: i32,
    /// Ticker the alert watches (stored uppercased).
    pub ticker: String,
    /// Which side of `threshold` triggers the alert.
    pub direction: AlertDirection,
    /// Absolute price threshold.
    pub threshold: f64,
    pub created_at: DateTime<Utc>,
    /// When the alert fired.
    pub triggered_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_alert_input_above() {
        assert_eq!(parse_alert_input(">150"), Ok((AlertDirection::Above, 150.0)));
    }

    #[test]
    fn parse_alert_input_below() {
        assert_eq!(parse_alert_input("<100"), Ok((AlertDirection::Below, 100.0)));
    }

    #[test]
    fn parse_alert_input_tolerates_whitespace() {
        assert_eq!(parse_alert_input("  > 150.5 "), Ok((AlertDirection::Above, 150.5)));
    }

    #[test]
    fn parse_alert_input_missing_operator_is_error() {
        assert_eq!(parse_alert_input("150"), Err(AlertParseError::MissingOperator));
    }

    #[test]
    fn parse_alert_input_empty_is_error() {
        assert_eq!(parse_alert_input("   "), Err(AlertParseError::MissingOperator));
    }

    #[test]
    fn parse_alert_input_non_numeric_is_error() {
        assert_eq!(
            parse_alert_input(">abc"),
            Err(AlertParseError::InvalidNumber("abc".to_string()))
        );
    }

    #[test]
    fn parse_alert_input_negative_is_error() {
        assert_eq!(parse_alert_input(">-5"), Err(AlertParseError::NonPositive(-5.0)));
    }

    #[test]
    fn parse_alert_input_zero_is_error() {
        assert_eq!(parse_alert_input("<0"), Err(AlertParseError::NonPositive(0.0)));
    }

    #[test]
    fn parse_alert_entry_valid() {
        assert_eq!(parse_alert_entry("aapl >150"), Ok(("AAPL".to_string(), AlertDirection::Above, 150.0)));
        assert_eq!(parse_alert_entry("TSLA <100"), Ok(("TSLA".to_string(), AlertDirection::Below, 100.0)));
    }

    #[test]
    fn parse_alert_entry_missing_spec_is_error() {
        assert_eq!(parse_alert_entry("AAPL"), Err(AlertParseError::MissingOperator));
    }

    #[test]
    fn parse_alert_entry_propagates_spec_errors() {
        assert_eq!(parse_alert_entry("AAPL 150"), Err(AlertParseError::MissingOperator));
        assert_eq!(parse_alert_entry("AAPL >-5"), Err(AlertParseError::NonPositive(-5.0)));
    }

    #[test]
    fn crosses_above_fires_at_or_past_threshold() {
        assert!(crosses(AlertDirection::Above, 150.0, 151.0), "just above");
        assert!(crosses(AlertDirection::Above, 150.0, 150.0), "exactly at");
        assert!(!crosses(AlertDirection::Above, 150.0, 149.99), "just below");
    }

    #[test]
    fn crosses_below_fires_at_or_past_threshold() {
        assert!(crosses(AlertDirection::Below, 100.0, 99.0), "just below");
        assert!(crosses(AlertDirection::Below, 100.0, 100.0), "exactly at");
        assert!(!crosses(AlertDirection::Below, 100.0, 100.01), "just above");
    }

    #[test]
    fn alert_direction_label_maps_each_variant() {
        assert_eq!(AlertDirection::Above.label(), "above");
        assert_eq!(AlertDirection::Below.label(), "below");
    }
}
