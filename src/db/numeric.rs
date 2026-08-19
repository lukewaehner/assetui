//! Conversions between the `f64` domain and Postgres `NUMERIC` columns.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::AppError;

/// Converts a domain `f64` into a `NUMERIC`-bindable [`Decimal`].
pub fn to_numeric(value: f64, field: &'static str) -> Result<Decimal, AppError> {
    Decimal::try_from(value)
        .map_err(|e| AppError::Data(format!("cannot store {field} as numeric ({value}): {e}")))
}

/// Converts a `NUMERIC` column back into the domain's `f64`.
pub fn from_numeric(value: Decimal, field: &'static str) -> Result<f64, AppError> {
    value
        .to_f64()
        .ok_or_else(|| AppError::Data(format!("{field} does not fit in an f64: {value}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_numeric_stores_the_shortest_decimal_form_of_a_drifted_sum() {
        let stored = to_numeric(2811.8999999999996, "market_value").unwrap();

        assert_eq!(stored.to_string(), "2811.9");
    }

    /// The classic float surprise, which would otherwise be written to disk.
    #[test]
    fn to_numeric_collapses_accumulated_addition_error() {
        let stored = to_numeric(0.1 + 0.2, "market_value").unwrap();

        assert_eq!(stored.to_string(), "0.3");
    }

    /// A price the user typed survives exactly, rather than as 150.0999...
    #[test]
    fn to_numeric_preserves_a_user_entered_cost() {
        let stored = to_numeric(150.10, "avg_cost").unwrap();

        assert_eq!(stored.to_string(), "150.1");
    }

    #[test]
    fn to_numeric_rejects_a_non_finite_value() {
        let err = to_numeric(f64::NAN, "market_value").unwrap_err();

        assert!(
            err.to_string().contains("market_value"),
            "error should name the field: {err}"
        );
    }

    #[test]
    fn from_numeric_round_trips_a_stored_value() {
        let stored = to_numeric(2839.91, "market_value").unwrap();

        assert_eq!(from_numeric(stored, "market_value").unwrap(), 2839.91);
    }
}
