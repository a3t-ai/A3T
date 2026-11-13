# Trigger mappings written by an A3T agent, approved by a person

*13 Nov 2026. Status: **built, dev time** (live on vc-dev, flowr ee-ag10).*
*The user, 13 Nov: "A trigger input to flow/agent mapper could be an agent and mapping could be a
perfect procedural memory learned." — "That should be an A3T agent. And think of how user cared
about the generated mappings and approvals of mapping script." — "It can be dev time not runtime."*

## The shape

```
dev time   Send tab ─ samples ─▶ agent_a3t_mapper (A3T agent, /chat) ─▶ proposed jq
                                   │
                                   ▼
           FlowR POST /triggers/mapping/check: the jq on each sample, the trigger's own
           event and engine, each result against the flow's input.schema
                                   │
           a person: Approve and use │ Change it first │ Reject (reason)
                                   ▼
           the jq written into the trigger (saved); a record kept, append-only, as
           PROCEDURAL memory of agent_a3t_mapper
run time   the trigger fires ─▶ the approved jq ─▶ the flow / agent    (no agent, no LLM)
```

## What a person cares about, and where it is

| Concern | Answer |
|---|---|
| What will run? | The jq itself, with the mapper's explanation, its assumptions and what it leaves out (e.g. `customer`) |
| Is it right? | Run on the sample events by FlowR's own engine, each result checked against the flow's `input.schema`; **Approve is off until every sample passes** |
| Who decided? | Each proposal / approval / rejection is a record: who, when, version, how many samples, the reason for a rejection |
| Is what runs what was approved? | The field's pill: *Approved vN · by · when*, *Changed since approval vN*, *Proposal waiting for review*, *Hand-written · not approved* |
| Can I go back? | History, newest first, with *Use this version* (then check and approve again) |
| Where do my samples go? | To the mapper's model — said where samples are added and where the mapper is asked; the samples themselves are not stored, only their count |
| Hand-written mappings? | Same path: *Check on samples*, then *Approve this mapping* (recorded as hand-written) |
| Cost and latency at run time | None: only the approved jq runs |

## Pieces

- **FlowR** `POST /triggers/mapping/check` `{ expr, samples[], field: input|message|conversation, target?: ns/flow, trigger?, source?, message? }`
  → `{ all_ok, results[{ ok, value | error, schema_errors[] }], schema, schema_note }`. The event is built by
  `triggers::event_of` and shaped by `triggers::shape_for` — the same functions delivery uses.
- **agent_a3t_mapper** — a platform agent (system/flow_system_agents_install 1.0.6), from `tmpl_a3t_agent`, shared;
  replies with only `{jq, explanation, assumptions, drops}`.
- **Records** — `agent_memory` PROCEDURAL of agent_a3t_mapper, `kind: trigger_mapping`, `event: proposed|approved|rejected`,
  `trigger_id, target (flow:ns/name | agent:id), field, jq, version, by, at, source (agent|hand)`.
- **UI** — `components/triggers/TriggerMapping.svelte` (one per mapped field), `mapperApi.ts`; the Send tab's *Sample events* card;
  Test's *Keep as a sample*.
- **Tests** — `a3t/backend/tests/e2e-triggers.mjs` (the check endpoint), scratchpad `mapui.mjs` (the whole review, with a real firing).

## Not done (say if wanted)

- **Approval is recorded, not enforced**: a mapping edited in the YAML or the Studio still runs; the pill shows it as changed or
  not approved. Enforcing it ("only approved mappings run", per project) would be a FlowR check at save time.
- **Four eyes**: the person who asks can approve. A rule that someone else must approve is a policy option.
- **Rights**: records are written to agent_a3t_mapper's memory as the signed-in user; tested as a super admin. A user without
  rights on that agent's memory would see "approval was not recorded" — a records flow with platform rights would fix that.
- **History depth**: the agent's memory read returns its newest 100 procedural entries.
- **Drift**: a firing whose mapping fails is answered 422 with the reason, but not yet flagged on the trigger for a re-ask.
