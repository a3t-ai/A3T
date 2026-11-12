# Core agent hooks: spec and the FlowR work it needs

*12 Nov 2026. Branch `core-agent`. Follows `.ai/CORE_AGENT_ENGINE_REQUESTS.md`.*

## What the user asked for

- An agent is a call to an agent framework: `call: a3t:agent` today, and later
  `langchain:agent`, `crew:agent` and `strands:agent`.
- The call gives every phase of a turn as a hook (`use.extensions`).
- Flows are connected to hooks.
- The system agent (`system/agent-engine`) owns everything around the call:
  - error and exception handling;
  - logging;
  - its core hooks.
- An A3T agent inherits the system agent. The A3T agent's hooks add to the core hooks
  and do not replace them, so every hook propagates all the way down to the system agent.

## The shape

```yaml
# system/agent-engine 3.0.0, the core template
document: { dsl: '1.0.3', namespace: system, name: agent-engine, version: '3.0.0' }
metadata:
  a3t:
    extends: system/a3t-base@1.0.0           # onError locked: every failure → a3t-error-handler
    locked: [onError, 'extension:on_turn_start', 'extension:on_tool_error', 'extension:on_turn_end']
use:
  extensions:
    - on_turn_start:   { extend: all, when: '${ $task.name == "init_turn" }',
                         before: [ {start: {run: {workflow: {namespace: system, name: agent-hook-turn-start}}}} ] }
    - before_recall:   { extend: all, when: '${ $task.name == "recall_memory" }', before: [], after: [] }
    - before_llm:      { extend: all, when: '${ $task.name == "call_llm" }',
                         before: [ {docs: {run: {workflow: {namespace: system, name: agent-hook-knowledge}}}} ] }
    - on_llm_response: { extend: all, when: '${ $task.name == "call_llm" }', before: [], after: [] }
    - on_tool_call:    { extend: all, when: '${ $task.name == "dispatch_tools" }', before: [], after: [] }
    - on_tool_error:   { extend: all, when: '${ $task.name == "dispatch_tools" and $context._tool_error != null }',
                         after: [ {report: {run: {workflow: {namespace: system, name: a3t-error-handler}}}} ] }
    - before_store:    { extend: all, when: '${ $task.name == "store_memory" }', before: [], after: [] }
    - after_response:  { extend: all, when: '${ $task.name == "emit_response" }', before: [], after: [] }
    - on_turn_end:     { extend: all, when: '${ $task.name == "save_session" }',
                         after: [ {log: {run: {workflow: {namespace: system, name: agent-hook-turn-log}}}} ] }
do:
  - run_agent:
      call: a3t:agent
      with:
        config: '${ {<core defaults>} * (.config // {}) }'
        session_id: '${ .session_id // "default" }'
        message: '${ .message }'
        ui_context: '${ .ui_context }'
```

```yaml
# An A3T agent: inherits all of the above, adds its own hooks and values
document: { dsl: '1.0.3', namespace: a3t, name: a3t-chat-engine, version: '5.0.0' }
metadata:
  a3t:
    extends: system/agent-engine@3.0.0
    config: { system_prompt: "...", llm_resource: a3t/Austra, mcp_servers: [a3t/agent-flows] }
use:
  extensions:
    - on_turn_end: { after: [ {usage: {run: {workflow: {namespace: a3t, name: a3t-usage}}}} ] }   # runs after the core's
```

These are the flows the core's hooks run:

| Flow | Hook | What it does |
|---|---|---|
| `system/agent-hook-turn-start` | on_turn_start | Start time, principal and run id, into the turn's context |
| `system/agent-hook-knowledge` | before_llm | Recalls the FlowR doc sections (SEMANTIC `system-knowledge`) and adds them to the prompt. A failure is reported and the turn goes on |
| `system/a3t-error-handler` | on_tool_error, onError | One `a3t_errors` row and a plain message (exists) |
| `system/agent-hook-turn-log` | on_turn_end | One `a3t_agent_turns` row: outcome, iterations, tool calls, docs used, duration |

Tools need no step of their own. They are the call's own settings: `mcp_servers`,
`enabled_tools` and `tool_flows`, with permissions checked live on every turn.

## What FlowR does today (checked 12 Nov, local FlowR EE + probes)

1. **Hooks inside `a3t:agent` never run.** `fire_before` and `fire_after` in
   `call_agent.rs` evaluate with `ctx.eval_fn`, and nothing ever sets it. Only the
   executor's own `eval_fn` is set, in `swf-runtime/src/executor/mod.rs:137`. So every
   hook in every agent flow does nothing, including those in the 2.0.0 template.
   - Probe: three hooks (`when` on `emit_response`, no `when`, `extend: call`), and
     none fired.
2. **When they do run, they run `set` only.** Any other task is skipped, so a hook can't
   run a flow (CA-R9).
3. **`$task` is not bound in a hook's `when`.** The `$task.name == "…"` conditions
   can't match.
4. **A hook can't change the turn.** The engine writes the turn's state into the
   context before each phase, and never reads it back. So a hook can't add to the
   prompt, the messages, the tool arguments or the reply.
5. **Inheritance replaces, it doesn't chain.** In `extends.rs::apply_base`, a child's
   hook with the same name replaces the base's, or is refused if the base locks it. It
   never runs together with the base's.
6. **`on_tool_error` never fires,** because nothing sets `_tool_error` (CA-R4).
7. **After the call, the calling flow's context is gone** (CA-R1).

## Requests to FlowR (Rust; the A3T side does not change it)

| ID | Priority | Request |
|---|---|---|
| AH-1 | high | Agent hooks run: `fire_before`/`fire_after` use the executor's evaluator; `$task` is bound to `{name: <phase>, type: <call:llm…>}` in `when` and in the tasks. |
| AH-2 | high | A hook runs any task, through the normal executor: `run: workflow`, `call: adb`, `call: lancedb`, `set`, `try`. A failing hook task goes to the flow's `onError` as a handled error, and the turn goes on. |
| AH-3 | high | A hook can change the turn, through named fields read back after each hook: `_system_prompt`, `_messages` (before_llm), `_tool_call` args (on_tool_call), `_agent_response` (after_response). Anything else a hook sets stays in the turn's context for later hooks. |
| AH-4 | high | Inherited hooks chain. A child's hook with a base's hook name runs inside the base's: base before, child before, phase, child after, base after. `locked` means the child can't remove or replace the base's, but can still add its own. Today it can only replace. |
| AH-5 | medium | One hook contract for every agent framework. The phase names and context fields above are the contract. `call: <framework>:agent` handlers (langchain, crew, strands) fire the same phases, so the same hooks and flows work unchanged. |
| AH-6 | medium | `on_tool_error` fires: `_tool_error` = `{tool, server, error}` in the hook context (CA-R4). |
| AH-7 | medium | The calling flow's context is restored after the call, with the call's output added (CA-R1). With AH-1..4 the core no longer needs this, but other flows do. |

**Done when:** `system/agent-engine` 3.0.0 has the shape above and its test passes on a
real FlowR EE:

- each core hook's flow runs once per turn;
- the knowledge hook's doc sections reach the LLM's prompt;
- a hook the A3T agent adds runs together with the core's;
- a tool failure writes one `a3t_errors` row;
- one `a3t_agent_turns` row per turn.

## Until then

- `system/agent-engine` 2.1.0 stays on vc-dev. It does the same work as steps around
  the call, in a second flow (`system/agent-turn`), because of point 7.
- Its hooks don't run (point 1); this applies to all agent flows, not only this one.
- Once AH-1 to AH-4 land, 3.0.0 replaces it, `system/agent-turn` is deleted, and the
  A3T agents move to `extends: system/agent-engine@3.0.0`.
