//! Black-Scholes-Merton option pricing and Greeks.

/// The two kinds of vanilla option contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    Call,
    Put,
}

/// Structured errors from the pricing core.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlackScholesError {
    NonPositiveInput { field: &'static str, value: f64 },
    /// A non-`s`/`k`/`t`/`sigma` input (`r` or `q`) was non-finite.
    NonFiniteInput { field: &'static str, value: f64 },
    PriceOutOfBounds { market_price: f64 },
    /// The implied-volatility solver hit its iteration cap without converging.
    DidNotConverge,
}

impl std::fmt::Display for BlackScholesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlackScholesError::NonPositiveInput { field, value } => {
                write!(f, "input `{field}` must be finite and positive (was {value})")
            }
            BlackScholesError::NonFiniteInput { field, value } => {
                write!(f, "input `{field}` must be finite (was {value})")
            }
            BlackScholesError::PriceOutOfBounds { market_price } => {
                write!(f, "market price {market_price} is outside no-arbitrage bounds")
            }
            BlackScholesError::DidNotConverge => {
                write!(f, "implied-volatility solver did not converge")
            }
        }
    }
}

impl std::error::Error for BlackScholesError {}

pub const DEFAULT_RISK_FREE_RATE: f64 = 0.04;

/// The validated market/contract inputs to the Black-Scholes-Merton model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlackScholesInputs {
    /// Spot price of the underlying.
    pub s: f64,
    /// Strike price.
    pub k: f64,
    /// Time to expiry, in years.
    pub t: f64,
    /// Continuously-compounded risk-free rate (fraction).
    pub r: f64,
    /// Annualised volatility (fraction).
    pub sigma: f64,
    /// Continuous dividend yield (fraction).
    pub q: f64,
}

impl BlackScholesInputs {
    pub fn new(s: f64, k: f64, t: f64, r: f64, sigma: f64, q: f64) -> Result<Self, BlackScholesError> {
        for (field, value) in [("s", s), ("k", k), ("t", t), ("sigma", sigma)] {
            if !value.is_finite() || value <= 0.0 {
                return Err(BlackScholesError::NonPositiveInput { field, value });
            }
        }
        for (field, value) in [("r", r), ("q", q)] {
            if !value.is_finite() {
                return Err(BlackScholesError::NonFiniteInput { field, value });
            }
        }
        Ok(Self { s, k, t, r, sigma, q })
    }
}

/// The five first-order Greeks for a contract, in the conventions pinned in the module docs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Greeks {
    /// ∂price/∂spot.
    pub delta: f64,
    /// ∂²price/∂spot².
    pub gamma: f64,
    pub theta: f64,
    /// ∂price/∂sigma, **per 1.00 (100%)** vol change.
    pub vega: f64,
    /// ∂price/∂r, **per 1.00 (100%)** rate change.
    pub rho: f64,
}

/// Standard-normal probability density function φ(x).
fn phi(x: f64) -> f64 {
    const INV_SQRT_2PI: f64 = 0.398_942_280_401_432_7;
    INV_SQRT_2PI * (-0.5 * x * x).exp()
}

/// Standard-normal cumulative distribution function N(x).
fn norm_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2))
}

/// Error function via Abramowitz & Stegun 7.1.26 (max abs error ~1.5e-7).
fn erf(x: f64) -> f64 {
    const A1: f64 = 0.254_829_592;
    const A2: f64 = -0.284_496_736;
    const A3: f64 = 1.421_413_741;
    const A4: f64 = -1.453_152_027;
    const A5: f64 = 1.061_405_429;
    const P: f64 = 0.327_591_1;

    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + P * x);
    let y = 1.0 - (((((A5 * t + A4) * t) + A3) * t + A2) * t + A1) * t * (-x * x).exp();
    sign * y
}

/// `d1` term of the Black-Scholes-Merton formula.
fn d1(i: &BlackScholesInputs) -> f64 {
    ((i.s / i.k).ln() + (i.r - i.q + 0.5 * i.sigma * i.sigma) * i.t) / (i.sigma * i.t.sqrt())
}

/// `d2 = d1 - sigma * sqrt(t)`.
fn d2(i: &BlackScholesInputs, d1: f64) -> f64 {
    d1 - i.sigma * i.t.sqrt()
}

/// Black-Scholes-Merton theoretical price for `kind` given validated `inputs`.
pub fn price(inputs: &BlackScholesInputs, kind: OptionKind) -> f64 {
    let d1 = d1(inputs);
    let d2 = d2(inputs, d1);
    let disc_q = (-inputs.q * inputs.t).exp();
    let disc_r = (-inputs.r * inputs.t).exp();
    match kind {
        OptionKind::Call => {
            inputs.s * disc_q * norm_cdf(d1) - inputs.k * disc_r * norm_cdf(d2)
        }
        OptionKind::Put => {
            inputs.k * disc_r * norm_cdf(-d2) - inputs.s * disc_q * norm_cdf(-d1)
        }
    }
}

/// The five first-order Greeks for `kind` given validated `inputs`.
pub fn greeks(inputs: &BlackScholesInputs, kind: OptionKind) -> Greeks {
    let d1 = d1(inputs);
    let d2 = d2(inputs, d1);
    let sqrt_t = inputs.t.sqrt();
    let disc_q = (-inputs.q * inputs.t).exp();
    let disc_r = (-inputs.r * inputs.t).exp();
    let pdf_d1 = phi(d1);

    let gamma = disc_q * pdf_d1 / (inputs.s * inputs.sigma * sqrt_t);
    let vega = inputs.s * disc_q * pdf_d1 * sqrt_t;

    let decay = -inputs.s * disc_q * pdf_d1 * inputs.sigma / (2.0 * sqrt_t);

    let (delta, theta, rho) = match kind {
        OptionKind::Call => {
            let delta = disc_q * norm_cdf(d1);
            let theta = decay - inputs.r * inputs.k * disc_r * norm_cdf(d2)
                + inputs.q * inputs.s * disc_q * norm_cdf(d1);
            let rho = inputs.k * inputs.t * disc_r * norm_cdf(d2);
            (delta, theta, rho)
        }
        OptionKind::Put => {
            let delta = -disc_q * norm_cdf(-d1);
            let theta = decay + inputs.r * inputs.k * disc_r * norm_cdf(-d2)
                - inputs.q * inputs.s * disc_q * norm_cdf(-d1);
            let rho = -inputs.k * inputs.t * disc_r * norm_cdf(-d2);
            (delta, theta, rho)
        }
    };

    Greeks { delta, gamma, theta, vega, rho }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hull's canonical example: S=42, K=40, r=10%, sigma=20%, T=0.5y, no dividend.
    fn hull_inputs() -> BlackScholesInputs {
        BlackScholesInputs::new(42.0, 40.0, 0.5, 0.10, 0.20, 0.0).unwrap()
    }

    #[test]
    fn price_matches_hull_reference_call() {
        // arrange
        let inputs = hull_inputs();

        // act
        let call = price(&inputs, OptionKind::Call);

        // assert
        assert!((call - 4.76).abs() < 0.01, "call was {call}");
    }

    #[test]
    fn price_matches_hull_reference_put() {
        let inputs = hull_inputs();

        let put = price(&inputs, OptionKind::Put);

        assert!((put - 0.81).abs() < 0.01, "put was {put}");
    }

    #[test]
    fn put_call_parity_holds() {
        // arrange
        let inputs = hull_inputs();

        // act
        let call = price(&inputs, OptionKind::Call);
        let put = price(&inputs, OptionKind::Put);

        // assert
        let expected = inputs.s * (-inputs.q * inputs.t).exp()
            - inputs.k * (-inputs.r * inputs.t).exp();
        assert!((call - put - expected).abs() < 1e-9, "parity diff {}", call - put - expected);
    }

    #[test]
    fn call_delta_is_within_zero_and_one() {
        let g = greeks(&hull_inputs(), OptionKind::Call);
        assert!(g.delta >= 0.0 && g.delta <= 1.0, "call delta {}", g.delta);
    }

    #[test]
    fn put_delta_is_within_minus_one_and_zero() {
        let g = greeks(&hull_inputs(), OptionKind::Put);
        assert!(g.delta >= -1.0 && g.delta <= 0.0, "put delta {}", g.delta);
    }

    #[test]
    fn gamma_is_non_negative() {
        let g = greeks(&hull_inputs(), OptionKind::Call);
        assert!(g.gamma >= 0.0, "gamma {}", g.gamma);
    }

    #[test]
    fn vega_is_non_negative() {
        let g = greeks(&hull_inputs(), OptionKind::Call);
        assert!(g.vega >= 0.0, "vega {}", g.vega);
    }

    #[test]
    fn gamma_and_vega_are_kind_independent() {
        let call = greeks(&hull_inputs(), OptionKind::Call);
        let put = greeks(&hull_inputs(), OptionKind::Put);
        assert_eq!(call.gamma, put.gamma);
        assert_eq!(call.vega, put.vega);
    }

    #[test]
    fn delta_relation_between_call_and_put() {
        let call = greeks(&hull_inputs(), OptionKind::Call);
        let put = greeks(&hull_inputs(), OptionKind::Put);
        assert!((call.delta - put.delta - 1.0).abs() < 1e-9);
    }

    #[test]
    fn constructor_rejects_zero_spot() {
        let err = BlackScholesInputs::new(0.0, 40.0, 0.5, 0.10, 0.20, 0.0).unwrap_err();
        assert_eq!(err, BlackScholesError::NonPositiveInput { field: "s", value: 0.0 });
    }

    #[test]
    fn constructor_rejects_negative_volatility() {
        let err = BlackScholesInputs::new(42.0, 40.0, 0.5, 0.10, -0.20, 0.0).unwrap_err();
        assert_eq!(err, BlackScholesError::NonPositiveInput { field: "sigma", value: -0.20 });
    }

    #[test]
    fn constructor_rejects_non_finite_rate() {
        let err = BlackScholesInputs::new(42.0, 40.0, 0.5, f64::NAN, 0.20, 0.0).unwrap_err();
        assert!(matches!(err, BlackScholesError::NonFiniteInput { field: "r", .. }));
    }

    #[test]
    fn constructor_accepts_negative_rate() {
        assert!(BlackScholesInputs::new(42.0, 40.0, 0.5, -0.01, 0.20, 0.0).is_ok());
    }

    #[test]
    fn norm_cdf_is_one_half_at_zero() {
        assert!((norm_cdf(0.0) - 0.5).abs() < 1e-7);
    }

    #[test]
    fn norm_cdf_matches_known_quantile() {
        assert!((norm_cdf(1.96) - 0.975).abs() < 1e-4, "N(1.96) = {}", norm_cdf(1.96));
    }
}
