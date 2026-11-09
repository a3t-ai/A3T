# Core agent: requests to FlowR and ADB

*9 Nov 2026, from the core agent work (branch `core-agent`).*

The core agent is `system/agent-engine` 2.1.0, and every A3T agent runs through it. Its
flows and its test are in this branch:

- flows: `a3t/backend/flows/system/agent-engine.yaml`, `agent_turn.yaml`,
  `agent_knowledge_ingest.yaml`, `agent_tools_sync.yaml`, `a3t/a3t_chat_engine.yaml`;
- test: `a3t/backend/tests/core-agent.mjs`, which runs on a real FlowR EE, ADB and a stub LLM.

Flows and UI belong to the A3T side. These requests need Rust changes in FlowR or ADB, so
the A3T side does not make them. Each request says:

- what is wrong;
- where it is in the code;
- what "done" looks like;
- which flow is waiting for it.

Where the flows work around a gap today, the request says how. That workaround comes
out when the request lands.

## FlowR

| ID | Priority | Request |
|---|---|---|
| CA-R1 | high | `call: a3t:agent` replaces the caller's `$context` |
| CA-R2 | high | `$workflow` and `$runtime` are null in a subflow |
| CA-R3 | high | Write the whole turn to WORKING and EPISODIC |
| CA-R4 | high | Errors inside a turn are handled once and reported |
| CA-R5 | high | One failure is logged once, not once per enclosing flow |
| CA-R6 | high | The engine's PROCEDURAL recall has no namespace |
| CA-R7 | medium | Inheriting steps and hooks from the core agent |
| CA-R8 | medium | `try` / `catch` in the CNCF shape is silently ignored |
| CA-R9 | medium | Hooks run `set` tasks only |
| CA-R10 | medium | Reading a field of `null` is an error |
| CA-R11 | low | Retry transient LLM and tool failures |
| CA-R12 | low | The org's ADB resource name, visible to flows |
| CA-R13 | medium | `call: mcp` can't reach the platform's own MCP server |

### CA-R1 — `call: a3t:agent` replaces the caller's `$context`

`call_agent.rs` sets `ctx.context = state.to_context_value(&config)` before the turn and
again at the end (around lines 507 and 1235). It never puts back the context of the flow
that made the call. So everything a flow set before the call is gone afterwards: in the
core agent, that is the recalled knowledge, the tool hints, the soft errors and the start
time.

- **Done when:**
  - the flow's `$context` after the call is what it was before, plus the task's export;
  - the hooks still see the turn's own context while the turn runs.
- **Workaround today:** the call sits alone in its own flow, `system/agent-turn`. When
  this lands, that flow can move back into `system/agent-engine`.

### CA-R2 — `$workflow` and `$runtime` are null in a subflow

At top level, `${ $workflow }` gives `{id, name, namespace, version}`. In a flow started by
`run: workflow`, it is `null`. Every A3T agent runs the core agent as a subflow, so the
core can't name its run. Its `a3t_agent_turns` rows and its `a3t_errors` rows have an
empty `run_id`.

- **Done when:** a subflow sees its own `$workflow`. Ideally it also sees the root run's
  id, `$workflow.root_id`, which joins a turn to the run the user started.
- **Workaround today:** `(($workflow // {}).id // "")`.
- **Test:** `core-agent.mjs` reports it as known gap CA-R2.

### CA-R3 — Write the whole turn to WORKING and EPISODIC

`agent_memory.rs::store_turn` (around line 422) writes only two things:

- the user's message, to WORKING;
- the reply text, to EPISODIC.

The assistant's tool calls, their results, the outcome and the run id are not written,
so an episode can't be resumed, audited or learned from.

- **Done when:**
  - WORKING holds each message of the turn as its own item: user, assistant, and each
    tool call with its result (truncated). Fields: `session_id`, `role`, `turn`, `owner`.
  - EPISODIC holds one episode per turn with these fields:
    - `intent`, the user's message;
    - `actions`, as `[{tool, server, ok, ms}]`;
    - `outcome`: `answered`, `tool_error`, `max_iterations` or `procedure`;
    - `reply`, truncated;
    - `run_id`, `session_id`, `owner` and `turn`.
  - `store_turn` returns a `Result`, so a failed write reaches CA-R4.
  - The turn's output carries `actions` and `usage` (tokens, ms), so the flow's turn log
    can record them. Today `a3t_agent_turns` has only counts.
- This is phase A1 of `.ai/A3T_FLOWS_ADB_ARCHITECTURE.md` §3.2.

### CA-R4 — Errors inside a turn are handled once and reported

Today these failures inside a turn are only a `warn!` in the server log:

- a failed tool call;
- a failed memory recall or write;
- a failed reflection;
- a failed MCP `tools/list`.

The turn goes on, which is right, but nobody can see them.

- **Done when:**
  - each such failure is passed to the flow's `onError` handler (`system/a3t-error-handler`)
    as a handled error: one `a3t_errors` row, the run goes on;
  - the turn's output lists them as `degraded: [{step, class, ref}]`, as the core agent
    already does for its own steps;
  - `_tool_error` is set in the hook context, so the `on_tool_error` hook can match. Today
    it never fires, because nothing sets `_tool_error`.

### CA-R5 — One failure is logged once, not once per enclosing flow

When an LLM call fails in `system/agent-turn`, `a3t_errors` gets three rows: one for the
turn, one for the core agent, and one for the A3T agent that started it. A nested
failure reaches the parent as a new error, "Nested workflow failed: raised error …", so
each parent's `onError` logs it again. ST-1 meant an already-handled error to pass
through.

- **Done when:** the error a nested run fails with carries the mark that it was handled,
  and enclosing `onError` hooks pass it through.
- **Test:** `core-agent.mjs` reports it as known gap CA-R5.

### CA-R6 — The engine's PROCEDURAL recall has no namespace

`agent_memory.rs` around line 362 runs `RECALL FROM PROCEDURAL PATTERN $m …` with no
`NAMESPACE`. So an agent with `procedural.read` on sees every namespace's procedures,
including the core's tool catalogue (namespace `system-tools`, kind `tool`). A tool
record that matches a message well enough becomes a canned answer.

- **Done when:** the recall is limited to the agent's namespace and skips
  `kind = "tool"` records.
- **Until then:** the core keeps `procedural.read` off by default, and the catalogue
  carries `kind = "tool"`.

### CA-R7 — Inheriting steps and hooks from the core agent

`metadata.a3t.extends` passes down hooks and the `metadata.a3t` envelope, but not `do`
(`swf-parser/src/extends.rs`). So an A3T agent runs the core as a subflow
(`run: workflow system/agent-engine`). Hooks the agent declares are not passed to that
subflow, so they never reach the turn.

- **Done when:**
  - a child that has `extends` and no `do` runs its base's `do`, with its own
    `metadata.a3t.config` merged under the input's `config`;
  - the child's hooks apply to that run.

  An A3T agent then becomes a config document over the core, which is what
  "Template = code, Agent = values" asks for.

### CA-R8 — `try` / `catch` in the CNCF shape is silently ignored

In CNCF 1.0, `catch` sits beside `try`. FlowR's `TryTask`
(`swf-parser/src/flow_control.rs:78`) wants it inside: `try: {do, catch}`. A flow
written in the CNCF shape loads, and its `catch` is dropped without a word, so errors
are not caught.

- **Done when:** both shapes work, or the CNCF shape is refused at save with a message.

### CA-R9 — Hooks run `set` tasks only

`fire_before` and `fire_after` in `call_agent.rs` (around line 1249) run only
`Task::Set`. Any other task in a hook is skipped without a word. A hook can't log,
call ADB or run a flow.

- **Done when:** hooks can run any task, or other tasks are refused at save with a
  message.

### CA-R10 — Reading a field of `null` is an error

In jq, `null | .a` is `null`. In FlowR's jaq, it fails with "cannot use null as
iterable". So `$context._row.mode` fails when `_row` is null, and
`$workflow.id` fails in a subflow.

- **Done when:** field access on null gives null, as jq and the CNCF expression spec do.
- **Until then:** flows guard with `(x // {}).a`.

### CA-R11 — Retry transient LLM and tool failures

The error handler classifies 429, 503, timeouts and resets as `transient`, but nothing
tries again.

- **Done when:** LLM and tool calls inside a turn retry transient failures with backoff,
  limited by the run's step and time budget (AC-8).

### CA-R12 — The org's ADB resource name, visible to flows

`agent_space::org_adb()` reads `A3T_ADB_RESOURCE`, and flows can't see it. So the core
defaults to `a3t/agent-memory`.

- **Done when:** flows can read it, for example as `$runtime.adb`.

### CA-R13 — `call: mcp` can't reach the platform's own MCP server

`call_mcp.rs` posts JSON-RPC over plain HTTP, with no credentials except ADB's. On EE,
FlowR's own MCP server needs a bearer token, so `tools/list` from a flow fails with
"MCP response parse error". The 401 body is not JSON, so the real cause is lost too.

The agent turn already reaches its own server in-process
(`call_agent.rs::is_local_mcp_url`), as the run's principal.

- **Done when:**
  - `call: mcp` to the platform's own server dispatches in-process as the run's
    principal, as the turn does;
  - a call to an external server sends the resource's credentials;
  - a non-2xx answer fails with its HTTP status.
- **Workaround today:** `load-core-agent-memory.mjs` lists the platform's tools with the
  user's token and passes them to `system/agent-tools-sync` as `tools` with
  `complete_for`.

## ADB

| ID | Priority | Request |
|---|---|---|
| ADB-R1 | high | Variables in `STORE` and `UPDATE` values |
| ADB-R2 | medium | Store-or-update in one statement |
| ADB-R3 | low | `LIKE` returns its score |

### ADB-R1 — Variables in `STORE` and `UPDATE` values

`STORE INTO SEMANTIC (key = $k, content = $c)` with `variables` stores the literal text
`$k` and `$c`. Variables do work in `WHERE` and `LIKE`.

- **Done when:** variables are substituted in payload values too.
- **Until then:** the flows quote values themselves, escaping `\` and `"`.

### ADB-R2 — Store-or-update in one statement

The ingest and tool sync flows do a `STORE`, catch `key_exists`, then `UPDATE`: two
round trips and a caught error for each record that already exists.

- **Done when:** one statement stores the record, or replaces it if the key exists. For
  example `STORE … ON KEY UPDATE`, or `UPSERT INTO`.

### ADB-R3 — `LIKE` returns its score

`RECALL … LIKE` returns records in rank order, but without a score in `metadata`
(`PATTERN` has `metadata.confidence`). The core can't drop weak hits, so it always takes
the top N.

- **Done when:** each record's metadata carries its similarity.

Also seen while testing locally: with no embedding model (`ADB_EMBED_MODEL_DIR` unset),
the ADB log says "LIKE finds nothing". It did rank results in the test, so that log line
is wrong, or the ranking falls back to lexical. Worth checking.
