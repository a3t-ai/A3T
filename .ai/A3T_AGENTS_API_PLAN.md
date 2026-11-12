# A3T agents over API: plan

*12 Nov 2026. Concepts: https://claude.ai/artifact/DP81mLYzobNAuokycHSfcx (Details pane only; the shell is locked).*

**Goal:** A3T provides agents over an API, as Stripe does for payments and Fly.io for
infrastructure.
- **Creating an agent:** instantiate a template, or clone or branch an existing agent.
- **Relationship to the source:** an agent either inherits it (the parent's hooks fire,
  data goes back) or forks it (nothing goes back).
- **Bringing a fork back:**
  - **merge** if only procedural memory changed (ADB's sequencer applies it to main);
  - **evolve** otherwise: the fork becomes a new agent.
- **Sharing:** a namespace shares an agent with other namespaces, or makes it public.
- **URLs:** every agent gets a control-plane URL (config, runtime) and a data-plane URL
  (chat, SSE).

## What exists (checked 12 Nov)

| Area | Where | What |
|---|---|---|
| Core agent | `system/agent-engine` 2.1.0 (vc-dev) | Errors, logging, docs and tool recall; hooks inert until AH-1..4 |
| Agent records | `a3t/a3t-agents` (agents_crud) | list, get, create, update, delete, events on `agent_configs` |
| Templates | `a3t/a3t-templates` (templates_crud) | list, get, create, update, delete on `template_registry` |
| Memory space | FlowR REST `agent_space.rs` | `/agents/{id}/memory`, `branches` (+promote, conflicts, merge, rebase), `fork`, `transfer`, `tags`, `layers`, `versions`, `deployments`, `runs`, `usage` |
| Merge requests | FlowR REST `merge_requests.rs` | create, get, resolutions, approvals, evidence, apply, rollback |
| Data plane | FlowR REST | `POST /chat` (UI message stream), `GET /chat/stream/{session}` (SSE), `POST /webhook/agent/{id}/chat` |
| Sharing | `a3t/a3t-share` | Publish or clone flows, resources and triggers between projects; not agents |

## What is missing, and who builds it

### A3T side (me): flows, tables, UI

1. **Table `agent_lineage`.** One row per agent:
   - `agent_id`, `parent_id`, `relation` (`template|inherit|fork|branch|clone|evolved`),
     `source_version`, `created_at`;
   - a lineage query walks it up and down.
2. **Table `agent_shares`.** One row per agent and target:
   - `agent_id`, `target` (namespace, org, or `*` for public);
   - rights `call`, `inherit`, `fork`, `merge_back`; `granted_by`, `granted_at`.
3. **`a3t/a3t-agents` 2.0, the control plane.** New actions over the same flow:
   - `create {from: template:<id>@<v> | agent:<id>[@branch], mode: inherit|fork|clone|branch, values}`.
     It writes `agent_configs` and `agent_lineage`, creates the memory space (branch: an
     ADB branch; fork and clone: `/fork`), and returns `{id, control_url, data_url}`;
   - `get`: the record, its endpoints and its lineage summary;
   - `runtime`: deployments, runs, usage and health, gathered from the FlowR REST routes;
   - `lineage`: ancestors and descendants;
   - `compare {fork}`: which memory types and config fields differ from the source
     (procedural only means it can merge);
   - `merge {fork}`: a merge request on the source, through FlowR merge requests
     (the sequencer applies it);
   - `evolve {fork}`: re-parents the fork as an `evolved` agent;
   - `shares`, `share {target, rights}`, `unshare`, `visibility {private|namespaces|public}`.
4. **`a3t/a3t-templates` 2.0.** Adds `instantiate` (same as agents `create` from a
   template) and `versions`. The core template `tpl-agent-engine` is listed first and
   can't be deleted.
5. **The data-plane contract.** Each agent's data URL is its existing chat path, and
   the docs describe the SSE events. One flow, `a3t/a3t-agent-endpoints`, returns both
   URLs for any agent, so the UI and the API agree.
6. **UI, Details pane only:**
   - agent tabs: Overview · API · Hooks · Lineage · Sharing · Access · Controls · Audit · Settings;
   - the New agent screen;
   - the shell is not touched.
7. **Tests.** A harness in the style of `core-agent.mjs` that runs:
   - each create mode;
   - lineage, compare, merge and evolve;
   - share and visibility;
   - the endpoint URLs answering.

### FlowR / ADB side (requests, not mine)

- **AH-1..4:** hooks run, run flows, can change the turn, and chain through inheritance.
  Without them, "inherit" records the parent, but the parent's hooks don't fire.
- **Per-agent data-plane hostnames** (`agt-xxxx.run.a3t.io`): gateway routing to the
  agent's chat path. Until then the data URL is the console's path.
- **"Data goes back" on inherit:** a child's turns and procedures written to the
  parent's space as well. This is an ADB or FlowR memory-space feature: child space →
  parent space through the sequencer.
- **Inheritance checks across namespaces:** FlowR checks share rights (`inherit`,
  `fork`, `merge_back`) when another namespace calls `/fork` or opens a merge request.

## Order

1. Tables and `a3t/a3t-agents` 2.0 control-plane actions, then the tests.
2. `a3t/a3t-templates` 2.0 and the endpoints flow.
3. UI: Overview and New agent, then Lineage, Sharing, Hooks and API.
4. Roll to vc-dev: flows and schema need your OK; the UI goes to dash.dev.
