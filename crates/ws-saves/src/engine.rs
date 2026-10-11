//! Bounded execution of generated save semantics; no host I/O or dynamic code.
use crate::value::*;
use formatkit_core::{bytes_at, Result};
use formatkit_layout::{patch_bytes, U8Field};
use serde_json::Value as Json;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

pub(crate) struct Engine {
    pub files: HashMap<String, Vec<u8>>,
    pub context: HashMap<String, Vec<u8>>,
    pub checksums: HashMap<String, Json>,
    pub primitives: HashMap<String, Json>,
    pub output: Vec<Json>,
    pub fuel: usize,
    pub depth: usize,
}
#[derive(Clone)]
pub(crate) enum Flow {
    Next,
    Return(Value),
    Break,
    Continue,
}
impl Engine {
    pub fn new(bytes: &[u8], name: &str) -> Result<Self> {
        if bytes.len() > MAX_ITEMS {
            return Err(bad("save exceeds media limit"));
        }
        Ok(Self {
            files: HashMap::from([(name.to_owned(), bytes.to_vec())]),
            context: HashMap::new(),
            checksums: HashMap::new(),
            primitives: HashMap::new(),
            output: vec![],
            fuel: 50_000_000,
            depth: 0,
        })
    }
    fn tick(&mut self) -> Result<()> {
        self.fuel = self
            .fuel
            .checked_sub(1)
            .ok_or_else(|| bad("save semantic operation limit"))?;
        Ok(())
    }
    pub fn initialize(&mut self, program: &Json) -> Result<Scope> {
        let scope = Rc::new(RefCell::new(Environment::default()));
        Environment::set(&scope, "__doc__", Value::Text("save grammar".into()))?;
        self.block(array(&program["body"])?, &scope)?;
        Ok(scope)
    }
    pub fn block(&mut self, body: &[Json], scope: &Scope) -> Result<Flow> {
        for statement in body {
            let flow = self.statement(statement, scope)?;
            if !matches!(flow, Flow::Next) {
                return Ok(flow);
            }
        }
        Ok(Flow::Next)
    }
    pub fn statement(&mut self, n: &Json, s: &Scope) -> Result<Flow> {
        self.tick()?;
        match tag(n) {
            "Assign" => {
                let v = self.eval(&n["value"], s)?;
                for t in array(&n["targets"])? {
                    self.assign(t, v.clone(), s)?;
                }
            }
            "AnnAssign" => {
                if !n["value"].is_null() {
                    let v = self.eval(&n["value"], s)?;
                    self.assign(&n["target"], v, s)?;
                }
            }
            "AugAssign" => {
                let a = self.eval(&n["target"], s)?;
                let b = self.eval(&n["value"], s)?;
                let v = if tag(&n["op"]) == "Add" && matches!(a, Value::List(_) | Value::Bytes(_)) {
                    self.method("extend", a.clone(), vec![b], HashMap::new())?;
                    a
                } else {
                    self.binary(tag(&n["op"]), a, b)?
                };
                self.assign(&n["target"], v, s)?;
            }
            "Expr" => {
                self.eval(&n["value"], s)?;
            }
            "Return" => {
                return Ok(Flow::Return(if n["value"].is_null() {
                    Value::None
                } else {
                    self.eval(&n["value"], s)?
                }))
            }
            "If" => {
                let test = self.eval(&n["test"], s)?.truth();
                return self.block(array(&n[if test { "body" } else { "orelse" }])?, s);
            }
            "For" => {
                let iterable = self.eval(&n["iter"], s)?;
                let seq = iterable.sequence()?;
                for v in seq {
                    self.tick()?;
                    self.assign(&n["target"], v, s)?;
                    match self.block(array(&n["body"])?, s)? {
                        Flow::Break => return Ok(Flow::Next),
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                }
                return self.block(array(&n["orelse"])?, s);
            }
            "While" => {
                while self.eval(&n["test"], s)?.truth() {
                    self.tick()?;
                    match self.block(array(&n["body"])?, s)? {
                        Flow::Break => return Ok(Flow::Next),
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                }
                return self.block(array(&n["orelse"])?, s);
            }
            "Break" => return Ok(Flow::Break),
            "Continue" => return Ok(Flow::Continue),
            "FunctionDef" => {
                let name = string(&n["name"])?;
                let f = Function {
                    name: name.into(),
                    args: n["args"].clone(),
                    body: array(&n["body"])?.clone(),
                    scope: Rc::downgrade(s),
                };
                Environment::set(s, name, Value::Function(Rc::new(f)))?;
            }
            "Nonlocal" => {
                for name in array(&n["names"])? {
                    s.borrow_mut().nonlocal.push(string(name)?.into());
                }
            }
            "Raise" => {
                return Err(bad(if n["exc"].is_null() {
                    "save rejected".into()
                } else {
                    self.eval(&n["exc"], s)?.text()
                }));
            }
            "Assert" => {
                if !self.eval(&n["test"], s)?.truth() {
                    return Err(bad("save assertion failed"));
                }
            }
            "With" => {
                for item in array(&n["items"])? {
                    let v = self.eval(&item["context_expr"], s)?;
                    if !item["optional_vars"].is_null() {
                        self.assign(&item["optional_vars"], v, s)?;
                    }
                }
                return self.block(array(&n["body"])?, s);
            }
            "Try" => {
                let result = self.block(array(&n["body"])?, s);
                let flow = match result {
                    Ok(flow) => {
                        if matches!(flow, Flow::Next) {
                            self.block(array(&n["orelse"])?, s)?
                        } else {
                            flow
                        }
                    }
                    Err(e) => {
                        if let Some(handler) = array(&n["handlers"])?.first() {
                            if let Some(name) = handler["name"].as_str() {
                                Environment::set(s, name, Value::Text(e.to_string()))?;
                            }
                            self.block(array(&handler["body"])?, s)?
                        } else {
                            return Err(e);
                        }
                    }
                };
                let final_flow = self.block(array(&n["finalbody"])?, s)?;
                return Ok(if matches!(final_flow, Flow::Next) {
                    flow
                } else {
                    final_flow
                });
            }
            "Delete" => {
                for target in array(&n["targets"])? {
                    if tag(target) == "Name" {
                        s.borrow_mut().values.remove(string(&target["id"])?);
                        continue;
                    }
                    if tag(target) != "Subscript" {
                        return Err(bad("unsupported save deletion"));
                    }
                    let obj = self.eval(&target["value"], s)?;
                    let key = self.eval(&target["slice"], s)?;
                    self.remove(&obj, &key)?;
                }
            }
            "Pass" | "Import" | "ImportFrom" => {}
            other => return Err(bad(format!("unsupported save statement {other}"))),
        }
        Ok(Flow::Next)
    }
    pub fn eval(&mut self, n: &Json, s: &Scope) -> Result<Value> {
        self.tick()?;
        match tag(n) {
            "Constant" => Ok(match &n["value"] {
                Json::Null => Value::None,
                Json::Bool(b) => Value::Bool(*b),
                Json::String(v) => Value::Text(v.clone()),
                Json::Number(v) => {
                    if let Some(i) = v.as_i64() {
                        Value::Int(i.into())
                    } else if let Some(i) = v.as_u64() {
                        Value::Int(i.into())
                    } else {
                        Value::Float(v.as_f64().ok_or_else(|| bad("constant number"))?)
                    }
                }
                _ => return Err(bad("constant kind")),
            }),
            "Bytes" => Ok(Value::bytes(
                array(&n["v"])?
                    .iter()
                    .map(|b| {
                        b.as_u64()
                            .and_then(|b| u8::try_from(b).ok())
                            .ok_or_else(|| bad("constant byte"))
                    })
                    .collect::<Result<_>>()?,
            )),
            "Name" => {
                let name = string(&n["id"])?;
                Ok(Environment::get(s, name).unwrap_or_else(|| Value::Native(name.into())))
            }
            "List" | "Tuple" => Ok(Value::list(
                array(&n["elts"])?
                    .iter()
                    .map(|e| self.eval(e, s))
                    .collect::<Result<_>>()?,
            )),
            "Set" => {
                let mut values = vec![];
                for n in array(&n["elts"])? {
                    let v = self.eval(n, s)?;

                    if Value::find_equal(&values, &v)?.is_none() {
                        values.push(v);
                    }
                }
                Ok(Value::set(values))
            }
            "Dict" => {
                let mut out = vec![];
                for (k, v) in array(&n["keys"])?.iter().zip(array(&n["values"])?) {
                    let v = self.eval(v, s)?;
                    if k.is_null() {
                        if let Value::Map(m) = v {
                            for (key, value) in m.borrow().iter() {
                                Self::dict_insert(&mut out, key.clone(), value.clone())?;
                            }
                        } else {
                            return Err(bad("dictionary spread"));
                        }
                    } else {
                        let key = self.eval(k, s)?;
                        Self::dict_insert(&mut out, key, v)?;
                    }
                }
                Ok(Value::map(out))
            }
            "Subscript" => {
                let obj = self.eval(&n["value"], s)?;
                if tag(&n["slice"]) == "Slice" {
                    self.slice(&obj, &n["slice"], s)
                } else {
                    let key = self.eval(&n["slice"], s)?;
                    Self::get(&obj, &key)
                }
            }
            "Attribute" => {
                let obj = self.eval(&n["value"], s)?;
                let name = string(&n["attr"])?;
                Ok(match obj {
                    Value::Native(module) if module == "sys" && name == "argv" => {
                        Environment::get(s, "sys_argv")
                            .ok_or_else(|| bad("missing save argument vector"))?
                    }
                    Value::Native(module) => Value::Native(format!("{module}.{name}")),
                    v => Value::Bound(name.into(), Box::new(v)),
                })
            }
            "Call" => {
                let fun = self.eval(&n["func"], s)?;
                let mut args = vec![];
                for a in array(&n["args"])? {
                    if tag(a) == "Starred" {
                        let iterable = self.eval(&a["value"], s)?;
                        args.extend(iterable.sequence()?);
                    } else {
                        args.push(self.eval(a, s)?);
                    }
                }
                let mut kwargs = HashMap::new();
                for kw in array(&n["keywords"])? {
                    kwargs.insert(string(&kw["arg"])?.into(), self.eval(&kw["value"], s)?);
                }
                self.call(fun, args, kwargs)
            }
            "BinOp" => {
                let a = self.eval(&n["left"], s)?;
                let b = self.eval(&n["right"], s)?;
                self.binary(tag(&n["op"]), a, b)
            }
            "UnaryOp" => {
                let a = self.eval(&n["operand"], s)?;
                Ok(match tag(&n["op"]) {
                    "Not" => Value::Bool(!a.truth()),
                    "USub" => match a {
                        Value::Float(v) => Value::Float(-v),
                        _ => Value::Int(
                            a.integer()?
                                .checked_neg()
                                .ok_or_else(|| bad("integer negation overflow"))?,
                        ),
                    },
                    "Invert" => {
                        if let Value::Big(b) = a {
                            Value::Big(!b)
                        } else {
                            Value::Int(!a.integer()?)
                        }
                    }
                    _ => return Err(bad("unary operation")),
                })
            }
            "BoolOp" => {
                let and = tag(&n["op"]) == "And";
                let mut last = Value::None;
                for v in array(&n["values"])? {
                    last = self.eval(v, s)?;
                    if last.truth() != and {
                        break;
                    }
                }
                Ok(last)
            }
            "Compare" => {
                let mut left = self.eval(&n["left"], s)?;
                for (op, right) in array(&n["ops"])?.iter().zip(array(&n["comparators"])?) {
                    let right = self.eval(right, s)?;
                    if !Self::compare(tag(op), &left, &right)? {
                        return Ok(Value::Bool(false));
                    }
                    left = right;
                }
                Ok(Value::Bool(true))
            }
            "IfExp" => {
                let b = self.eval(&n["test"], s)?.truth();
                self.eval(&n[if b { "body" } else { "orelse" }], s)
            }
            "JoinedStr" => {
                let mut out = String::new();
                for item in array(&n["values"])? {
                    if tag(item) == "FormattedValue" {
                        let v = self.eval(&item["value"], s)?;
                        let spec = if item["format_spec"].is_null() {
                            String::new()
                        } else {
                            self.eval(&item["format_spec"], s)?.text()
                        };
                        out.push_str(&crate::native::format_value(&v, &spec)?);
                    } else {
                        out.push_str(&self.eval(item, s)?.text());
                    }
                }
                Ok(Value::Text(out))
            }
            "ListComp" | "GeneratorExp" | "SetComp" | "DictComp" => {
                let local = Rc::new(RefCell::new(Environment {
                    parent: Some(s.clone()),
                    ..Environment::default()
                }));
                let mut out = vec![];
                self.comprehension(n, array(&n["generators"])?, 0, &local, &mut out)?;
                if tag(n) == "DictComp" {
                    let mut pairs = vec![];
                    for p in out {
                        let p = p.sequence()?;
                        pairs.push((p[0].clone(), p[1].clone()));
                    }
                    Ok(Value::map(pairs))
                } else {
                    if tag(n) == "SetComp" {
                        let mut unique = vec![];
                        for v in out {
                            if Value::find_equal(&unique, &v)?.is_none() {
                                unique.push(v);
                            }
                        }
                        out = unique;
                        return Ok(Value::set(out));
                    }
                    Ok(Value::list(out))
                }
            }
            "Lambda" => Ok(Value::Function(Rc::new(Function {
                name: "<lambda>".into(),
                args: n["args"].clone(),
                body: vec![serde_json::json!({"t":"Return","value":n["body"]})],
                scope: Rc::downgrade(s),
            }))),
            "NamedExpr" => {
                let v = self.eval(&n["value"], s)?;
                self.assign(&n["target"], v.clone(), s)?;
                Ok(v)
            }
            other => Err(bad(format!("unsupported save expression {other}"))),
        }
    }
    fn comprehension(
        &mut self,
        n: &Json,
        gens: &[Json],
        i: usize,
        s: &Scope,
        out: &mut Vec<Value>,
    ) -> Result<()> {
        if i == gens.len() {
            if out.len() >= MAX_ITEMS {
                return Err(bad("save list limit"));
            }
            if tag(n) == "DictComp" {
                let k = self.eval(&n["key"], s)?;
                let v = self.eval(&n["value"], s)?;
                out.push(Value::list(vec![k, v]));
            } else {
                out.push(self.eval(&n["elt"], s)?);
            }
            return Ok(());
        }
        let g = &gens[i];
        let iterable = self.eval(&g["iter"], s)?;
        let seq = iterable.sequence()?;
        for v in seq {
            self.assign(&g["target"], v, s)?;
            let mut accept = true;
            for test in array(&g["ifs"])? {
                if !self.eval(test, s)?.truth() {
                    accept = false;
                    break;
                }
            }
            if accept {
                self.comprehension(n, gens, i + 1, s, out)?;
            }
        }
        Ok(())
    }
    pub fn call(
        &mut self,
        fun: Value,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    ) -> Result<Value> {
        self.tick()?;
        match fun {
            Value::Native(name) => self.native(&name, args, kwargs),
            Value::Bound(name, obj) => self.method(&name, *obj, args, kwargs),
            Value::Function(f) => {
                if let Some(result) = self.checksum_native(&f.name, &args) {
                    return result;
                }
                if self.depth >= 16 {
                    return Err(bad("save call depth"));
                }
                let parent = f
                    .scope
                    .upgrade()
                    .ok_or_else(|| bad("expired save closure"))?;
                let scope = Rc::new(RefCell::new(Environment {
                    parent: Some(parent.clone()),
                    ..Environment::default()
                }));
                let params = array(&f.args["args"])?;
                let defaults = array(&f.args["defaults"])?;
                if defaults.len() > params.len() {
                    return Err(bad("function defaults exceed parameters"));
                }
                if args.len() > params.len() {
                    return Err(bad(format!("too many arguments for {}", f.name)));
                }
                for (i, p) in params.iter().enumerate() {
                    let name = string(&p["arg"])?;
                    let v = if let Some(v) = args.get(i) {
                        v.clone()
                    } else if let Some(v) = kwargs.get(name) {
                        v.clone()
                    } else if let Some(d) = i
                        .checked_sub(params.len() - defaults.len())
                        .and_then(|j| defaults.get(j))
                    {
                        self.eval(d, &parent)?
                    } else {
                        return Err(bad(format!("missing argument {name} for {}", f.name)));
                    };
                    Environment::set(&scope, name, v)?;
                }
                self.depth += 1;
                let result = self.block(&f.body, &scope);
                self.depth -= 1;
                result
                    .map(|flow| match flow {
                        Flow::Return(v) => v,
                        _ => Value::None,
                    })
                    .map_err(|e| bad(format!("{}: {e}", f.name)))
            }
            _ => Err(bad("save value is not callable")),
        }
    }
    fn assign(&mut self, t: &Json, v: Value, s: &Scope) -> Result<()> {
        match tag(t) {
            "Name" => Environment::set(s, string(&t["id"])?, v),
            "Tuple" | "List" => {
                let values = v.sequence()?;
                let targets = array(&t["elts"])?;
                if targets.len() != values.len() {
                    return Err(bad("unpack length"));
                }
                for (t, v) in targets.iter().zip(values) {
                    self.assign(t, v, s)?;
                }
                Ok(())
            }
            "Subscript" => {
                let obj = self.eval(&t["value"], s)?;
                if tag(&t["slice"]) == "Slice" {
                    let (start, end, step) = self.slice_bounds(&t["slice"], obj.len()?, s)?;
                    if step != 1 {
                        return Err(bad("strided assignment"));
                    }
                    match obj {
                        Value::Bytes(b) => {
                            let data = v.raw()?;
                            let mut b = b.borrow_mut();
                            if data.len() != end.saturating_sub(start) {
                                return Err(bad("save edits must preserve extent"));
                            }
                            patch_bytes(&mut b, start, &data)
                        }
                        Value::List(l) => {
                            let seq = v.sequence()?;
                            let mut l = l.borrow_mut();
                            if l.len() - end.saturating_sub(start) + seq.len() > MAX_ITEMS {
                                return Err(bad("list assignment limit"));
                            }
                            l.splice(start..end, seq);
                            Ok(())
                        }
                        _ => Err(bad("slice assignment target")),
                    }
                } else {
                    let k = self.eval(&t["slice"], s)?;
                    Self::set(&obj, k, v)
                }
            }
            _ => Err(bad("assignment target")),
        }
    }
    pub fn get(obj: &Value, key: &Value) -> Result<Value> {
        match obj {
            Value::Map(m) => {
                let m = m.borrow();
                Value::map_index(&m, key)?
                    .map(|index| m[index].1.clone())
                    .ok_or_else(|| bad(format!("missing key {}", key.text())))
            }
            Value::Bytes(b) => {
                let b = b.borrow();
                let i = Self::normalize_index(key.integer()?, b.len())?;
                Ok(Value::Int(U8Field::new(i).read(&b)? as i128))
            }
            Value::List(l) => {
                let l = l.borrow();
                let i = Self::normalize_index(key.integer()?, l.len())?;
                l.get(i).cloned().ok_or_else(|| bad("list index"))
            }
            Value::Text(text) => {
                let chars: Vec<_> = text.chars().collect();
                let i = Self::normalize_index(key.integer()?, chars.len())?;
                Ok(Value::Text(
                    chars.get(i).ok_or_else(|| bad("text index"))?.to_string(),
                ))
            }
            _ => Err(bad("subscript target")),
        }
    }
    pub fn set(obj: &Value, key: Value, v: Value) -> Result<()> {
        match obj {
            Value::Map(m) => {
                let mut m = m.borrow_mut();
                if let Some(index) = Value::map_index(&m, &key)? {
                    m[index].1 = v;
                } else {
                    if m.len() >= MAX_ITEMS {
                        return Err(bad("map limit"));
                    }
                    m.push((key, v));
                }
                Ok(())
            }
            Value::Bytes(b) => {
                let mut b = b.borrow_mut();
                let i = Self::normalize_index(key.integer()?, b.len())?;
                let byte = u8::try_from(v.integer()?).map_err(|_| bad("byte write range"))?;
                U8Field::new(i).write(&mut b, byte)
            }
            Value::List(l) => {
                let mut l = l.borrow_mut();
                let i = Self::normalize_index(key.integer()?, l.len())?;
                *l.get_mut(i).ok_or_else(|| bad("list write index"))? = v;
                Ok(())
            }
            _ => Err(bad("write target")),
        }
    }
    pub fn remove(&mut self, obj: &Value, key: &Value) -> Result<Value> {
        match obj {
            Value::Map(m) => {
                let mut m = m.borrow_mut();
                let i = Value::map_index(&m, key)?.ok_or_else(|| bad("map remove key"))?;
                Ok(m.remove(i).1)
            }
            Value::List(l) => {
                let mut l = l.borrow_mut();
                let i = Self::normalize_index(key.integer()?, l.len())?;
                Ok(l.remove(i))
            }
            _ => Err(bad("remove target")),
        }
    }
    fn normalize_index(i: i128, len: usize) -> Result<usize> {
        let i = if i < 0 { len as i128 + i } else { i };
        usize::try_from(i)
            .ok()
            .filter(|i| *i < len)
            .ok_or_else(|| bad("index outside save value"))
    }
    fn slice_bounds(&mut self, n: &Json, len: usize, s: &Scope) -> Result<(usize, usize, i128)> {
        let step = if n["step"].is_null() {
            1
        } else {
            self.eval(&n["step"], s)?.integer()?
        };
        if step == 0 {
            return Err(bad("zero slice step"));
        }
        let mut bound = |e: &Json, default: usize| -> Result<usize> {
            if e.is_null() {
                Ok(default)
            } else {
                let i = self.eval(e, s)?.integer()?;
                let i = if i < 0 { len as i128 + i } else { i };
                Ok(i.clamp(0, len as i128) as usize)
            }
        };
        let start = bound(&n["lower"], 0)?;
        let end = bound(&n["upper"], len)?;
        Ok((start, end.max(start), step))
    }
    fn slice(&mut self, obj: &Value, n: &Json, s: &Scope) -> Result<Value> {
        let len = obj.len()?;
        let (start, end, step) = self.slice_bounds(n, len, s)?;
        if step < 0 {
            let mut seq = obj.sequence()?;
            seq.reverse();
            return Ok(Value::list(seq));
        }
        let step = usize::try_from(step).map_err(|_| bad("slice step overflow"))?;
        match obj {
            Value::Bytes(b) => {
                let b = b.borrow();
                Ok(Value::bytes(
                    bytes_at(&b, start, end - start)?
                        .iter()
                        .step_by(step)
                        .copied()
                        .collect(),
                ))
            }
            Value::Text(t) => Ok(Value::Text(
                t.chars()
                    .skip(start)
                    .take(end - start)
                    .step_by(step)
                    .collect(),
            )),
            _ => Ok(Value::list(
                obj.sequence()?
                    .into_iter()
                    .skip(start)
                    .take(end - start)
                    .step_by(step)
                    .collect(),
            )),
        }
    }
    pub fn compare(op: &str, a: &Value, b: &Value) -> Result<bool> {
        Ok(match op {
            "Eq" | "Is" => a.equal(b)?,
            "NotEq" | "IsNot" => !a.equal(b)?,
            "In" | "NotIn" => {
                let yes = match (a, b) {
                    (Value::Text(a), Value::Text(b)) => b.contains(a),
                    _ => Value::find_equal(&b.sequence()?, a)?.is_some(),
                };
                if op == "In" {
                    yes
                } else {
                    !yes
                }
            }
            "Lt" | "LtE" | "Gt" | "GtE" => {
                let order = match (a, b) {
                    (Value::Text(a), Value::Text(b)) => a.partial_cmp(b),
                    (Value::Big(_), _) | (_, Value::Big(_)) => Some(a.big()?.cmp(&b.big()?)),
                    (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
                    (Value::List(a), Value::List(b)) => {
                        let a = a.borrow();
                        let b = b.borrow();
                        let mut order = std::cmp::Ordering::Equal;
                        for (a, b) in a.iter().zip(b.iter()) {
                            if Self::compare("Lt", a, b)? {
                                order = std::cmp::Ordering::Less;
                                break;
                            }
                            if Self::compare("Gt", a, b)? {
                                order = std::cmp::Ordering::Greater;
                                break;
                            }
                        }
                        if order == std::cmp::Ordering::Equal {
                            order = a.len().cmp(&b.len());
                        }
                        Some(order)
                    }
                    _ => a.number()?.partial_cmp(&b.number()?),
                }
                .ok_or_else(|| bad("unordered save comparison"))?;
                match op {
                    "Lt" => order.is_lt(),
                    "LtE" => !order.is_gt(),
                    "Gt" => order.is_gt(),
                    _ => !order.is_lt(),
                }
            }
            _ => return Err(bad("comparison operation")),
        })
    }
    pub fn binary(&mut self, op: &str, a: Value, b: Value) -> Result<Value> {
        if let (Value::Bool(x), Value::Bool(y)) = (&a, &b) {
            if let Some(v) = match op {
                "BitAnd" => Some(*x && *y),
                "BitOr" => Some(*x || *y),
                "BitXor" => Some(*x ^ *y),
                _ => None,
            } {
                return Ok(Value::Bool(v));
            }
        }
        if op == "BitOr" && matches!(a, Value::Set(_)) && matches!(b, Value::Set(_)) {
            let mut seq = a.sequence()?;
            for v in b.sequence()? {
                if Value::find_equal(&seq, &v)?.is_none() {
                    seq.push(v);
                    checked_limit(seq.len())?;
                }
            }
            return Ok(Value::set(seq));
        }
        if matches!(a, Value::Big(_)) || matches!(b, Value::Big(_)) {
            let x = a.big()?;
            let y = b.big()?;
            let value = match op {
                "BitAnd" => x & y,
                "BitOr" => x | y,
                "BitXor" => x ^ y,
                "Add" => x + y,
                "Sub" => x - y,
                "Mult" => x * y,
                "LShift" | "RShift" => {
                    let shift = b.index()?;
                    if shift > MAX_ITEMS * 8 {
                        return Err(bad("big integer shift limit"));
                    }
                    if op == "LShift" {
                        x << shift
                    } else {
                        x >> shift
                    }
                }
                _ => return Err(bad("big integer operation")),
            };
            if value.bits() > MAX_ITEMS as u64 * 8 {
                return Err(bad("big integer size limit"));
            }
            return Ok(Value::Big(value));
        }
        if op == "Add" {
            match (&a, &b) {
                (Value::Text(x), Value::Text(y)) => {
                    if x.len() + y.len() > MAX_ITEMS {
                        return Err(bad("text limit"));
                    }
                    return Ok(Value::Text(format!("{x}{y}")));
                }
                (Value::List(_), Value::List(_)) => {
                    let mut seq = a.sequence()?;
                    seq.extend(b.sequence()?);
                    if seq.len() > MAX_ITEMS {
                        return Err(bad("list limit"));
                    }
                    return Ok(Value::list(seq));
                }
                (Value::Bytes(_), Value::Bytes(_)) => {
                    let mut seq = a.raw()?;
                    seq.extend(b.raw()?);
                    if seq.len() > MAX_ITEMS {
                        return Err(bad("byte limit"));
                    }
                    return Ok(Value::bytes(seq));
                }
                _ => {}
            }
        }
        if op == "Mult" && matches!(a, Value::List(_) | Value::Bytes(_) | Value::Text(_)) {
            let n = b.index()?;
            let len = a.len()?;
            if len.checked_mul(n).is_none_or(|v| v > MAX_ITEMS) {
                return Err(bad("repeat limit"));
            }
            return Ok(match a {
                Value::Text(t) => Value::Text(t.repeat(n)),
                Value::Bytes(_) => Value::bytes(a.raw()?.repeat(n)),
                _ => {
                    let seq = a.sequence()?;
                    let mut out = vec![];
                    for _ in 0..n {
                        out.extend(seq.clone());
                    }
                    Value::list(out)
                }
            });
        }
        if op == "Mod" && matches!(a, Value::Text(_)) {
            return crate::native::percent_format(&a.text(), &b).map(Value::Text);
        }
        if matches!(a, Value::Float(_)) || matches!(b, Value::Float(_)) || op == "Div" {
            let (a, b) = (a.number()?, b.number()?);
            if b == 0.0 && matches!(op, "Div" | "FloorDiv" | "Mod") {
                return Err(bad("division by zero"));
            }
            return Ok(Value::Float(match op {
                "Add" => a + b,
                "Sub" => a - b,
                "Mult" => a * b,
                "Div" => a / b,
                "FloorDiv" => (a / b).floor(),
                "Mod" => a.rem_euclid(b),
                _ => return Err(bad("float operation")),
            }));
        }
        let (a, b) = (a.integer()?, b.integer()?);
        let v = match op {
            "Add" => a.checked_add(b),
            "Sub" => a.checked_sub(b),
            "Mult" => a.checked_mul(b),
            "FloorDiv" => a.checked_div_euclid(b),
            "Mod" => a.checked_rem_euclid(b),
            "BitAnd" => Some(a & b),
            "BitOr" => Some(a | b),
            "BitXor" => Some(a ^ b),
            "LShift" => u32::try_from(b).ok().filter(|n| *n < 128).and_then(|n| {
                let shifted = a.checked_shl(n)?;
                (shifted >> n == a).then_some(shifted)
            }),
            "RShift" => u32::try_from(b)
                .ok()
                .filter(|n| *n < 128)
                .and_then(|n| a.checked_shr(n)),
            _ => return Err(bad(format!("integer operation {op}"))),
        }
        .ok_or_else(|| bad("integer operation overflow or zero divisor"))?;
        Ok(Value::Int(v))
    }
    fn dict_insert(out: &mut Vec<(Value, Value)>, key: Value, value: Value) -> Result<()> {
        if let Some(index) = Value::map_index(out, &key)? {
            out[index].1 = value;
        } else {
            out.push((key, value));
        }
        Ok(())
    }
}

fn checked_limit(n: usize) -> Result<()> {
    if n > MAX_ITEMS {
        Err(bad("set limit"))
    } else {
        Ok(())
    }
}
