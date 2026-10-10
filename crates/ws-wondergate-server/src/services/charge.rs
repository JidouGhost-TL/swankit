// SPDX-License-Identifier: MIT
// Ported from the private WonderGate revival server, wgserver/shared.py (MIT).
// Copyright (C) 2026 jidoughost.
use super::Services;
use crate::{
    http::{self, Request},
    store::Store,
    Clock, Rng,
};
use formatkit_core::{Error, Result};
pub fn handle(
    request: &Request,
    services: &Services,
    store: &dyn Store,
    clock: &dyn Clock,
    rng: &mut dyn Rng,
) -> Result<Option<Vec<u8>>> {
    let action = request
        .path
        .strip_prefix("/service/charge/")
        .unwrap_or(&request.path);
    if !services.charge_hosts.contains(&request.host)
        || (!matches!(
            action,
            "nph-session" | "nph-gettime" | "nph-srvcinfo" | "nph-srvccount"
        ) && request.path != "/cgi-bin/kakin.cgi")
    {
        return Ok(None);
    }
    if request.method != "POST" {
        return Ok(Some(http::empty_response(405)));
    }
    let now = clock.now_utc();
    let fields: Vec<(String, String)> = match action {
        "nph-gettime" => vec![
            ("X-SWAN-TIME-RET".into(), "0".into()),
            ("X-SWAN-TIME-NOW".into(), now.jst_digits()),
        ],
        "nph-srvccount" => vec![
            ("X-SWAN-SRVC-RET".into(), "0".into()),
            ("X-SWAN-SRVC-CNT".into(), "0".into()),
        ],
        "nph-srvcinfo" => [
            ("X-SWAN-SRVC-RET", "0"),
            ("X-SWAN-SRVC-PRC", "0"),
            ("X-SWAN-SRVC-OPEN", "20000101000000"),
            ("X-SWAN-SRVC-CLOSE", "20991231235959"),
            ("X-SWAN-SRVC-LCHRG", "0"),
            ("X-SWAN-SRVC-WAY", "0"),
            ("X-SWAN-SRVC-TIM", "0"),
            ("X-SWAN-SRVC-TRM", "0"),
            ("X-SWAN-SRVC-MTN-FROM", "00000000000000"),
            ("X-SWAN-SRVC-MTN-TO", "00000000000000"),
            ("X-SWAN-CHRG-MTN-FROM", "00000000000000"),
            ("X-SWAN-CHRG-MTN-TO", "00000000000000"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect(),
        _ => {
            let sid = request
                .headers
                .get("x-swan-cmnd-sid")
                .or_else(|| request.headers.get("x-swan-cnts-aid"))
                .map(String::as_str)
                .unwrap_or("");
            let mut entropy = [0u8; 40];
            rng.fill(&mut entropy)
                .map_err(|e| Error::Unsupported(format!("entropy: {e}")))?;
            let candidate: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
            let mut aid = store
                .aid(&request.host, sid, &candidate, now)
                .map_err(|e| Error::Unsupported(format!("store: {e}")))?;
            let prefix = if action == "nph-session" {
                "CMND"
            } else {
                "CNTS"
            };
            if prefix == "CNTS" && sid.len() == 80 && sid.bytes().all(|b| (32..127).contains(&b)) {
                aid = sid.into();
            }
            let mut fields = vec![
                (format!("X-SWAN-{prefix}-RET"), "0".into()),
                (format!("X-SWAN-{prefix}-AID"), aid),
                (format!("X-SWAN-{prefix}-WAY"), "0".into()),
                (format!("X-SWAN-{prefix}-TIM"), "0".into()),
                (format!("X-SWAN-{prefix}-TRM"), "0".into()),
            ];
            if prefix == "CMND" {
                fields.extend([
                    ("X-SWAN-CMND-BAL".into(), "0".into()),
                    ("X-SWAN-CMND-URL".into(), services.billing_url.clone()),
                ]);
            }
            fields
        }
    };
    let refs: Vec<_> = fields
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    Ok(Some(http::response(200, &refs, &[])?))
}
