//! JSON that keeps its objects' key order, for files yo edits in place: its
//! own config (where order is behavior: the first repo is the default),
//! `~/.claude.json` and `settings.local.json`. serde_json's own maps sort
//! their keys unless a feature (and three more crates) is turned on.

use std::fmt;

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum OValue {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<OValue>),
    Object(Vec<(String, OValue)>),
}

impl OValue {
    pub fn object() -> OValue {
        OValue::Object(Vec::new())
    }

    pub fn parse(text: &str) -> Result<OValue> {
        serde_json::from_str(text).map_err(|e| Error::Failed(format!("not valid JSON: {e}")))
    }

    /// Two-space indents and a final newline, as Python's `json.dump(d,
    /// indent=2)` plus `"\n"` wrote it.
    pub fn to_pretty(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).unwrap_or_default();
        s.push('\n');
        s
    }

    /// On one line, as compact as JSON goes.
    pub fn to_compact(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Takes `key` out of an object, if it's there.
    pub fn remove(&mut self, key: &str) -> Option<OValue> {
        let OValue::Object(m) = self else {
            return None;
        };
        let i = m.iter().position(|(k, _)| k == key)?;
        Some(m.remove(i).1)
    }

    pub fn get(&self, key: &str) -> Option<&OValue> {
        match self {
            OValue::Object(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut OValue> {
        match self {
            OValue::Object(m) => m.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Sets `key`, in its place if it's there, else at the end. `false` when
    /// this isn't an object.
    pub fn insert(&mut self, key: &str, value: OValue) -> bool {
        let OValue::Object(m) = self else {
            return false;
        };
        match m.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => m.push((key.to_string(), value)),
        }
        true
    }

    /// The object at `key`, made (at the end) if it isn't there. `None` when
    /// this isn't an object, or `key` holds something else.
    pub fn entry_object(&mut self, key: &str) -> Option<&mut OValue> {
        if self.get(key).is_none() && !self.insert(key, OValue::object()) {
            return None;
        }
        self.get_mut(key).filter(|v| matches!(v, OValue::Object(_)))
    }

    pub fn as_array(&self) -> Option<&Vec<OValue>> {
        match self {
            OValue::Array(a) => Some(a),
            _ => None,
        }
    }
}

impl Serialize for OValue {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            OValue::Null => s.serialize_unit(),
            OValue::Bool(b) => s.serialize_bool(*b),
            OValue::Number(n) => n.serialize(s),
            OValue::String(t) => s.serialize_str(t),
            OValue::Array(a) => {
                let mut seq = s.serialize_seq(Some(a.len()))?;
                for v in a {
                    seq.serialize_element(v)?;
                }
                seq.end()
            }
            OValue::Object(m) => {
                let mut map = s.serialize_map(Some(m.len()))?;
                for (k, v) in m {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for OValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        d.deserialize_any(V)
    }
}

struct V;

impl<'de> Visitor<'de> for V {
    type Value = OValue;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JSON")
    }
    fn visit_unit<E: de::Error>(self) -> std::result::Result<OValue, E> {
        Ok(OValue::Null)
    }
    fn visit_none<E: de::Error>(self) -> std::result::Result<OValue, E> {
        Ok(OValue::Null)
    }
    fn visit_bool<E: de::Error>(self, b: bool) -> std::result::Result<OValue, E> {
        Ok(OValue::Bool(b))
    }
    fn visit_i64<E: de::Error>(self, n: i64) -> std::result::Result<OValue, E> {
        Ok(OValue::Number(n.into()))
    }
    fn visit_u64<E: de::Error>(self, n: u64) -> std::result::Result<OValue, E> {
        Ok(OValue::Number(n.into()))
    }
    fn visit_f64<E: de::Error>(self, n: f64) -> std::result::Result<OValue, E> {
        Number::from_f64(n)
            .map(OValue::Number)
            .ok_or_else(|| E::custom("not a JSON number"))
    }
    fn visit_str<E: de::Error>(self, s: &str) -> std::result::Result<OValue, E> {
        Ok(OValue::String(s.to_string()))
    }
    fn visit_string<E: de::Error>(self, s: String) -> std::result::Result<OValue, E> {
        Ok(OValue::String(s))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> std::result::Result<OValue, A::Error> {
        let mut v = Vec::new();
        while let Some(x) = a.next_element()? {
            v.push(x);
        }
        Ok(OValue::Array(v))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> std::result::Result<OValue, A::Error> {
        let mut m: Vec<(String, OValue)> = Vec::new();
        while let Some((k, v)) = a.next_entry::<String, OValue>()? {
            // A repeated key: the last one wins, in the first one's place,
            // as Python's json does.
            match m.iter_mut().find(|(x, _)| *x == k) {
                Some((_, old)) => *old = v,
                None => m.push((k, v)),
            }
        }
        Ok(OValue::Object(m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_survives_a_round_trip() {
        let text = r#"{"z": 1, "a": {"y": [true, null, 1.5, "s"], "b": {}}, "m": -2}"#;
        let mut v = OValue::parse(text).unwrap();
        assert_eq!(
            v.to_pretty(),
            "{\n  \"z\": 1,\n  \"a\": {\n    \"y\": [\n      true,\n      null,\n      1.5,\n      \"s\"\n    ],\n    \"b\": {}\n  },\n  \"m\": -2\n}\n"
        );
        v.insert("a", OValue::Bool(false));
        v.insert("new", OValue::Null);
        assert_eq!(
            serde_json::to_string(&v).unwrap(),
            r#"{"z":1,"a":false,"m":-2,"new":null}"#
        );
        assert!(v.entry_object("m").is_none());
        assert!(v.entry_object("o").is_some());
        assert!(OValue::parse("{").is_err());
        assert_eq!(
            serde_json::to_string(&OValue::parse(r#"{"k": 1, "k": 2}"#).unwrap()).unwrap(),
            r#"{"k":2}"#
        );
    }
}
