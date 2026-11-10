//! Values between the script and the host: JSON in (`JSON.parse` of the
//! engine, which no script can replace), a walk of the value out, capped,
//! that keeps plain data only (functions and symbols are left out).

use rquickjs::{Ctx, Type, Value};

/// `s` as Rust text when it is at most `max` bytes long (UTF-8), else
/// `None`. The length is read before anything is copied out of the engine:
/// the engine's own UTF-8 copy, if it needs one, counts against its memory
/// limit.
pub(super) fn text_within(s: &rquickjs::String<'_>, max: usize) -> rquickjs::Result<Option<String>> {
    let c = s.clone().to_cstring()?;
    Ok((c.len() <= max).then(|| c.as_str().to_owned()))
}

/// Depth of nested arrays and objects a script may return.
const MAX_DEPTH: usize = 32;
/// Values (each array, object, string, number...) a script may return.
const MAX_VALUES: usize = 100_000;
/// Characters of all its strings together.
const MAX_TEXT: usize = 8 * 1024 * 1024;

pub(super) fn from_json<'js>(ctx: &Ctx<'js>, value: &serde_json::Value) -> rquickjs::Result<Value<'js>> {
    ctx.json_parse(serde_json::to_string(value).unwrap_or_else(|_| "null".into()))
}

struct Budget {
    values: usize,
    text: usize,
}

impl Budget {
    /// `s` copied out, counted in the text (keys too) before it is copied.
    fn take(&mut self, s: &rquickjs::String<'_>) -> Result<String, String> {
        let left = MAX_TEXT.saturating_sub(self.text);
        let too_much = || format!("it returned more than {MAX_TEXT} bytes of text");
        let text = text_within(s, left).map_err(|e| format!("reading what it returned failed: {e}"))?.ok_or_else(too_much)?;
        self.text += text.len();
        Ok(text)
    }
}

/// `value` as JSON. `Err` when it is larger than a source may return, or a
/// getter of it throws.
pub(super) fn to_json(value: &Value<'_>) -> Result<serde_json::Value, String> {
    walk(value, 0, &mut Budget { values: 0, text: 0 }).map(|v| v.unwrap_or(serde_json::Value::Null))
}

fn walk(value: &Value<'_>, depth: usize, budget: &mut Budget) -> Result<Option<serde_json::Value>, String> {
    use serde_json::Value as J;
    budget.values += 1;
    if budget.values > MAX_VALUES {
        return Err(format!("it returned more than {MAX_VALUES} values"));
    }
    if depth > MAX_DEPTH {
        return Err(format!("it returned values nested deeper than {MAX_DEPTH}"));
    }
    let read = |e: rquickjs::Error| format!("reading what it returned failed: {e}");
    Ok(Some(match value.type_of() {
        Type::Undefined | Type::Uninitialized | Type::Null => J::Null,
        Type::Bool => J::Bool(value.as_bool().unwrap_or(false)),
        Type::Int => J::from(value.as_int().unwrap_or(0)),
        Type::Float => serde_json::Number::from_f64(value.as_float().unwrap_or(f64::NAN)).map_or(J::Null, J::Number),
        Type::String => J::String(budget.take(value.as_string().ok_or("not a string")?)?),
        Type::Array => {
            let array = value.as_array().ok_or("not an array")?;
            let mut out = Vec::with_capacity(array.len().min(1024));
            for item in array.iter::<Value>() {
                out.push(walk(&item.map_err(read)?, depth + 1, budget)?.unwrap_or(J::Null));
            }
            J::Array(out)
        }
        Type::Object | Type::Exception => {
            let object = value.as_object().ok_or("not an object")?;
            let mut out = serde_json::Map::new();
            for key in object.keys::<rquickjs::String>() {
                let key = key.map_err(read)?;
                let item: Value = object.get(key.clone()).map_err(read)?;
                if let Some(v) = walk(&item, depth + 1, budget)? {
                    out.insert(budget.take(&key)?, v);
                }
            }
            J::Object(out)
        }
        // Functions, symbols, promises, modules and big integers are not data.
        _ => return Ok(None),
    }))
}
