use monty::{DictPairs, ExcType, MontyException, MontyObject};
use num_bigint::BigInt;
use num_traits::Zero;
use serde_json::Value;

pub fn json_to_monty(value: &Value) -> MontyObject {
    match value {
        Value::Null => MontyObject::None,
        Value::Bool(b) => MontyObject::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                MontyObject::Int(i)
            } else {
                MontyObject::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => MontyObject::String(s.clone()),
        Value::Array(arr) => MontyObject::List(arr.iter().map(json_to_monty).collect()),
        Value::Object(obj) => {
            let pairs: Vec<(MontyObject, MontyObject)> = obj
                .iter()
                .map(|(k, v)| (MontyObject::String(k.clone()), json_to_monty(v)))
                .collect();
            MontyObject::Dict(DictPairs::from(pairs))
        }
    }
}

pub fn monty_to_json(obj: &MontyObject) -> Value {
    match obj {
        MontyObject::None => Value::Null,
        MontyObject::Bool(b) => Value::Bool(*b),
        MontyObject::Int(i) => Value::from(*i),
        MontyObject::BigInt(bi) => bigint_to_json(bi),
        MontyObject::Float(f) => {
            if f.is_nan() {
                Value::String("NaN".to_string())
            } else {
                Value::from(*f)
            }
        }
        MontyObject::String(s) => Value::String(s.clone()),
        MontyObject::List(items) => Value::Array(items.iter().map(monty_to_json).collect()),
        MontyObject::Tuple(items) | MontyObject::NamedTuple { values: items, .. } => {
            Value::Array(items.iter().map(monty_to_json).collect())
        }
        MontyObject::Dict(pairs) => {
            let mut map = serde_json::Map::new();
            for (k, v) in pairs {
                let key_str = match k {
                    MontyObject::String(s) => s.clone(),
                    MontyObject::Int(i) => i.to_string(),
                    other => other.py_repr(),
                };
                map.insert(key_str, monty_to_json(v));
            }
            Value::Object(map)
        }
        MontyObject::Exception { exc_type, arg, .. } => {
            let type_str: &'static str = (*exc_type).into();
            let mut map = serde_json::Map::new();
            map.insert("type".to_string(), Value::String(type_str.to_string()));
            if let Some(msg) = arg {
                map.insert("message".to_string(), Value::String(msg.clone()));
            }
            Value::Object(map)
        }
        _ => Value::Null,
    }
}

fn bigint_to_json(bi: &BigInt) -> Value {
    if let Ok(i) = i64::try_from(bi) {
        Value::from(i)
    } else if bi.is_zero() {
        Value::from(0i64)
    } else {
        Value::String(bi.to_string())
    }
}

pub fn make_iron_exception(type_name: &str, message: &str) -> MontyException {
    let exc_type = match type_name {
        "ToolFailedError" | "ToolError" => ExcType::RuntimeError,
        "ToolDeniedError" => ExcType::RuntimeError,
        "ToolTimeoutError" => ExcType::TimeoutError,
        "ToolCancelledError" => ExcType::RuntimeError,
        _ => ExcType::RuntimeError,
    };
    MontyException::new(exc_type, Some(format!("{}: {}", type_name, message)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_json_to_monty_null() {
        assert_eq!(json_to_monty(&Value::Null), MontyObject::None);
    }

    #[test]
    fn test_json_to_monty_bool() {
        assert_eq!(json_to_monty(&json!(true)), MontyObject::Bool(true));
        assert_eq!(json_to_monty(&json!(false)), MontyObject::Bool(false));
    }

    #[test]
    fn test_json_to_monty_number() {
        assert_eq!(json_to_monty(&json!(42)), MontyObject::Int(42));
        assert_eq!(json_to_monty(&json!(2.72)), MontyObject::Float(2.72));
    }

    #[test]
    fn test_json_to_monty_string() {
        assert_eq!(
            json_to_monty(&json!("hello")),
            MontyObject::String("hello".to_string())
        );
    }

    #[test]
    fn test_json_to_monty_array() {
        let result = json_to_monty(&json!([1, 2, 3]));
        assert_eq!(
            result,
            MontyObject::List(vec![
                MontyObject::Int(1),
                MontyObject::Int(2),
                MontyObject::Int(3),
            ])
        );
    }

    #[test]
    fn test_json_to_monty_dict() {
        let result = json_to_monty(&json!({"a": 1}));
        match result {
            MontyObject::Dict(pairs) => {
                let pairs_vec: Vec<_> = pairs.into_iter().collect();
                assert_eq!(pairs_vec.len(), 1);
                assert_eq!(pairs_vec[0].0, MontyObject::String("a".to_string()));
                assert_eq!(pairs_vec[0].1, MontyObject::Int(1));
            }
            _ => panic!("expected dict"),
        }
    }

    #[test]
    fn test_monty_to_json_null() {
        assert_eq!(monty_to_json(&MontyObject::None), json!(null));
    }

    #[test]
    fn test_monty_to_json_bool() {
        assert_eq!(monty_to_json(&MontyObject::Bool(true)), json!(true));
        assert_eq!(monty_to_json(&MontyObject::Bool(false)), json!(false));
    }

    #[test]
    fn test_monty_to_json_number() {
        assert_eq!(monty_to_json(&MontyObject::Int(42)), json!(42));
    }

    #[test]
    fn test_monty_to_json_string() {
        assert_eq!(
            monty_to_json(&MontyObject::String("hello".into())),
            json!("hello")
        );
    }

    #[test]
    fn test_monty_to_json_list() {
        assert_eq!(
            monty_to_json(&MontyObject::List(vec![
                MontyObject::Int(1),
                MontyObject::Int(2),
                MontyObject::Int(3),
            ])),
            json!([1, 2, 3])
        );
    }

    #[test]
    fn test_monty_to_json_dict() {
        let dict = MontyObject::Dict(DictPairs::from(vec![(
            MontyObject::String("a".into()),
            MontyObject::Int(1),
        )]));
        assert_eq!(monty_to_json(&dict), json!({"a": 1}));
    }

    #[test]
    fn test_roundtrip() {
        let original = json!({
            "name": "test",
            "count": 42,
            "active": true,
            "items": [1, 2, 3],
            "nested": {"key": "value"},
            "nothing": null
        });
        let monty_val = json_to_monty(&original);
        let roundtripped = monty_to_json(&monty_val);
        assert_eq!(original, roundtripped);
    }
}
