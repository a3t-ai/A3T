# The core agent: `system/agent-engine`

Every agent in A3T runs through one flow, `system/agent-engine`. It does the work every
agent needs, so an agent you build only says what makes it different: its instructions,
model, tools and memory settings.

On every turn, the core:

- gives the agent its memory: working and episodic memory per conversation, on ADB;
- recalls reference knowledge: the FlowR docs, from semantic memory;
- recalls the tools that fit the request, from its tool catalogue in procedural memory;
- logs one row per turn;
- reports every failure once.

The core lives in the `system` namespace. A3T agents (namespace `a3t`, or a project's
own) extend it.

## The flows

| Flow | What it does |
|---|---|
| `system/agent-engine` | **The core agent.** Applies defaults, recalls knowledge and tools, assembles the prompt, runs the turn, logs it and answers. |
| `system/agent-turn` | One turn on the native engine (`call: a3t:agent`): memory recall, the LLM and tool loop, the memory write, the session save. Only the core runs it. |
| `system/agent-knowledge-ingest` | Loads documents into semantic memory as sections (namespace `system-knowledge`). |
| `system/agent-tools-sync` | Keeps the tool catalogue in procedural memory (namespace `system-tools`), from the MCP servers. |
| `system/a3t-base` | The base of every platform flow. Its error handler is locked, so no flow can switch it off. |
| `system/a3t-error-handler` | Sorts a failure into one of five classes (policy, transient, limit, input, bug), writes one `a3t_errors` row, and gives the user a plain message. |
| `a3t/a3t-chat-engine` | A3T chat: the first A3T agent on the core. |

## A turn, step by step

```
input {message, session_id, ui_context?, config?}
  1. settings     the core's defaults, with the agent's config merged over them
  2. knowledge    RECALL FROM SEMANTIC LIKE <message> NAMESPACE "system-knowledge" LIMIT 4
  3. tools        RECALL FROM PROCEDURAL LIKE <message> NAMESPACE "system-tools" LIMIT 6
  4. prompt       the agent's instructions, then "## Reference" and "## Tools that fit this request"
  5. turn         system/agent-turn → call: a3t:agent
                    recall: WORKING + EPISODIC of this session; SEMANTIC facts of the agent
                    LLM ↔ tools loop (tools from the agent's MCP servers, with live permissions)
                    store:  WORKING ← the user's message; EPISODIC ← the reply
                    session saved in the state store
  6. log          one row in a3t_agent_turns
  7. answer       {content, session_id, tool_calls_made, iterations, procedural_hit,
                   stopped_reason, knowledge, tool_hints, degraded}
```

Steps 2, 3 and 6 never stop a turn. If one fails:

- it goes to `system/a3t-error-handler`, which writes one `a3t_errors` row;
- it is listed in the answer's `degraded` as `{step, class, ref}`;
- the turn goes on without it.

If the turn itself fails (the LLM refuses, a limit is hit), the run fails. The base's
error handler then answers in plain words: "Not allowed: …", "A service A3T depends on
didn't answer in time …", and so on.

## Memory

| Memory type | What is in it | Namespace | Written by | Read by |
|---|---|---|---|---|
| WORKING | this conversation's messages | the agent's own (`<project>--<agent>`), or `config.memory_namespace` | the turn | the turn |
| EPISODIC | what the agent answered, per turn | the agent's own | the turn | the turn |
| SEMANTIC (the agent's facts) | lasting facts about the user and the work | the agent's own | reflection, when `memory_config.semantic.reflect` is on | the turn |
| SEMANTIC (knowledge) | the FlowR docs, in sections | `system-knowledge`, shared | `system/agent-knowledge-ingest` | the core, step 2 |
| PROCEDURAL (tool catalogue) | one record per tool: name, server, description, usage, schema | `system-tools`, shared | `system/agent-tools-sync` | the core, step 3 |

- **Where it is:** all memory is on the org's ADB, resource `a3t/adb`, unless
  the agent names another one.
- **Agents with a memory space:** an agent that has its own space (Agents → Memory, or
  `PUT /agents/{id}/memory`) always uses it. Branches, forks and tags work on that space.
- **The tool list itself:** the live tool list still comes from the agent's MCP servers
  on every turn, because that is where permissions are checked. The catalogue is what the
  agent recalls to choose among them.

## Settings (`config`)

The agent's `config` is merged over these defaults, key by key:

```yaml
system_prompt: "You are a helpful assistant."
llm_resource: <none>            # e.g. a3t/Austra; required in practice
mcp_servers: []                 # e.g. [a3t/agent-flows]
enabled_tools: <absent>         # absent = every tool of those servers; a list narrows it
temperature: 0.7
max_iterations: 10
memory_resource: a3t/adb   # null = no memory at all
memory_namespace: default
memory_config:
  working:    {read: true, write: true}
  episodic:   {read: true, write: true}
  semantic:   {read: true}          # reflect: true writes facts after each turn
  procedural: {read: false}         # learned procedures; off until CA-R6
knowledge:      {enabled: true, namespace: system-knowledge, limit: 4, max_chars: 600}   # resource: defaults to memory_resource
tool_catalogue: {enabled: true, namespace: system-tools, limit: 6}                       # resource: defaults to memory_resource
```

Error handling and the turn log are not settings. They are steps of the core, and its
`onError` is locked.

## Build an agent on the core

An A3T agent is a short flow. It extends the core and runs it with its own config:

```yaml
document: { dsl: '1.0.3', namespace: my-project, name: release-checker, version: '1.0.0' }
metadata:
  a3t:
    extends: system/agent-engine@2.1.0     # the core's base: error handler, locks
do:
  - agent:
      run:
        workflow:
          namespace: system
          name: agent-engine
          input:
            message: '${ .message }'
            session_id: '${ .session_id }'
            config: >-
              ${ {
                  system_prompt: "You check releases before they ship. …",
                  llm_resource: "a3t/Austra",
                  mcp_servers: ["a3t/agent-flows"],
                  temperature: 0.2
                } * (.config // {}) }
```

- **Session ids:** pass a session id of the form `<agent-id>:<anything>` (for example
  `agt-1a2b:web-42`). Memory and the owner check then belong to that agent.
- **Reading the agent's values:** `a3t/a3t-chat-engine` is a full example. It reads the
  agent's record (`agent_configs`) and turns its values into `config`.

In the console, agents made from the `agent-engine` template run through
`a3t/a3t-chat-engine`, so they get the core without writing a flow.

## Clone and use an agent

Ways to copy an agent, by where you work:

- **Console:** Agents → an agent → Releases → Clone (a copy with fresh memory) or Fork (a
  copy that starts from its memory).
- **REST, fork:** `POST /agents/{id}/fork` with `{"new_agent": {"name": "…"}}`. This
  copies the agent's record and its memory's current `main` into a new agent.
- **REST, to another project:** `POST /agents/{id}/transfer` with
  `{"project": "…", "new_agent": {"name": "…"}}`. This carries procedures only: no
  episodes or facts.
- **The core itself:** to give an org its own core, copy `agent-engine.yaml` into a
  project namespace and change it there. A3T agents keep extending `system/agent-engine`
  unless you point them at the copy.

Ways to call an agent:

- **Chat:** `POST /chat` with `{message, session_id: "<agent-id>:<session>"}`. The answer
  streams back (AI SDK UI Message Stream v1).
- **REST:** the agent's endpoint, `POST /webhook/<path>` (Agents → Exposure).
- **MCP:** a client connected to FlowR's MCP server calls `execute_flow` with the agent's
  flow.

## Load the shared memory

Run after an install, and again after the docs or the tools change:

```sh
FLOWR_TOKEN=<your console token> node a3t/backend/scripts/load-core-agent-memory.mjs \
  --url https://<flowr>
```

What it loads:

- **Knowledge:** the `a3t/wwwdocs` pages and this guide, through
  `system/agent-knowledge-ingest`.
- **Tools:** the platform's own tools, as server `a3t/agent-flows`. The script lists them
  with your token. It also loads the tools of any `--servers <ns/name>,…` that FlowR can
  list itself. Both go through `system/agent-tools-sync`.

Running it again updates in place and forgets what is gone. Add `--dry-run` to see what
it would send.

## Logs

| Where | What |
|---|---|
| `a3t_agent_turns` (platform DB) | One row per turn: `run_id`, `session_id`, `agent_id`, `principal`, `llm`, `memory`, `outcome`, `iterations`, `tool_calls`, `knowledge`, `tool_hints`, `degraded`, `duration_ms`. |
| `a3t_errors` (platform DB) | One row per handled failure: `class`, `code`, `flow`, `step`, `run_id`, `detail`, `user_message`. Read it through `a3t/a3t-error-log` (Admin and SuperAdmin). |
| `agent_runs`, `agent_usage` | Written by the engine for agents with a memory space: version, memory point, tokens and time per turn. |

## Writing flows for FlowR: where it differs from CNCF 1.0

- **`catch` goes inside `try`:** `try: {do: [...], catch: {do: [...]}}`. A `catch` placed
  beside `try` (the CNCF shape) is ignored without a word (CA-R8).
- **Reading a field of `null` fails:** in FlowR, `null | .a` is an error. Guard with
  `(x // {}).a` (CA-R10).
- **`$workflow` and `$runtime` are `null` in a subflow:** guard with
  `($workflow // {}).id` (CA-R2).
- **Agent hooks (`use.extensions` on `system/agent-turn`) run `set` tasks only** (CA-R9).
- **`create_flow` doesn't replace a flow that exists:** use `update_flow`.
- **`call: mcp` can't reach the platform's own MCP server on EE**, which needs a token
  (CA-R13).
- **AQL `STORE` and `UPDATE` take no `$variables` in their values:** quote values in the
  expression and escape `\` and `"` (ADB-R1). `WHERE` and `LIKE` do take variables.

## Limits

These need FlowR or ADB changes, requested in `.ai/CORE_AGENT_ENGINE_REQUESTS.md`:

- **What the turn writes to memory:** WORKING gets only the user's message, and EPISODIC
  only the reply text. Tool calls, results, outcome and run id are not written (CA-R3).
- **Failures inside a turn:** tool, memory and reflection failures are only in the server
  log, not in `a3t_errors` (CA-R4).
- **Duplicate error rows:** one failure is logged by each enclosing flow, which makes
  three rows for an A3T agent (CA-R5).
- **Run ids:** turn and error rows from an A3T agent have an empty `run_id` (CA-R2).
- **Child agents:** hooks a child agent declares don't reach the turn, and a child can't
  inherit the core's steps without running it as a subflow (CA-R7).

## Test

```sh
node a3t/backend/tests/core-agent.mjs --adb <adb binary> --flowr <flowr EE binary>
```

It starts its own ADB, FlowR and stub LLM, and checks:

- ingest and tool sync;
- the prompt, memory and turn log;
- a child agent;
- a degraded turn and a failed turn;
- the chat engine reading an agent's record.

Engine gaps are shown as `!` and are not counted as failures.
