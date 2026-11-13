# Trigger mapping learned by an agent — kept as procedural memory (parked)

*13 Nov 2026. Status: **parked, design only**. The user, 13 Nov: "A trigger input to
flow/agent mapper could be an agent and mapping could be a perfect procedural memory learned."*

## The idea

Today each trigger target has a hand-written jq mapping from the event
(`{ body, message, source, trigger, time }`) to the target's input. Instead, a **mapper
agent** writes that jq once, from real events and the target's input schema. The jq
it settles on is kept as **procedural memory**, a learned "how to". After that, every
firing runs the stored jq: deterministic, cheap, no LLM in the hot path. The agent is
asked again only when the stored procedure stops fitting.

```
event ─▶ procedure for (trigger, target, event shape)?
           ├─ yes ─▶ run its jq ─▶ input valid for the target? ─ yes ─▶ run the flow / agent
           │                                                   └ no ──┐
           └─ no ─────────────────────────────────────────────────────┴▶ mapper agent:
                 event + target input schema + past procedures ─▶ proposes jq
                 ─▶ checked: runs on this event and the kept samples, output valid for the schema
                 ─▶ stored as a new procedure version (approve first, if the trigger asks)
                 ─▶ the run goes ahead with it
```

## Why procedural memory fits

- It is a *skill*, not a fact: "to turn a Shopify refund webhook into
  `flow_default_refund`'s input, do X". It belongs to the agent and is reused for every
  trigger with the same event shape and target.
- It can be checked: a procedure is right when its output validates against the
  target's input schema, for every kept sample. That is what "perfect" means here, and
  it is tested, not assumed.
- It is versioned like everything else (agents as repos): each relearn is a new
  version, with the samples that made it and the event shape it is for. A rollback is
  a version pick.

## In the YAML

```yaml
send:
  - flow: default/flow_default_refund
    input: learn                         # the mapper agent writes it, kept as a procedure
    mapper: agent_default_mapper         # optional; default: the platform's mapper agent
    approve: first                       # first | every_change | never (default first)
  - agent: agent_default_support
    message: '${ "Refund for " + .body.order_id }'   # hand-written still works, unchanged
```

`input: learn` (or `message: learn`) is the only new thing a trigger author writes.
The learned jq shows in the trigger's Send step as read-only text, with "pin it":
copy the jq into the YAML, which turns learning off.

## Pieces

| ID | Work | Notes |
|---|---|---|
| LM-1 | Target input schemas: a flow's `input.schema` (SWF), an agent's message (string) | Without a schema, "valid" means only "the jq ran"; ask for one |
| LM-2 | Procedure store: `agent_memory` kind `procedure`, key `(target, event-shape hash)`, value `{jq, version, samples[], schema hash, approved_by}` | The event-shape hash is over the body's key paths and types, not its values |
| LM-3 | Mapper agent (`agent_a3t_mapper`, from `tmpl_a3t_agent`): tool `propose_mapping(event, schema, prior)` returns jq; the platform checks it | The agent never runs the target; it only writes jq |
| LM-4 | Trigger delivery: `learn` targets look up the procedure; on a miss or an invalid output, call the mapper, check, store, then deliver | One relearn per shape at a time (a lease), so a burst does not ask the agent N times |
| LM-5 | Approval: `approve: first` holds the run until a person accepts the first procedure (Inbox), `every_change` on each new version | Held runs are delivered once approved (the firing's `_dedupe` key still holds) |
| LM-6 | UI: Send step shows the learned jq, its version, its samples and "pin it"; the agent's Memory › Procedural lists every procedure it learned | |
| LM-7 | e2e: a webhook with `input: learn` to a flow with an input schema; first call learns and runs; second call runs with no agent turn; a body with a new shape relearns | |

## Open questions for the user

- The default for `approve`: `first` (safe) or `never` (fast)?
- One platform mapper agent per org, or each team's own (their own model and policy)?
- Should a learned procedure be shared across triggers in a project with the same
  event shape and target? (Recommended: yes, it is the same skill.)
