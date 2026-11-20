//! Decisions sent on to a SIEM (AUD-7), as each batch is written to the log.
//!
//! Configured per FlowR, from the environment (secrets from a secret store):
//! - `FLOWR_AUDIT_OTLP_URL` — an OpenTelemetry collector's OTLP/HTTP logs
//!   endpoint (`…/v1/logs`); each decision is a log record whose body is the
//!   decision and whose attributes are its fields. A collector forwards to
//!   Splunk, Sentinel, Datadog, S3 or anything else it exports to.
//! - `FLOWR_AUDIT_SYSLOG` — `udp://host:514` or `tcp://host:514`: one RFC 5424
//!   line per decision, its message the decision as JSON.
//! - `FLOWR_AUDIT_HTTP_URL` (+ `FLOWR_AUDIT_HTTP_AUTH`, the Authorization
//!   header) — newline-delimited JSON, e.g. Splunk HEC or a Sentinel ingestion
//!   endpoint.
//!
//! Export never holds up the log: it runs beside it, and a sink that fails is
//! retried once and then logged.

use std::sync::OnceLock;
use std::time::Duration;

use flowr_policy::audit::AuditRecord;
use serde_json::{json, Value};
use tracing::warn;

struct Sinks {
    otlp: Option<String>,
    syslog: Option<String>,
    http: Option<(String, Option<String>)>,
    client: reqwest::Client,
}

fn sinks() -> &'static Sinks {
    static S: OnceLock<Sinks> = OnceLock::new();
    S.get_or_init(|| {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Sinks {
            otlp: env("FLOWR_AUDIT_OTLP_URL"),
            syslog: env("FLOWR_AUDIT_SYSLOG"),
            http: env("FLOWR_AUDIT_HTTP_URL").map(|u| (u, env("FLOWR_AUDIT_HTTP_AUTH"))),
            client: reqwest::Client::builder().timeout(Duration::from_secs(10)).build().unwrap_or_default(),
        }
    })
}

/// Whether any sink is configured.
pub fn enabled() -> bool {
    let s = sinks();
    s.otlp.is_some() || s.syslog.is_some() || s.http.is_some()
}

/// Where decisions go, for the console (no secrets).
pub fn describe() -> Value {
    let s = sinks();
    json!({ "otlp": s.otlp, "syslog": s.syslog, "http": s.http.as_ref().map(|(u, _)| u) })
}

fn record_json(r: &AuditRecord) -> Value {
    serde_json::to_value(r).unwrap_or(Value::Null)
}

/// An OTLP/HTTP JSON logs request for these decisions.
pub fn otlp_body(rs: &[AuditRecord]) -> Value {
    let attr = |k: &str, v: &str| json!({ "key": k, "value": { "stringValue": v } });
    let records: Vec<Value> = rs.iter().map(|r| {
        let mut attrs = vec![
            attr("a3t.decision", &r.decision), attr("a3t.action", &r.action),
            attr("a3t.principal", &r.principal_id), attr("a3t.resource", &format!("{}::{}", r.resource_type, r.resource_id)),
        ];
        for (k, v) in [("a3t.org", &r.org), ("a3t.project", &r.project), ("a3t.agent", &r.agent), ("a3t.on_behalf_of", &r.on_behalf_of), ("a3t.hash", &r.hash)] {
            if let Some(v) = v { attrs.push(attr(k, v)); }
        }
        json!({
            "timeUnixNano": (r.ts_ms as u128 * 1_000_000).to_string(),
            "severityText": if r.decision == "deny" { "WARN" } else { "INFO" },
            "severityNumber": if r.decision == "deny" { 13 } else { 9 },
            "body": { "stringValue": record_json(r).to_string() },
            "attributes": attrs,
        })
    }).collect();
    json!({ "resourceLogs": [{
        "resource": { "attributes": [{ "key": "service.name", "value": { "stringValue": "a3t-decision-log" } }] },
        "scopeLogs": [{ "scope": { "name": "a3t.audit" }, "logRecords": records }],
    }] })
}

/// One RFC 5424 line.
pub fn syslog_line(r: &AuditRecord) -> String {
    let ts = chrono::DateTime::from_timestamp_millis(r.ts_ms as i64).unwrap_or_else(chrono::Utc::now).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    // facility 13 (log audit) * 8 + severity (4 warning for a deny, 6 info)
    let pri = 13 * 8 + if r.decision == "deny" { 4 } else { 6 };
    format!("<{pri}>1 {ts} {} a3t - decision - {}", if r.pod.is_empty() { "-" } else { &r.pod }, record_json(r))
}

async fn post(client: &reqwest::Client, url: &str, auth: Option<&str>, ctype: &str, body: String) -> Result<(), String> {
    let mut req = client.post(url).header("content-type", ctype).body(body);
    if let Some(a) = auth { req = req.header("authorization", a); }
    let r = req.send().await.map_err(|e| e.to_string())?;
    if r.status().is_success() { Ok(()) } else { Err(format!("{url} answered {}", r.status())) }
}

async fn syslog(target: &str, lines: &[String]) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    if let Some(addr) = target.strip_prefix("udp://") {
        let sock = tokio::net::UdpSocket::bind("0.0.0.0:0").await.map_err(|e| e.to_string())?;
        for l in lines { sock.send_to(l.as_bytes(), addr).await.map_err(|e| e.to_string())?; }
        Ok(())
    } else if let Some(addr) = target.strip_prefix("tcp://") {
        let mut s = tokio::net::TcpStream::connect(addr).await.map_err(|e| e.to_string())?;
        // Octet counting (RFC 6587).
        for l in lines { s.write_all(format!("{} {l}", l.len()).as_bytes()).await.map_err(|e| e.to_string())?; }
        Ok(())
    } else {
        Err(format!("{target}: udp://host:port or tcp://host:port"))
    }
}

/// Send a written batch on, beside the log.
pub fn send(batch: Vec<AuditRecord>) {
    if batch.is_empty() || !enabled() {
        return;
    }
    tokio::spawn(async move {
        let s = sinks();
        for attempt in 0..2 {
            let mut failed = false;
            if let Some(url) = &s.otlp {
                if let Err(e) = post(&s.client, url, None, "application/json", otlp_body(&batch).to_string()).await { failed = true; if attempt == 1 { warn!(error = %e, n = batch.len(), "decision export: OTLP failed"); } }
            }
            if let Some(t) = &s.syslog {
                let lines: Vec<String> = batch.iter().map(syslog_line).collect();
                if let Err(e) = syslog(t, &lines).await { failed = true; if attempt == 1 { warn!(error = %e, "decision export: syslog failed"); } }
            }
            if let Some((url, auth)) = &s.http {
                let nd = batch.iter().map(|r| record_json(r).to_string()).collect::<Vec<_>>().join("\n");
                if let Err(e) = post(&s.client, url, auth.as_deref(), "application/x-ndjson", nd).await { failed = true; if attempt == 1 { warn!(error = %e, "decision export: HTTP failed"); } }
            }
            if !failed { break; }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        let r = AuditRecord { id: "p-1".into(), ts_ms: 1_795_000_000_000, pod: "pod".into(), principal_id: "u1".into(),
            action: "mcp:call".into(), resource_type: "Tool".into(), resource_id: "delete_flow".into(), decision: "deny".into(), ..Default::default() };
        let b = otlp_body(std::slice::from_ref(&r));
        assert_eq!(b["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]["severityText"], "WARN");
        assert!(syslog_line(&r).starts_with("<108>1 "));
    }
}
