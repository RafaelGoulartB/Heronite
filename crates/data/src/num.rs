//! JavaScript-compatible number: integral values serialize without `.0`,
//! like `JSON.stringify`, so files Heronite wrote round-trip byte-for-byte.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// `Math.round`: halves go toward +infinity.
pub fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct JsNum(pub f64);

impl JsNum {
    pub fn get(self) -> f64 {
        self.0
    }
}

impl From<f64> for JsNum {
    fn from(v: f64) -> Self {
        JsNum(v)
    }
}

impl From<i64> for JsNum {
    fn from(v: i64) -> Self {
        JsNum(v as f64)
    }
}

impl Serialize for JsNum {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let v = self.0;
        if v.fract() == 0.0 && v.abs() < 9_007_199_254_740_992.0 {
            s.serialize_i64(v as i64)
        } else if v.is_finite() {
            s.serialize_f64(v)
        } else {
            // JSON.stringify(NaN/Infinity) === "null"
            s.serialize_none()
        }
    }
}

impl<'de> Deserialize<'de> for JsNum {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Option::<f64>::deserialize(d)?;
        Ok(JsNum(v.unwrap_or(0.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn integral_values_have_no_fraction() {
        assert_eq!(serde_json::to_string(&JsNum(4.0)).unwrap(), "4");
        assert_eq!(serde_json::to_string(&JsNum(4.5)).unwrap(), "4.5");
        let n: JsNum = serde_json::from_str("94").unwrap();
        assert_eq!(n.get(), 94.0);
    }
}
