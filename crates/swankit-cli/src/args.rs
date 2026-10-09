use crate::Result;
use std::collections::{BTreeMap, BTreeSet};

pub struct Args {
    pub positional: Vec<String>,
    values: BTreeMap<String, String>,
    flags: BTreeSet<String>,
}
impl Args {
    pub fn parse(words: Vec<String>) -> Result<Self> {
        let mut out = Self {
            positional: Vec::new(),
            values: BTreeMap::new(),
            flags: BTreeSet::new(),
        };
        let mut words = words.into_iter();
        while let Some(word) = words.next() {
            if word == "--" {
                out.positional.extend(words);
                break;
            }
            if matches!(
                word.as_str(),
                "--json" | "--force" | "--snapshot" | "--color"
            ) {
                if !out.flags.insert(word.clone()) {
                    return Err(format!("duplicate option: {word}").into());
                }
            } else if word.starts_with("--") {
                let value = words
                    .next()
                    .ok_or_else(|| format!("missing value for {word}"))?;
                if value.starts_with("--") {
                    return Err(format!("missing value for {word}").into());
                }
                if out.values.insert(word.clone(), value).is_some() {
                    return Err(format!("duplicate option: {word}").into());
                }
            } else {
                out.positional.push(word);
            }
        }
        Ok(out)
    }
    pub fn flag(&mut self, key: &str) -> bool {
        self.flags.remove(key)
    }
    pub fn value(&mut self, key: &str) -> Option<String> {
        self.values.remove(key)
    }
    pub fn require(&mut self, key: &str) -> Result<String> {
        self.value(key)
            .ok_or_else(|| format!("required option: {key}").into())
    }
    pub fn number(&mut self, key: &str, default: usize) -> Result<usize> {
        self.value(key).map_or(Ok(default), |s| number(&s))
    }
    pub fn finish(&self, count: usize) -> Result<()> {
        if self.positional.len() != count {
            return Err("wrong number of arguments; see swankit --help".into());
        }
        if let Some(key) = self
            .values
            .keys()
            .next()
            .or_else(|| self.flags.iter().next())
        {
            return Err(format!("unknown option for this command: {key}").into());
        }
        Ok(())
    }
}
pub fn number(s: &str) -> Result<usize> {
    if let Some(hex) = s.strip_prefix("0x") {
        usize::from_str_radix(hex, 16).map_err(|_| format!("invalid integer: {s}").into())
    } else {
        s.parse()
            .map_err(|_| format!("invalid integer: {s}").into())
    }
}
