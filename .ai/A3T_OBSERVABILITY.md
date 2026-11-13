# A3T (FlowR) metrics and logs (AO-1 to AO-7)

Status: started 9 Nov 2026. Tasks and status: "Metrics and logs → A3T (FlowR)" in the
core-features doc. ADB's side: `ADB/.ai/OBSERVABILITY.md`.

Why: FlowR had no metrics (`/agents/:id/metrics` answers `{}`) and its one tool log
line named neither the tool nor its time (`Executing tool tool=tools/call`), so a
console query's 95 ms could not be split between the browser, FlowR and ADB (8 Nov).

## Outputs

Every FlowR process (console, gateway, runtimes) serves:

| Output | Where | Notes |
|---|---|---|
| Prometheus text | `GET /metrics` | FlowR's `flowr_*` metrics, then an embedded ADB's `adb_*` (CE). Needs a token unless `FLOWR_METRICS_OPEN=1` (EE auth) |
| JSON with p50 / p90 / p99 | `GET /admin/stats/metrics` | for the A3T UI |
| Logs at `info` | stdout | tool calls (name, ms, ok), finished runs (flow, durability, outcome, ms), slow steps / tool calls / ADB calls / engine ops (over `FLOWR_SLOW_MS`, default 1000) |
| Logs at `debug`, per target | `RUST_LOG=info,flowr::step=debug` | `flowr::step`, `flowr::adb`, `flowr::engine`, `flowr::llm` |

Rules: labels from fixed sets — tool names from FlowR's own tool list, task types from
registered handlers, engine ops and ADB call kinds from FlowR's code, LLM providers
from the API URL's host against a known list (`flowr_metrics::llm_provider`), HTTP
routes as templates; anything else is `other`. No label or log line carries a
payload, a prompt or a token.

## Metrics (prefix `flowr_`, crate `shared/flowr-metrics`)

| Name | Type | Labels | Recorded in |
|---|---|---|---|
| `http_requests_total`, `http_request_duration_seconds`, `http_requests_in_flight` | counter, histogram, gauge | route, status (ok, denied, client_error, server_error) | `flowr-ce` router middleware (`build_ce_router`) |
| `auth_failures_total` | counter | reason (no_token, bad_token) | EE auth (`flowr-server/src/auth.rs`) |
| `tool_calls_total`, `tool_call_duration_seconds` | counter, histogram | tool, status | MCP `tools/call` (`flowr-ce/src/lib.rs`) |
| `runs_started_total`, `runs_finished_total`, `run_duration_seconds`, `runs_active` | counter, counter, histogram, gauge | durability (durable, transient); outcome | `run_request` (`flowr-ce/src/lib.rs`) |
| `steps_total`, `step_duration_seconds` | counter, histogram | task_type, status | step execution (`swf-runtime/src/executor/mod.rs`) |
| `policy_decisions_total` | counter | decision (allow, deny, ask) | same |
| `adb_calls_total`, `adb_call_duration_seconds` | counter, histogram | kind (`action:<name>`, get, sql, put), status | `call_adb.rs` Flight helpers (call:adb, platform tables, registry, query_resource) |
| `engine_ops_total`, `engine_op_duration_seconds` | counter, histogram | op, status | `engine_adb.rs` (remote engine store) |
| `llm_calls_total`, `llm_call_duration_seconds`, `llm_tokens_total`, `llm_call_tokens` | counter, histogram, counter, histogram | provider, status; kind (prompt, completion) | `call_agent.rs` (agent loop), `call_llm.rs` (call:llm) |

Planned: AO-5 queue and workers (claims by pool, lease renewals failed, runs taken
over); AO-6 a 60 s summary line; AO-7 the query console showing ADB time, FlowR time
and total (UI, `a3t-ui` branch).
