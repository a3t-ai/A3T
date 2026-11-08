# A3T console — UI limitations and the APIs they need

Started 3 Nov 2026, with the persona-driven agent hub (branch `a3t-ui`). The console
was rebuilt around four ways people come to an agent — a learner exploring, a
developer, a security reviewer, an SRE — using only the APIs that exist. Where a
screen needs data no API gives, the UI says so and names the entry below
(`UI-n`); it never shows made-up data.

Status: **open** (the UI shows a gap), **partial** (the UI approximates it, as
described), **done** (with commit).

## Where each persona lands

| Persona | Way in | Lands on |
|---|---|---|
| Learner / agent infra owner | Chat home: "What is A3T?", "Create an agent", "Explore an agent" | Agent → Overview, then Activity, Memory, Controls |
| Developer | Agent → Code ("Open in Studio"), Deploy | Studio: run, debug, versions; canvas |
| Security reviewer | Chat home: "Review security"; agent → Controls | Decision log (filtered to the agent), Controls |
| SRE | Chat home: "Check health"; agent → Health | Runtimes, agent Health |

## API gaps

| # | What the UI needs | Where it shows | Today | Proposed API | Status |
|---|---|---|---|---|---|
| UI-1 | Read one agent's memory: entries per tier (working, episodic, semantic, procedural, tools), counts, newest first | Agent → Memory; Memory page | Tier cards show "entries: —"; admins are pointed at Settings → ADB SQL | `GET /agents/{id}/memory?tier=&limit=&cursor=` → `{tiers: [{tier, count, bytes}], entries: [{tier, key, value, ts, scope, source_run}]}`, read through AQL on the agent's scope; Cedar `memory:read` on the agent | open |
| UI-2 | Runs of **this** agent, not of its serving flow | Agent → Activity, Overview tiles, Health | Lists runs of the serving flow (`variables.target`), which agents can share (the chat engine) | `list_runs` gains `agent_id` (from the run's input or its trigger), indexed in the run store | partial |
| UI-3 | Decision-log records about one agent, server-filtered and paged | Agent → Activity, Controls; Decision log | Reads the latest 500 records and filters by agent id in principal / on-behalf-of / resource | `GET /security/audit?principal=agent:{id}&involves={id}&since=&cursor=` | partial |
| UI-4 | All conversations with an agent (for its owner / admins), not only the caller's | Agent → Activity | Shows only the signed-in user's own sessions (`<agent>:<client>` prefix) | `GET /agents/{id}/sessions?cursor=` (owner, admin), titles + counts + last activity, no transcripts without `session:read` | partial |
| UI-5 | What an agent learned, when: memory writes as events | Agent → Memory ("recent learning"), Activity | Not shown | Memory writes emitted as events (`memory.stored {agent, tier, key, run}`) and listable: `GET /agents/{id}/memory/events` | open |
| UI-6 | The agent's outbound identity: gateway, IdP token audience, egress policy | Agent → Controls → Identity | Text explains the policy choke points; no per-agent egress facts | From the gateway pool (cluster design §10): `GET /agents/{id}/egress` → `{gateway, token_audience, allowed_hosts, policy_ids}` | open |
| UI-7 | Per-agent data handling: audit level, transcript retention, redaction hooks in force | Agent → Controls → What is logged | Lists what is logged in general; says retention/redaction have no setting | Agent (or template) `policy.audit {level, retention_days, redact[]}` stored and returned with the agent; enforced in the hooks | open |
| UI-8 | Metrics per agent over time: latency, error rate, tokens, cost | Agent → Health | Deployment totals from `/ops/overview` (runs, last hour, error rate, p50, p95, refused, last error) when the agent is deployed through a canvas; nothing for undeployed agents and no history | `GET /agents/{id}/metrics?window=24h&step=5m` → series `{runs, errors, p50, p95, tokens_in, tokens_out, cost}`; tokens from the LLM call task | partial |
| UI-9 | Runtime events: restarts, crashes, out-of-memory, hibernate/resume | Agent → Health; Runtimes | Not shown; restart *policy* is visible in the canvas | `GET /ops/runtimes/{name}/events` and `GET /ops/deployments/{id}/events` (k8s events + controller decisions) | open |
| UI-10 | Application logs of an agent's runs and its runtime (stdout/stderr, tracing) | Agent → Health; Executions | The console's "Logs" was the decision log; renamed "Decision log" so it no longer suggests application logs | `GET /runs/{id}/logs` and `GET /ops/deployments/{id}/logs?since=&follow=1` (SSE) | open |
| UI-11 | A plain-language purpose for an agent separate from its template's description | Agent → Overview | Uses the template's `document.metadata.description` | Agent record gains `description`; the builder agent fills it | open |
| UI-12 | Alerts: thresholds on error rate / latency that notify | Bell → Notifications | Notifications hold console warnings and errors only | `alerts` resource (`{agent, metric, op, value, window}`) evaluated by the controller; notices pushed to the console | open |
| UI-13 | One run per conversation at a time | AI Chat composer | The composer waits while any permission question is pending (fc80acf); another client could still start a second run | `POST /chat` returns 409 `{waiting_on: request_id}` while the session has a run suspended on an approval | partial |
| UI-14 | Edits to the open document without a permission prompt | AI Chat with a document open | `propose_edit` is added to an agent's explicit tool list; an agent with no list (every call needs approval) still asks once, and the main-chat prompt only names `propose_edit` in designer mode | Treat `propose_edit` / `propose_flow` as no-effect tools that need no approval; in `prompt_with_ui_context`, say "change it with propose_edit" whenever a document is open, not only in designer mode | partial |

## Server limitations

Server faults found while building the UX shell (branch `a3t-dash`, 8 Nov 2026).
They are not missing APIs: the server does something wrong, or drops data the UI
needs. The server code is on `adb-integration` (sri-dev-ea's), so the UI only
works around them where that is safe, and says so here.

| # | Problem | What the user sees | Where | Fix | Status |
|---|---|---|---|---|---|
| SRV-1 | Results of tools on a **remote** (HTTP) MCP server are never parsed as JSON. `serde_json::from_str(text).unwrap_or(Ok(json!(text)))` infers `from_str::<Result<Value, String>>`, so a normal object such as `{"workflows": […]}` fails to parse and the whole result becomes one string. The list-card table then finds no `workflows` / `agents` / … key. The in-process path (`callresult_to_value`) parses correctly. | With some agents, "list flows" (agents, templates, resources, triggers, runtimes) answers with a text table and **no list card** to open the items. Same agents work for single items (`get_flow` cards use the text). | `flowr-ce/crates/flowr-ce/src/handlers/call_agent.rs`, `call_mcp_tool` (line 1979 on `adb-integration` bba72b6; since a3f4b4b, 15 Sep) | `return Ok(serde_json::from_str(text).unwrap_or_else(\|_\| json!(text)));`, the same as `callresult_to_value`. A test: a remote `list_flows` result yields a `data-flows` part. | open |
| SRV-2 | The chat transcript keeps text only. `GET /chat/sessions/{id}` returns `{role, content}`; the agent's session state has the tool traffic but no record of the `data-*` parts (cards) streamed to the user. | Cards (flow lists, an item card, the agent's proposed edits) **disappear after a reload** whenever the conversation is rebuilt from the server: another device or browser, cleared site data, or a reply the browser missed (`rehydrate`). The text stays. The browser's own copy keeps them, so a plain reload on the same browser usually does. | Chat handler (transcript write) and `GET /chat/sessions/{id}`; UI: `loadServerTranscript`, `rehydrate` in `ChatPageAI.svelte` | Store the streamed `data-*` parts with each assistant message in the transcript and return them (`{role, content, parts}`). The UI then restores the parts instead of building text-only messages (small change in the two functions above). Also gives the UI Agent's episodic memory what the user was shown, not only the words. Alternative, not recommended: the UI rebuilds cards from the tool traffic in `get_state agent:session:{id}`, which would copy the server's card table into the browser. | open |
| SRV-3 | `list_resources` returns the same resource twice: `a3t/flowr-system-db`. | Was: the Resources list in Items stayed blank (the repeated key stopped Svelte drawing the list). | `list_resources` (resource registry) | Return each resource once. The UI now drops repeated rows (77db3e6), which hides the duplicate; remove that once the server is fixed, or keep it and log the duplicate. | partial |
| SRV-4 | `ui_context` has no field for the app or the selected item, only for one document (`kind`, `name`, `section`, `dirty`, `draft_text`). | Nothing visible. With no document open, the UI sends the app as `kind: "page", name: "Agents"`, and an item no view has described as `kind: "Settings item", name: "<id>"`. The prompt reads correctly ("The console is currently showing page `Agents`"), but the agent cannot tell the levels apart. | `prompt_with_ui_context` in `call_agent.rs`; UI: `stores/activeItem.svelte.ts` | Accept `app` and `item` alongside the document fields, and say all three in the prompt ("in Agents, on agent acme/triage-bot, its Configuration tab"). | partial |

### Project environments (the Projects → Environments tab is built on sample data)

The tab shows what each environment should hold: which version of each flow and
agent runs where, and the variables each environment gives its deployments. It
is marked **Sample** and its edits are not saved until these land. Design:
`.ai/A3T_DASH_PROJECTS_ENVIRONMENTS.md` (D2).

| # | Missing | Where | Needed | Status |
|---|---|---|---|---|
| SRV-5 | Store a project's environments and their variables. The project record has an `environments` column (default `["dev","staging","prod"]`), but `update` never writes it, and there is nowhere for variables. | `a3t/backend/flows/a3t/a3t_projects.yaml` (`update-write`) | `environments: [{ name, order, vars: [{ name, value \| secret_ref }] }]` saved by `update` and returned by `list`; a secret is a reference to a Secret resource, never a value | open |
| SRV-6 | A canvas deploying into an environment does not pass that environment's variables on. Pod env is the runtime's defaults plus a fixed set of extras. | `flowr/crates/flowr-canvas/src/reconciler.rs` (`apply_sandbox`, `extra_env`); the canvas CRD | Resolve the canvas's environment, read the project's variables for it, put them in the CRD, and from there in the pod's `env` (secrets as `valueFrom.secretKeyRef`) | open |
| SRV-7 | Nothing says which version runs in which environment. | `/ops/overview` deployments; runtime labels | Each deployment carries its environment (from the runtime's `a3t.io/env` label) and the version it pinned, so the UI can draw the flows × environments matrix from live data | open |
| SRV-9 | Billing has no backend: no billing accounts, no per-project metering, no budgets. Settings → Billing is controls only. The Projects → Billing tab (and the billing account in the create wizard) is built on sample data and marked Sample. | Org/billing service; metering in the engine (LLM call task tokens, run counts, runtime hours, storage) | Billing accounts per org (and personal), each project linked to one (+ cost centre, billing contact); usage and cost per project per month, by agent/flow; a monthly budget with a warn threshold and an action at the limit (notify, or pause non-prod), enforced by the controller | open |
| SRV-10 | A template can't be tried on its own. It has no endpoint; Templates → Try it runs through an agent built from it (`/agent/{id}/chat`), which accepts a model and tools per request but not a prompt or input-contract change. Test cases can't be saved. | Agent chat endpoint (`call_agent`); template store | Run a template directly with a given input, model and prompt (a dry-run of its workflow, no agent record); accept a `system_prompt` override per request; store test cases (input, expected output) with the template and run them as a suite | open |
| SRV-8 | A project can't move between organisations: `move` takes only a personal project into an org. | `a3t_projects.yaml` (`move-allowed`) | Allow an org's project when the caller is admin in both orgs (or superadmin); the rest of `move` (namespace rewrite, references) already applies | open |

### Agents as repos and as APIs (Agents → Releases is partly sample data)

Agents are made in two ways only — **clone** (an independent copy) or **fork**
(a copy that keeps its link to the parent) — and changed on a **dev branch**
that is merged into prod. Spec: `.ai/A3T_AGENTS_AS_REPOS_SPEC.md`. Clone and
fork work today with what exists; branches, merges and releases are **Sample**.

| # | Missing | Where | Needed | Status |
|---|---|---|---|---|
| SRV-11 | Lineage and description have no columns. The UI keeps `clonedFrom` / `forkedFrom` / `fromName` and `description` inside the agent's `variables` JSON. A fork cannot take its parent's later changes; clone/fork cannot bring memory. | `a3t/backend/flows/a3t/agents_crud.yaml` (`create-insert`, `update`), `agent_configs` table | `description`, `parent_id`, `parent_kind` (`clone` \| `fork`) columns written by create/update and returned by list; "changes since fork" + "take parent's changes" (diff of behaviour fields); an option to copy memory (namespace copy in the org ADB shard). Then the UI reads the columns and stops writing them into `variables`. | open |
| SRV-12 | No branches, merges or releases for an agent. | agents-as-repos spec phases 1–3 (row-level overlay branching in ADB, sri-dev-ea) | Branch = overlay of the agent's config and memory in the dev environment; merge = new release of prod (behaviour replaces, procedures merge into memory); releases tagged and rollback-able; endpoints per environment. UI: `components/agent/agentBranches.svelte.ts` is the sample to replace. | open |
| SRV-13 | Deleting an agent removes only its row. The UI then deletes its endpoint trigger and its own canvas; its memory and decision history stay. | `agents_crud.yaml` (`delete`) | Server-side delete that refuses while deployed and removes the trigger, own canvas and memory namespace in one step (or reports what is left). | open |
| SRV-14 | **An agent is not resolved on the server when it is called.** `POST /chat` (`chat_ui_stream_handler`, `lib.rs`) passes only `message`, `session_id`, `llm_resource`, `enabled_tools`, `mcp_servers`, `context`, `ui_context` to its flow (default `a3t/a3t-chat-engine`). The webhook path passes the body as sent. The server reads `agent_configs` only for the owner check (`resolve_agent_owner`). So the agent's model, tools and allow-list apply only because the caller sends them (A3T's own chat does), and its saved **instructions** are sent by no caller. Whether `a3t-chat-engine` loads them is not in the repo (registry only), so that needs confirming. | `lib.rs` `chat_ui_stream_handler`; webhook trigger → serving flow; `a3t-chat-engine` (registry) | Resolve the agent from the session prefix / endpoint on the server and apply its record (model, instructions, tools, allow-list, memory mode). Ignore or reject those fields from callers, so an app cannot widen an agent's tools. The API tab then drops the "sent by the caller today" rows. | open |
| SRV-15 | No API for apps: a call starts a run and the app polls `get_execution_status` over MCP JSON-RPC; auth is a user's token; no per-app keys, allowed origins (CORS), release pinning or per-environment endpoints; calls carry no calling-app identity (so Observe can't say which app called). | Agent endpoint / `/chat`; auth | `POST /agents/{name}/messages` answering with the reply, or streaming it (SSE, AI SDK UI-message compatible); app keys (per app, rate-limited, revocable, recorded on each run); allowed origins per agent; `?release=` and dev/prod endpoints. | open |

UI-14 (edits to the open document without a permission prompt) is also on the
server side, and matters more now: the Studio's assistant is the Chat beside the
Explorer, which uses `propose_edit` for every document edit (ce1461a).

## UI changes made with this (no backend change)

- Agent pop-in (existing agent), for the coder who ships it: Overview · API · Config · Releases · Observe · Settings (8 Nov, branch `a3t-dash-agents`). API: endpoint, request schema from the template's `input`, TS/Python/curl for today's call path (webhook + poll), SRV-14/15 stated. Config: the agent's values as YAML (`components/agent/agentValues.ts`) over its template's code, read-only. Releases: prod's real deployment (deploy / pause / resume / undeploy), dev branches, releases and per-environment values as Sample. Observe: health, calls, errors, test chats, blocked, memory. Clone / Fork from Overview; Delete in Settings (undeploy first). Earlier tab names still deep-link.
- Chat home: five ways in (What is A3T? · Create an agent · Explore an agent · Review security · Check health).
- "Logs" → "Decision log", filterable to an agent (`navigateTo('logs', …, { q: agentId })`).
- Memory page: the made-up sample agents are gone; it lists real agents and opens each one's Memory tab. Added to the menu.
- Identity: an unreachable SSO provider reads as what it means for sign-in, with the technical error behind "Technical detail".
- Chat: an agent artifact opens the agent's page ("Open agent"); the Studio stays one click away.
- Deep links: `navigateToDetail('agents', 'Agents', id, { tab: 'overview' | 'api' | 'config' | 'releases' | 'observe' | 'settings' })`; older names (memory, policy, health, canvas, deploy, branches, activity, ownership, …) still land on the matching tab.
