//! Reading loosely typed script values. Scripts are hand written: a number
//! stands where a string is expected (`VERSION: 26`), a single string where a
//! list is (`app: dotnet48 vcrun2022`). Values become text the way the
//! scripts were written to expect (Python's `str()`): `26` -> `"26"`,
//! `true` -> `"True"`, null -> `""`.

use serde_json::{Map, Value};

/// The text of a scalar; `None` for lists and maps.
pub fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(true) => Some("True".into()),
        Value::Bool(false) => Some("False".into()),
        Value::Null => Some(String::new()),
        Value::Array(_) | Value::Object(_) => None,
    }
}

/// `scalar` of `map[key]`, `None` when missing or not a scalar.
pub fn get_str(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key).and_then(scalar)
}

/// `get_str` that treats an empty value as missing.
pub fn get_nonempty(map: &Map<String, Value>, key: &str) -> Option<String> {
    get_str(map, key).filter(|s| !s.trim().is_empty())
}

/// A switch: YAML booleans, and the strings and numbers people write for them.
pub fn truthy(v: Option<&Value>) -> Option<bool> {
    match v? {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => Some(n.as_f64().is_some_and(|x| x != 0.0)),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Some(true),
            "false" | "no" | "off" | "0" | "" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// `key: value` pairs of a map with scalar values (environment variables,
/// DLL overrides), in the script's order. Other values are skipped.
pub fn pairs(v: Option<&Value>) -> Vec<(String, String)> {
    match v {
        Some(Value::Object(m)) => m.iter().filter_map(|(k, v)| scalar(v).map(|s| (k.clone(), s))).collect(),
        _ => Vec::new(),
    }
}

/// The value as a list of strings: a list of scalars, or one string split
/// on whitespace.
pub fn words(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(items)) => items.iter().filter_map(scalar).filter(|s| !s.trim().is_empty()).collect(),
        Some(v) => scalar(v).map(|s| s.split_whitespace().map(str::to_string).collect()).unwrap_or_default(),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scalars_read_like_the_scripts_expect() {
        assert_eq!(scalar(&json!(26)).as_deref(), Some("26"));
        assert_eq!(scalar(&json!(1.5)).as_deref(), Some("1.5"));
        assert_eq!(scalar(&json!(true)).as_deref(), Some("True"));
        assert_eq!(scalar(&json!(null)).as_deref(), Some(""));
        assert_eq!(scalar(&json!([1])), None);
    }

    #[test]
    fn switches_and_lists() {
        assert_eq!(truthy(Some(&json!("yes"))), Some(true));
        assert_eq!(truthy(Some(&json!(0))), Some(false));
        assert_eq!(truthy(Some(&json!("maybe"))), None);
        assert_eq!(truthy(None), None);
        assert_eq!(words(Some(&json!("vcrun2022  unifont win10"))), ["vcrun2022", "unifont", "win10"]);
        assert_eq!(words(Some(&json!(["a", 2]))), ["a", "2"]);
        assert_eq!(pairs(Some(&json!({"A": 0, "B": "x", "C": [1]}))), [("A".into(), "0".into()), ("B".into(), "x".into())]);
    }
}
