use formatkit_core::{Error, Result};
use num_traits::{ToPrimitive, Zero};
use serde_json::{json, Value as Json};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};

pub(crate) const MAX_ITEMS: usize = 1_048_576;
pub(crate) fn bad(message: impl Into<String>) -> Error {
    Error::Malformed(message.into())
}
pub(crate) type Scope = Rc<RefCell<Environment>>;
#[derive(Default)]
pub(crate) struct Environment {
    pub values: HashMap<String, Value>,
    pub parent: Option<Scope>,
    pub nonlocal: Vec<String>,
}
impl Environment {
    pub fn get(scope: &Scope, name: &str) -> Option<Value> {
        let env = scope.borrow();
        env.values
            .get(name)
            .cloned()
            .or_else(|| env.parent.as_ref().and_then(|p| Self::get(p, name)))
    }
    pub fn set(scope: &Scope, name: &str, value: Value) -> Result<()> {
        if scope.borrow().nonlocal.iter().any(|n| n == name) {
            let parent = scope
                .borrow()
                .parent
                .clone()
                .ok_or_else(|| bad("missing enclosing scope"))?;
            return Self::set(&parent, name, value);
        }
        scope.borrow_mut().values.insert(name.to_owned(), value);
        Ok(())
    }
}
#[derive(Clone)]
pub(crate) struct Function {
    pub name: String,
    pub args: Json,
    pub body: Vec<Json>,
    pub scope: Weak<RefCell<Environment>>,
}
#[derive(Clone)]
pub(crate) enum Value {
    None,
    Bool(bool),
    Int(i128),
    Big(num_bigint::BigInt),
    Float(f64),
    Text(String),
    Set(Rc<RefCell<Vec<Value>>>),
    Bytes(Rc<RefCell<Vec<u8>>>),
    List(Rc<RefCell<Vec<Value>>>),
    Map(Rc<RefCell<Vec<(Value, Value)>>>),
    Function(Rc<Function>),
    Native(String),
    Bound(String, Box<Value>),
    File(String),
    Json(Json),
}
impl Value {
    pub fn bytes(v: Vec<u8>) -> Self {
        Self::Bytes(Rc::new(RefCell::new(v)))
    }
    pub fn list(v: Vec<Self>) -> Self {
        Self::List(Rc::new(RefCell::new(v)))
    }
    pub fn set(v: Vec<Self>) -> Self {
        Self::Set(Rc::new(RefCell::new(v)))
    }
    pub fn big(&self) -> Result<num_bigint::BigInt> {
        match self {
            Self::Big(v) => Ok(v.clone()),
            _ => Ok(num_bigint::BigInt::from(self.integer()?)),
        }
    }
    pub fn map(v: Vec<(Self, Self)>) -> Self {
        Self::Map(Rc::new(RefCell::new(v)))
    }
    pub fn integer(&self) -> Result<i128> {
        match self {
            Self::Int(n) => Ok(*n),
            Self::Bool(b) => Ok(i128::from(*b)),
            Self::Big(b) => b.to_i128().ok_or_else(|| bad("integer extent overflow")),
            _ => Err(bad("expected integer")),
        }
    }
    pub fn index(&self) -> Result<usize> {
        usize::try_from(self.integer()?).map_err(|_| bad("negative or overflowing extent"))
    }
    pub fn number(&self) -> Result<f64> {
        match self {
            Self::Float(n) => Ok(*n),
            Self::Big(b) => b.to_f64().ok_or_else(|| bad("numeric conversion overflow")),
            _ => Ok(self.integer()? as f64),
        }
    }
    pub fn text(&self) -> String {
        match self {
            Self::Text(s) | Self::Native(s) | Self::File(s) => s.clone(),
            Self::None => "None".into(),
            Self::Bool(b) => if *b { "True" } else { "False" }.into(),
            Self::Int(n) => n.to_string(),
            Self::Big(n) => n.to_string(),
            Self::Float(n) => n.to_string(),
            _ => self.to_json().map_or("<value>".into(), |j| j.to_string()),
        }
    }
    pub fn truth(&self) -> bool {
        match self {
            Self::None => false,
            Self::Bool(b) => *b,
            Self::Int(n) => *n != 0,
            Self::Big(n) => !n.is_zero(),
            Self::Float(n) => *n != 0.0,
            Self::Text(s) => !s.is_empty(),
            Self::List(v) | Self::Set(v) => !v.borrow().is_empty(),
            Self::Map(v) => !v.borrow().is_empty(),
            Self::Bytes(v) => !v.borrow().is_empty(),
            _ => true,
        }
    }
    pub fn len(&self) -> Result<usize> {
        Ok(match self {
            Self::Bytes(b) => b.borrow().len(),
            Self::List(l) | Self::Set(l) => l.borrow().len(),
            Self::Map(m) => m.borrow().len(),
            Self::Text(t) => t.chars().count(),
            _ => return Err(bad("length target")),
        })
    }
    pub fn sequence(&self) -> Result<Vec<Self>> {
        match self {
            Self::List(v) | Self::Set(v) => Ok(v.borrow().clone()),
            Self::Map(v) => Ok(v.borrow().iter().map(|(k, _)| k.clone()).collect()),
            Self::Bytes(v) => {
                let bytes = v.borrow();
                (0..bytes.len())
                    .map(|i| {
                        Ok(Self::Int(
                            formatkit_layout::U8Field::new(i).read(&bytes)? as i128
                        ))
                    })
                    .collect()
            }
            Self::Text(s) => Ok(s.chars().map(|c| Self::Text(c.to_string())).collect()),
            _ => Err(bad("expected sequence")),
        }
    }
    pub fn raw(&self) -> Result<Vec<u8>> {
        match self {
            Self::Bytes(v) => Ok(v.borrow().clone()),
            Self::List(v) => v
                .borrow()
                .iter()
                .map(|v| u8::try_from(v.integer()?).map_err(|_| bad("byte out of range")))
                .collect(),
            _ => Err(bad("expected bytes")),
        }
    }
    pub fn equal(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Big(_), Self::Big(_))
            | (Self::Big(_), Self::Int(_))
            | (Self::Int(_), Self::Big(_)) => self.big().ok() == other.big().ok(),
            (Self::Set(a), Self::Set(b)) => {
                let a = a.borrow();
                let b = b.borrow();
                a.len() == b.len() && a.iter().all(|a| b.iter().any(|b| a.equal(b)))
            }
            (Self::None, Self::None) => true,
            (Self::Json(a), Self::Json(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Bool(_), Self::Int(_)) | (Self::Int(_), Self::Bool(_)) => {
                self.integer().ok() == other.integer().ok()
            }
            (Self::Float(_), Self::Int(_))
            | (Self::Int(_), Self::Float(_))
            | (Self::Float(_), Self::Float(_)) => self.number().ok() == other.number().ok(),
            (Self::Text(a), Self::Text(b)) => a == b,
            (Self::Bytes(a), Self::Bytes(b)) => *a.borrow() == *b.borrow(),
            (Self::List(a), Self::List(b)) => {
                let a = a.borrow();
                let b = b.borrow();
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| a.equal(b))
            }
            (Self::Map(_), Self::Map(_)) => self.to_json().ok() == other.to_json().ok(),
            _ => false,
        }
    }
    pub fn to_json(&self) -> Result<Json> {
        Ok(match self {
            Self::None => Json::Null,
            Self::Bool(v) => json!(v),
            Self::Int(v) => {
                if let Ok(n) = i64::try_from(*v) {
                    json!(n)
                } else if let Ok(n) = u64::try_from(*v) {
                    json!(n)
                } else {
                    return Err(bad("JSON integer overflow"));
                }
            }
            Self::Big(b) => {
                if let Some(n) = b.to_i64() {
                    json!(n)
                } else if let Some(n) = b.to_u64() {
                    json!(n)
                } else {
                    return Err(bad("JSON big integer overflow"));
                }
            }
            Self::Float(v) => json!(v),
            Self::Text(v) => json!(v),
            Self::Json(v) => v.clone(),
            Self::List(v) => Json::Array(
                v.borrow()
                    .iter()
                    .map(Self::to_json)
                    .collect::<Result<_>>()?,
            ),
            Self::Map(v) => {
                let mut out = serde_json::Map::new();
                for (k, v) in v.borrow().iter() {
                    out.insert(k.text(), v.to_json()?);
                }
                Json::Object(out)
            }
            _ => return Err(bad("non-JSON save value")),
        })
    }
}
pub(crate) fn tag(n: &Json) -> &str {
    n["t"].as_str().unwrap_or("")
}
pub(crate) fn string(n: &Json) -> Result<&str> {
    n.as_str().ok_or_else(|| bad("invalid generated string"))
}
pub(crate) fn array(n: &Json) -> Result<&Vec<Json>> {
    n.as_array().ok_or_else(|| bad("invalid generated list"))
}
