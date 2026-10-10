// SPDX-License-Identifier: MIT OR Apache-2.0
pub mod bpl;
pub mod charge;
use crate::{http, malformed};
use formatkit_core::Result;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Services {
    pub schema: String,
    pub provenance: Provenance,
    pub charge_hosts: Vec<String>,
    pub bpl_hosts: Vec<String>,
    pub http_aliases: Vec<[String; 2]>,
    pub billing_url: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub server: String,
}
impl Services {
    pub fn bundled() -> Result<Self> {
        Self::from_json(include_str!("../../data/services.json"))
    }
    pub fn from_json(text: &str) -> Result<Self> {
        if text.len() > 16384 {
            return Err(malformed("oversized services definition"));
        }
        let value: Self = serde_json::from_str(text).map_err(|e| malformed(&e.to_string()))?;
        value.validate()?;
        Ok(value)
    }
    pub fn hosts(&self) -> impl Iterator<Item = &str> {
        self.charge_hosts
            .iter()
            .chain(&self.bpl_hosts)
            .map(String::as_str)
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema != "swankit-wondergate-services/1"
            || self.provenance.server != "new"
            || self.charge_hosts.is_empty()
            || self.charge_hosts.len() > 128
            || self.bpl_hosts.len() > 128
            || self.http_aliases.len() > 128
        {
            return Err(malformed("invalid services definition"));
        }
        let mut seen = std::collections::BTreeSet::new();
        for host in self.hosts() {
            if http::normalize_host(host)? != host || host.len() > 253 || !seen.insert(host) {
                return Err(malformed("invalid or duplicate service host"));
            }
        }
        for pair in &self.http_aliases {
            if pair[0] == pair[1] || !pair.iter().all(|host| self.charge_hosts.contains(host)) {
                return Err(malformed("invalid HTTP alias pair"));
            }
        }
        let req = format!("POST {} HTTP/1.1\r\n\r\n", self.billing_url);
        let req = http::parse_with_aliases(req.as_bytes(), &crate::Limits::default(), &[])?;
        if !self.billing_url.is_ascii()
            || self.billing_url.contains(['\r', '\n'])
            || !self.charge_hosts.contains(&req.host)
            || req.path != "/cgi-bin/kakin.cgi"
            || !req.query.is_empty()
        {
            return Err(malformed("invalid billing URL"));
        }
        Ok(())
    }
}
