use chrono::{DateTime, Utc};

use crate::black_scholes::{
    BlackScholesInputs, Greeks, OptionKind, greeks as bs_greeks, implied_volatility,
    price as bs_price,
};

#[derive(Debug, Clone)]
pub struct OptionContract {
    /// Call or put.
    pub kind: OptionKind,
    pub expiry: DateTime<Utc>,
    pub strike: f64,
    /// Best bid premium.
    pub bid: Option<f64>,
    /// Best ask premium.
    pub ask: Option<f64>,
    /// Last traded premium.
    pub last: Option<f64>,
    /// Provider's implied volatility as a fraction (`0.25` = 25%).
    pub implied_volatility: Option<f64>,
    pub volume: Option<u64>,
    pub open_interest: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct OptionChain {
    /// The underlying ticker symbol.
    pub underlying: String,
    pub underlying_price: Option<f64>,
    /// All expirations offered for this underlying, most-recent-first as the provider returns them.
    pub expirations: Vec<DateTime<Utc>>,
    /// Contracts (calls and puts) for the currently-fetched expiry.
    pub contracts: Vec<OptionContract>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContractAnalytics {
    /// Theoretical (fair) premium from Black-Scholes-Merton.
    pub theo_price: f64,
    pub iv: f64,
    /// First-order Greeks in the core's conventions (theta per year, vega/rho per 1.00 vol/rate move).
    pub greeks: Greeks,
}

fn year_fraction(expiry: DateTime<Utc>, now: DateTime<Utc>) -> f64 {
    (expiry - now).num_seconds() as f64 / (365.0 * 86_400.0)
}

fn contract_market_price(contract: &OptionContract) -> Option<f64> {
    match (contract.bid, contract.ask) {
        (Some(bid), Some(ask)) => Some((bid + ask) / 2.0),
        _ => contract.last,
    }
}

impl OptionContract {
    /// Computes this contract's theoretical price and Greeks as of `now`.
    pub fn analytics(
        &self,
        spot: Option<f64>,
        risk_free_rate: f64,
        dividend_yield: f64,
        now: DateTime<Utc>,
    ) -> Option<ContractAnalytics> {
        let spot = spot?;
        let t = year_fraction(self.expiry, now);

        let iv = match self.implied_volatility {
            Some(iv) if iv > 0.0 => iv,
            _ => {
                let market = contract_market_price(self)?;
                let seed = BlackScholesInputs::new(
                    spot,
                    self.strike,
                    t,
                    risk_free_rate,
                    0.5,
                    dividend_yield,
                )
                .ok()?;
                implied_volatility(market, &seed, self.kind).ok()?
            }
        };

        let inputs =
            BlackScholesInputs::new(spot, self.strike, t, risk_free_rate, iv, dividend_yield)
                .ok()?;
        Some(ContractAnalytics {
            theo_price: bs_price(&inputs, self.kind),
            iv,
            greeks: bs_greeks(&inputs, self.kind),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::black_scholes::{BlackScholesInputs, DEFAULT_RISK_FREE_RATE, OptionKind, price};
    use chrono::Duration as ChronoDuration;
    use chrono::Utc;

    fn one_year_contract(strike: f64) -> (OptionContract, chrono::DateTime<Utc>) {
        let now = Utc::now();
        let contract = OptionContract {
            kind: OptionKind::Call,
            expiry: now + ChronoDuration::days(365),
            strike,
            bid: None,
            ask: None,
            last: None,
            implied_volatility: None,
            volume: None,
            open_interest: None,
        };
        (contract, now)
    }

    #[test]
    fn year_fraction_one_year_out_is_one() {
        let now = Utc::now();
        let expiry = now + ChronoDuration::days(365);
        assert!((year_fraction(expiry, now) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn analytics_uses_provider_iv_when_present() {
        // arrange
        let (mut contract, now) = one_year_contract(100.0);
        contract.implied_volatility = Some(0.25);

        // act
        let a = contract
            .analytics(Some(100.0), DEFAULT_RISK_FREE_RATE, 0.0, now)
            .unwrap();

        // assert
        assert!((a.iv - 0.25).abs() < 1e-12);
        let expected = price(
            &BlackScholesInputs::new(100.0, 100.0, 1.0, DEFAULT_RISK_FREE_RATE, 0.25, 0.0).unwrap(),
            OptionKind::Call,
        );
        assert!((a.theo_price - expected).abs() < 1e-9);
    }

    #[test]
    fn analytics_solves_iv_from_mid_when_provider_iv_absent() {
        // arrange
        let (mut contract, now) = one_year_contract(100.0);
        let true_vol = 0.30;
        let market = price(
            &BlackScholesInputs::new(100.0, 100.0, 1.0, DEFAULT_RISK_FREE_RATE, true_vol, 0.0)
                .unwrap(),
            OptionKind::Call,
        );
        contract.bid = Some(market);
        contract.ask = Some(market);

        // act
        let a = contract
            .analytics(Some(100.0), DEFAULT_RISK_FREE_RATE, 0.0, now)
            .unwrap();

        // assert
        assert!((a.iv - true_vol).abs() < 1e-4, "solved iv {}", a.iv);
    }

    #[test]
    fn analytics_none_without_spot() {
        let (mut contract, now) = one_year_contract(100.0);
        contract.implied_volatility = Some(0.25);
        assert!(
            contract
                .analytics(None, DEFAULT_RISK_FREE_RATE, 0.0, now)
                .is_none()
        );
    }

    #[test]
    fn analytics_none_for_expired_contract() {
        let now = Utc::now();
        let contract = OptionContract {
            kind: OptionKind::Call,
            expiry: now - ChronoDuration::days(1),
            strike: 100.0,
            bid: None,
            ask: None,
            last: None,
            implied_volatility: Some(0.25),
            volume: None,
            open_interest: None,
        };
        assert!(
            contract
                .analytics(Some(100.0), DEFAULT_RISK_FREE_RATE, 0.0, now)
                .is_none()
        );
    }

    #[test]
    fn analytics_none_when_no_iv_and_no_market_price() {
        let (contract, now) = one_year_contract(100.0);
        assert!(
            contract
                .analytics(Some(100.0), DEFAULT_RISK_FREE_RATE, 0.0, now)
                .is_none()
        );
    }
}
