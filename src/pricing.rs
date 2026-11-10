//! Advisory prices from provider metadata and validated challenges.
pub mod advertised;
pub(crate) mod runtime;

use alloy_primitives::U256;

/// A validated target offer, expressed in atomic USDC units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservedPrice {
    pub amount: U256,
}

impl std::fmt::Display for ObservedPrice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let digits = self.amount.to_string();
        if digits.len() <= 6 {
            let fractional = format!("{digits:0>6}");
            let fractional = fractional.trim_end_matches('0');
            if fractional.is_empty() {
                f.write_str("0")
            } else {
                write!(f, "0.{fractional}")
            }
        } else {
            let (whole, fractional) = digits.split_at(digits.len() - 6);
            let fractional = fractional.trim_end_matches('0');
            f.write_str(whole)?;
            if !fractional.is_empty() {
                write!(f, ".{fractional}")?;
            }
            Ok(())
        }
    }
}

#[derive(Debug)]
struct ObservedPriceError {
    price: Option<ObservedPrice>,
    display: String,
}

impl std::fmt::Display for ObservedPriceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display)
    }
}

impl std::error::Error for ObservedPriceError {}

impl ObservedPrice {
    pub(crate) fn attach(self, error: anyhow::Error) -> anyhow::Error {
        if Self::from_error(&error) == Some(self) {
            return error;
        }
        let display = error.to_string();
        error.context(ObservedPriceError {
            price: Some(self),
            display,
        })
    }

    /// Combined management output has no single target offer.
    pub(crate) fn omit(error: anyhow::Error) -> anyhow::Error {
        if Self::from_error(&error).is_none() {
            return error;
        }
        let display = error.to_string();
        error.context(ObservedPriceError {
            price: None,
            display,
        })
    }

    pub(crate) fn from_error(error: &anyhow::Error) -> Option<Self> {
        error
            .downcast_ref::<ObservedPriceError>()
            .and_then(|context| context.price)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_prices_format_exact_usdc_decimals() {
        for (amount, expected) in [
            (U256::ZERO, "0"),
            (U256::from(1), "0.000001"),
            (U256::from(10), "0.00001"),
            (U256::from(5_000), "0.005"),
            (U256::from(1_000_000), "1"),
            (U256::from(12_340_000), "12.34"),
            (
                U256::MAX,
                "115792089237316195423570985008687907853269984665640564039457584007913129.639935",
            ),
        ] {
            assert_eq!(ObservedPrice { amount }.to_string(), expected);
        }
    }

    #[test]
    fn observed_error_context_preserves_display_and_typed_causes() {
        let price = ObservedPrice {
            amount: U256::from(17),
        };
        let original = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))
            .context("response unavailable");
        let expected_display = original.to_string();
        let error = price.attach(original);
        assert_eq!(error.to_string(), expected_display);
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
        assert_eq!(ObservedPrice::from_error(&error), Some(price));
        let depth = error.chain().count();
        let error = price.attach(error);
        assert_eq!(error.chain().count(), depth);
        let latest = ObservedPrice {
            amount: U256::from(18),
        };
        let error = latest.attach(error);
        assert_eq!(ObservedPrice::from_error(&error), Some(latest));
        assert_eq!(error.to_string(), expected_display);
        let error = crate::payment::with_payment_context::<()>(true, Err(error)).unwrap_err();
        let display = error.to_string();
        let error = ObservedPrice::omit(error);
        assert!(ObservedPrice::from_error(&error).is_none());
        assert_eq!(error.to_string(), display);
        assert!(error.is::<crate::payment::PossiblePayment>());
        assert!(error.is::<std::io::Error>());
        let error = price.attach(error);
        assert_eq!(ObservedPrice::from_error(&error), Some(price));
    }
}
