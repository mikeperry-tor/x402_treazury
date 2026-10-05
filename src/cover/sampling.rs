//! Bounded application-byte/timer sampling, never packet-level shaping.
use anyhow::{Result, bail, ensure};
use rand::{Rng, distr::Open01};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Uniform,
    Exponential,
    Weibull,
    LogNormal,
    WeightedDiscrete,
}

/// Unit-bearing fields deliberately remain explicit in the public TOML schema.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Distribution {
    pub distribution: Family,
    pub min_bytes: Option<u64>,
    pub max_bytes: Option<u64>,
    pub min_ms: Option<u64>,
    pub max_ms: Option<u64>,
    pub mean_bytes: Option<f64>,
    pub mean_ms: Option<f64>,
    pub scale_bytes: Option<f64>,
    pub scale_ms: Option<f64>,
    pub median_bytes: Option<f64>,
    pub median_ms: Option<f64>,
    pub sigma: Option<f64>,
    pub shape: Option<f64>,
    pub values_bytes: Option<Vec<u64>>,
    pub values_ms: Option<Vec<u64>>,
    pub weights: Option<Vec<u64>>,
}
#[derive(Clone, Copy)]
pub enum Unit {
    Bytes,
    Milliseconds,
}

type UnitFields<'a> = (
    Option<u64>,
    Option<u64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<&'a Vec<u64>>,
);
impl Distribution {
    fn fields(&self, unit: Unit) -> UnitFields<'_> {
        match unit {
            Unit::Bytes => (
                self.min_bytes,
                self.max_bytes,
                self.mean_bytes,
                self.scale_bytes,
                self.median_bytes,
                self.values_bytes.as_ref(),
            ),
            Unit::Milliseconds => (
                self.min_ms,
                self.max_ms,
                self.mean_ms,
                self.scale_ms,
                self.median_ms,
                self.values_ms.as_ref(),
            ),
        }
    }
    pub fn bounds(&self, unit: Unit) -> Result<(u64, u64)> {
        let (l, h, _, _, _, _) = self.fields(unit);
        Ok((
            l.ok_or_else(|| anyhow::anyhow!("distribution minimum required"))?,
            h.ok_or_else(|| anyhow::anyhow!("distribution maximum required"))?,
        ))
    }
    pub fn validate(&self, unit: Unit, zero: bool) -> Result<()> {
        let opposite = match unit {
            Unit::Bytes => Unit::Milliseconds,
            Unit::Milliseconds => Unit::Bytes,
        };
        let (l, h, m, s, d, v) = self.fields(opposite);
        ensure!(
            l.is_none() && h.is_none() && m.is_none() && s.is_none() && d.is_none() && v.is_none(),
            "distribution has wrong-unit fields"
        );
        let (l, h) = self.bounds(unit)?;
        ensure!(
            l <= h && (zero || l > 0) && h < (1u64 << 53),
            "invalid distribution bounds or floating-point precision"
        );
        let (_, _, mean, scale, median, values) = self.fields(unit);
        let positive = |x: Option<f64>| x.is_some_and(|x| x.is_finite() && x > 0.0);
        ensure!(
            mean.is_none() || self.distribution == Family::Exponential,
            "mean only applies to exponential"
        );
        ensure!(
            scale.is_none() && self.shape.is_none() || self.distribution == Family::Weibull,
            "scale/shape only apply to Weibull"
        );
        ensure!(
            median.is_none() && self.sigma.is_none() || self.distribution == Family::LogNormal,
            "median/sigma only apply to log_normal"
        );
        ensure!(
            values.is_none() && self.weights.is_none()
                || self.distribution == Family::WeightedDiscrete,
            "values/weights only apply to weighted_discrete"
        );
        match self.distribution {
            Family::Uniform => (),
            Family::Exponential => {
                ensure!(
                    l < h && positive(mean),
                    "exponential requires positive mean and unequal bounds"
                );
                self.hazard_bounds(unit)?;
            }
            Family::Weibull => {
                ensure!(
                    l < h && positive(scale) && positive(self.shape),
                    "Weibull requires positive scale/shape and unequal bounds"
                );
                self.hazard_bounds(unit)?;
            }
            Family::LogNormal => ensure!(
                l < h && positive(median) && positive(self.sigma),
                "log_normal requires positive median/sigma and unequal bounds"
            ),
            Family::WeightedDiscrete => {
                let values = values.ok_or_else(|| anyhow::anyhow!("discrete values required"))?;
                let weights = self
                    .weights
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("discrete weights required"))?;
                ensure!(
                    (1..=32).contains(&values.len()) && values.len() == weights.len(),
                    "discrete distribution requires 1..32 matching values/weights"
                );
                let mut sum = 0u64;
                for (i, (&value, &weight)) in values.iter().zip(weights).enumerate() {
                    ensure!(
                        (l..=h).contains(&value) && !values[..i].contains(&value) && weight > 0,
                        "invalid discrete value/weight"
                    );
                    sum = sum
                        .checked_add(weight)
                        .ok_or_else(|| anyhow::anyhow!("discrete weight sum overflow"))?;
                }
            }
        }
        Ok(())
    }
    fn hazard_bounds(&self, unit: Unit) -> Result<(f64, f64, f64, f64)> {
        let (l, h) = self.bounds(unit)?;
        let (_, _, mean, scale, _, _) = self.fields(unit);
        let (scale, shape) = if self.distribution == Family::Exponential {
            (mean.unwrap(), 1.0)
        } else {
            (scale.unwrap(), self.shape.unwrap())
        };
        let a = (l as f64 / scale).powf(shape);
        let b = ((h + 1) as f64 / scale).powf(shape);
        ensure!(
            a.is_finite() && b.is_finite() && b > a,
            "numerically unsupported distribution interval"
        );
        Ok((a, b, scale, shape))
    }
    /// Call validation before sampling; failures never clamp or silently change families.
    pub fn sample(&self, unit: Unit, rng: &mut impl Rng) -> Result<u64> {
        self.validate(unit, true)?;
        let (l, h) = self.bounds(unit)?;
        let (_, _, _, _, median, values) = self.fields(unit);
        let value = match self.distribution {
            Family::Uniform => return Ok(rng.random_range(l..=h)),
            Family::WeightedDiscrete => {
                let weights = self.weights.as_ref().unwrap();
                let mut ticket = rng.random_range(0..weights.iter().sum::<u64>());
                for (&value, &weight) in values.unwrap().iter().zip(weights) {
                    if ticket < weight {
                        return Ok(value);
                    }
                    ticket -= weight;
                }
                unreachable!("validated integer weight total")
            }
            Family::Exponential | Family::Weibull => {
                let (a, b, scale, shape) = self.hazard_bounds(unit)?;
                let u: f64 = rng.sample(Open01);
                let t = a - (u * (-(b - a)).exp_m1()).ln_1p();
                scale * t.powf(1.0 / shape)
            }
            Family::LogNormal => {
                // Box–Muller provides a closed-form standard normal without an extra crate.
                for _ in 0..128 {
                    let u: f64 = rng.sample(Open01);
                    let v: f64 = rng.sample(Open01);
                    let z = (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos();
                    let x = (median.unwrap().ln() + self.sigma.unwrap() * z).exp();
                    if x >= l as f64 && x < (h + 1) as f64 {
                        return Ok(x.floor() as u64);
                    }
                }
                bail!("sampling_exhausted: log_normal exceeded 128 attempts")
            }
        };
        ensure!(
            value.is_finite() && value >= l as f64 && value < (h + 1) as f64,
            "sampling_precision: value outside conditional interval"
        );
        Ok(value.floor() as u64)
    }
}
