//! FlowR's process metrics (AO-1, `.ai/A3T_OBSERVABILITY.md`): one Prometheus registry,
//! served as text at `GET /metrics` and as JSON with percentiles at
//! `GET /admin/stats/metrics` by every FlowR process (console, gateway, runtimes).
//!
//! Labels are bounded: tool names, task types, engine ops and LLM providers come from
//! fixed sets (anything else is `other`); HTTP routes are their templates. No label or
//! log line carries a payload, a prompt or a token.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use prometheus::{Encoder, HistogramOpts, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry, TextEncoder};

/// Latency buckets, seconds: 0.5 ms to 60 s (LLM calls and runs take long).
const SECONDS: &[f64] = &[0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0];
const TOKENS: &[f64] = &[10.0, 100.0, 1_000.0, 10_000.0, 100_000.0];

pub struct Metrics {
    pub registry: Registry,
    /// HTTP API by route template and status class.
    pub requests: IntCounterVec,
    pub request_seconds: HistogramVec,
    pub requests_in_flight: IntGauge,
    pub auth_failures: IntCounterVec,
    /// MCP tool calls by tool and status.
    pub tool_calls: IntCounterVec,
    pub tool_seconds: HistogramVec,
    /// Runs by durability; finished by outcome; active now.
    pub runs_started: IntCounterVec,
    pub runs_finished: IntCounterVec,
    pub run_seconds: HistogramVec,
    pub runs_active: IntGaugeVec,
    /// Steps by task type and status.
    pub steps: IntCounterVec,
    pub step_seconds: HistogramVec,
    /// Calls to ADB (AQL, platform tables, actions) by kind and status.
    pub adb_calls: IntCounterVec,
    pub adb_seconds: HistogramVec,
    /// Engine store ops (runs, leases, journal) by op and status.
    pub engine_ops: IntCounterVec,
    pub engine_seconds: HistogramVec,
    /// LLM calls by provider and status; tokens by provider and kind.
    pub llm_calls: IntCounterVec,
    pub llm_seconds: HistogramVec,
    pub llm_tokens: IntCounterVec,
    pub llm_call_tokens: HistogramVec,
    /// Policy decisions by result.
    pub policy: IntCounterVec,
}

fn counter(r: &Registry, name: &str, help: &str, labels: &[&str]) -> IntCounterVec {
    let c = IntCounterVec::new(Opts::new(name, help), labels).expect("metric");
    r.register(Box::new(c.clone())).expect("registered once");
    c
}

fn histogram(r: &Registry, name: &str, help: &str, labels: &[&str], buckets: &[f64]) -> HistogramVec {
    let h = HistogramVec::new(HistogramOpts::new(name, help).buckets(buckets.to_vec()), labels).expect("metric");
    r.register(Box::new(h.clone())).expect("registered once");
    h
}

/// The process's metrics.
pub fn get() -> &'static Metrics {
    static M: OnceLock<Metrics> = OnceLock::new();
    M.get_or_init(|| {
        let r = Registry::new_custom(Some("flowr".into()), None).expect("registry");
        let in_flight = IntGauge::with_opts(Opts::new("http_requests_in_flight", "HTTP requests being served")).expect("metric");
        r.register(Box::new(in_flight.clone())).expect("registered once");
        let runs_active = IntGaugeVec::new(Opts::new("runs_active", "Runs executing in this process, by durability"), &["durability"]).expect("metric");
        r.register(Box::new(runs_active.clone())).expect("registered once");
        Metrics {
            requests: counter(&r, "http_requests_total", "HTTP requests by route and status", &["route", "status"]),
            request_seconds: histogram(&r, "http_request_duration_seconds", "HTTP request duration by route", &["route"], SECONDS),
            requests_in_flight: in_flight,
            auth_failures: counter(&r, "auth_failures_total", "Refused requests by reason", &["reason"]),
            tool_calls: counter(&r, "tool_calls_total", "MCP tool calls by tool and status", &["tool", "status"]),
            tool_seconds: histogram(&r, "tool_call_duration_seconds", "MCP tool call duration by tool", &["tool"], SECONDS),
            runs_started: counter(&r, "runs_started_total", "Runs started by durability", &["durability"]),
            runs_finished: counter(&r, "runs_finished_total", "Runs finished by durability and outcome", &["durability", "outcome"]),
            run_seconds: histogram(&r, "run_duration_seconds", "Run duration by durability", &["durability"], SECONDS),
            runs_active,
            steps: counter(&r, "steps_total", "Steps by task type and status", &["task_type", "status"]),
            step_seconds: histogram(&r, "step_duration_seconds", "Step duration by task type", &["task_type"], SECONDS),
            adb_calls: counter(&r, "adb_calls_total", "Calls to ADB by kind and status", &["kind", "status"]),
            adb_seconds: histogram(&r, "adb_call_duration_seconds", "ADB call duration by kind", &["kind"], SECONDS),
            engine_ops: counter(&r, "engine_ops_total", "Engine store ops by op and status", &["op", "status"]),
            engine_seconds: histogram(&r, "engine_op_duration_seconds", "Engine store op duration by op", &["op"], SECONDS),
            llm_calls: counter(&r, "llm_calls_total", "LLM calls by provider and status", &["provider", "status"]),
            llm_seconds: histogram(&r, "llm_call_duration_seconds", "LLM call duration by provider", &["provider"], SECONDS),
            llm_tokens: counter(&r, "llm_tokens_total", "LLM tokens by provider and kind (prompt, completion)", &["provider", "kind"]),
            llm_call_tokens: histogram(&r, "llm_call_tokens", "Tokens per LLM call by provider", &["provider"], TOKENS),
            policy: counter(&r, "policy_decisions_total", "Policy decisions by result", &["decision"]),
            registry: r,
        }
    })
}

/// `ok` / `error`.
pub fn status(ok: bool) -> &'static str {
    if ok {
        "ok"
    } else {
        "error"
    }
}

/// `value` if it is one of `known`, else `other`.
pub fn bounded<'a>(value: &'a str, known: &[&'a str]) -> &'a str {
    if known.contains(&value) {
        value
    } else {
        "other"
    }
}

/// Count one timed call in `calls` / `seconds` under `labels` (the status goes last
/// in `calls`).
pub fn observe(calls: &IntCounterVec, seconds: &HistogramVec, labels: &[&str], ok: bool, took: Duration) {
    let mut with_status: Vec<&str> = labels.to_vec();
    with_status.push(status(ok));
    calls.with_label_values(&with_status).inc();
    seconds.with_label_values(labels).observe(took.as_secs_f64());
}

/// A timer: `Timer::start()`, then `.took()`.
#[derive(Clone, Copy)]
pub struct Timer(Instant);

impl Timer {
    pub fn start() -> Self {
        Self(Instant::now())
    }
    pub fn took(&self) -> Duration {
        self.0.elapsed()
    }
    pub fn ms(&self) -> f64 {
        self.0.elapsed().as_secs_f64() * 1000.0
    }
}

/// Slow threshold for tool calls, steps and calls out (`FLOWR_SLOW_MS`, default 1 s).
pub fn slow() -> Duration {
    static T: OnceLock<Duration> = OnceLock::new();
    *T.get_or_init(|| Duration::from_millis(std::env::var("FLOWR_SLOW_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(1000)))
}

/// Every metric in Prometheus' text format.
pub fn render() -> String {
    let mut out = Vec::new();
    let _ = TextEncoder::new().encode(&get().registry.gather(), &mut out);
    String::from_utf8(out).unwrap_or_default()
}

/// Every metric as JSON: counters and gauges by labels; histograms with count, sum,
/// mean and p50 / p90 / p99 estimated from their buckets.
pub fn snapshot() -> serde_json::Value {
    use prometheus::proto::MetricType;
    let mut out = serde_json::Map::new();
    for family in get().registry.gather() {
        let mut series = Vec::new();
        for m in family.get_metric() {
            let labels: serde_json::Map<String, serde_json::Value> =
                m.get_label().iter().map(|l| (l.get_name().to_string(), l.get_value().into())).collect();
            let value = match family.get_field_type() {
                MetricType::COUNTER => serde_json::json!(m.get_counter().get_value()),
                MetricType::GAUGE => serde_json::json!(m.get_gauge().get_value()),
                MetricType::HISTOGRAM => {
                    let h = m.get_histogram();
                    let count = h.get_sample_count();
                    let buckets: Vec<(f64, u64)> = h.get_bucket().iter().map(|b| (b.get_upper_bound(), b.get_cumulative_count())).collect();
                    let q = |p: f64| quantile(&buckets, count, p);
                    serde_json::json!({
                        "count": count,
                        "sum": h.get_sample_sum(),
                        "mean": if count > 0 { h.get_sample_sum() / count as f64 } else { 0.0 },
                        "p50": q(0.5), "p90": q(0.9), "p99": q(0.99),
                    })
                }
                _ => continue,
            };
            series.push(serde_json::json!({ "labels": labels, "value": value }));
        }
        out.insert(family.get_name().to_string(), serde_json::Value::Array(series));
    }
    serde_json::Value::Object(out)
}

fn quantile(buckets: &[(f64, u64)], count: u64, p: f64) -> f64 {
    if count == 0 {
        return 0.0;
    }
    let rank = p * count as f64;
    let (mut lower, mut below) = (0.0, 0u64);
    for &(upper, cum) in buckets {
        if cum as f64 >= rank {
            let inside = (cum - below) as f64;
            let frac = if inside > 0.0 { (rank - below as f64) / inside } else { 1.0 };
            return lower + (upper - lower) * frac.clamp(0.0, 1.0);
        }
        lower = upper;
        below = cum;
    }
    lower
}

/// LLM providers known by name; any other is `other`.
pub const LLM_PROVIDERS: &[&str] =
    &["openai", "anthropic", "openrouter", "ollama", "azure", "bedrock", "google", "gemini", "mistral", "groq", "deepseek", "vllm", "local"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_bounded_and_render() {
        assert_eq!(bounded("openai", LLM_PROVIDERS), "openai");
        assert_eq!(bounded("my-proxy", LLM_PROVIDERS), "other");
        observe(&get().tool_calls, &get().tool_seconds, &["get_state"], true, Duration::from_millis(3));
        let text = render();
        assert!(text.contains("flowr_tool_calls_total{status=\"ok\",tool=\"get_state\"} 1"), "{text}");
        let json = snapshot();
        assert_eq!(json["flowr_tool_call_duration_seconds"][0]["value"]["count"], 1);
    }
}

/// The provider an LLM API URL belongs to (by host), from [`LLM_PROVIDERS`]; `other`
/// for any host not known — a user-configured name never becomes a label.
pub fn llm_provider(api_url: &str) -> &'static str {
    let host = api_url.split("://").nth(1).unwrap_or(api_url).split(['/', ':']).next().unwrap_or("").to_ascii_lowercase();
    let known: &[(&str, &'static str)] = &[
        ("openrouter", "openrouter"),
        ("anthropic", "anthropic"),
        ("openai.azure", "azure"),
        ("openai", "openai"),
        ("bedrock", "bedrock"),
        ("googleapis", "google"),
        ("mistral", "mistral"),
        ("groq", "groq"),
        ("deepseek", "deepseek"),
        ("ollama", "ollama"),
        ("localhost", "local"),
        ("127.0.0.1", "local"),
    ];
    known.iter().find(|(k, _)| host.contains(k)).map(|(_, p)| *p).unwrap_or("other")
}

/// One LLM call: count, time, and the tokens its `usage` reports (OpenAI's
/// `prompt_tokens` / `completion_tokens` or Anthropic's `input_tokens` / `output_tokens`).
pub fn observe_llm(provider: &str, ok: bool, took: Duration, usage: Option<&serde_json::Value>) {
    let m = get();
    observe(&m.llm_calls, &m.llm_seconds, &[provider], ok, took);
    let n = |a: &str, b: &str| usage.and_then(|u| u.get(a).or_else(|| u.get(b))).and_then(|v| v.as_u64()).unwrap_or(0);
    let (prompt, completion) = (n("prompt_tokens", "input_tokens"), n("completion_tokens", "output_tokens"));
    if prompt + completion > 0 {
        m.llm_tokens.with_label_values(&[provider, "prompt"]).inc_by(prompt);
        m.llm_tokens.with_label_values(&[provider, "completion"]).inc_by(completion);
        m.llm_call_tokens.with_label_values(&[provider]).observe((prompt + completion) as f64);
    }
}
