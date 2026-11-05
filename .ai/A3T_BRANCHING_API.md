# A3T memory branches and agents: API and message reference

5 Nov 2026. For whoever builds the console and Studio screens for branch, fork, merge
(promote), tags, layers and agents. Design: `.ai/A3T_AGENTS_AS_REPOS_SPEC.md`; ADB
build notes: `ADB/.ai/BRANCHING.md`.

## 1. Overview

An agent's memory is a **space**: one ADB namespace, `<project>--<agent>`. Branch, fork
and merge act on spaces and today exist **only in ADB**, as Arrow Flight actions. FlowR
has no HTTP or MCP endpoints for them yet, so the console can't call them directly
(§6 lists the planned FlowR endpoints; §8 the status).

| Term | What it is | ADB action |
|---|---|---|
| Space | An agent's memory, one namespace. Its own line is `main`. | none (implicit) |
| Branch | A named, isolated line cut from `main` (or another branch) at a point. Reads see the parent as of that point plus the branch's own writes. Nothing is copied. | `branch_create`, `branch_list`, `branch_drop` |
| Fork (clone) | A new space filled with a copy of another space's current records. | `space_clone` |
| Merge (promote) | A branch's view copied into a new space, which becomes a new agent. The original agent is not changed. | `branch_promote` |
| Tag | A named point of a space, for pinning and for keeping old versions. | `tag_create`, `tag_list`, `tag_drop` |
| Layer | Another space a space reads through, current or at a tag, optionally sealed. | `layers_set`, `layers_get` |

**Calling convention.** Each action is a Flight `do_action` whose type is the action name
and whose body is JSON. The answer is one JSON result. A refusal is a gRPC error
(`INVALID_ARGUMENT`, `FAILED_PRECONDITION` or `INTERNAL`) whose message is a readable
sentence (§7). Statements (STORE, RECALL, FORGET, …) go through the `aql` action,
`{query, variables?}`, which answers `{success: true, data}` or `{success: false, error}`.

**Record ids carry the namespace** (ADB schema v5): `acme--bot|~user:alice`. The
payload's `key` stays bare (`user:alice`); show `data.key`, not `id`.

## 2. Branch

A branch is cut at the current log position and copies nothing. It has its own engine
(log, sequencer, table), so its writes never reach the agent's `main`.

| Action | Request body | Answer |
|---|---|---|
| `branch_create` | `{namespace, name, parent?, created_by?}`; `parent` defaults to `main`, or names another branch of the same namespace | the Branch object |
| `branch_list` | `{namespace?}`; omit for every space | `{branches: [Branch]}` |
| `branch_drop` | `{namespace, name}` | `{dropped: true \| false}` |

Branch object:

```json
{
  "id": "br:acme--bot/try-new-recall",
  "namespace": "acme--bot",
  "name": "try-new-recall",
  "parent": "main",
  "point_offset": 2287,
  "point_ts": 1793836800000000,
  "created_by": "alice@acme.com",
  "placement": "http://adb-br-acme-bot-try.a3t:50051",
  "pending_forgets": []
}
```

- `point_ts`: commit time of the branch point, in microseconds. The branch reads its
  parent as of this point.
- `placement`: present only when the branch runs in its own pod; absent = hosted in the
  org's ADB.
- `pending_forgets`: present only while FORGETs on `main` wait for a stopped branch pod.

Names: 1–64 letters, digits, `-`, `_` or `.`; `main` is reserved. Drop deletes the
branch's engine and data, and is refused while the branch has branches of its own.

### Reading and writing on a branch

Add `BRANCH "<name>"` to any statement sent through `aql`. It needs `NAMESPACE`, because a
branch belongs to one space. Without `BRANCH`, a statement runs on `main`.

```
RECALL FROM SEMANTIC WHERE subject = "acme" NAMESPACE "acme--bot" BRANCH "try-new-recall" LIMIT 5
STORE INTO EPISODIC (event = "tried new prompt") NAMESPACE "acme--bot" BRANCH "try-new-recall"
```

- Reads: the parent as of the branch point, the branch's writes on top, minus records the
  branch changed or forgot.
- `UPDATE` of a parent record on the branch writes a branch version; the parent keeps its
  own.
- `FORGET` on `main` also erases the records from every branch (compliance).

### Placement

| Placement | Where the branch engine runs | How it is set |
|---|---|---|
| Hosted (default) | In the org's ADB, under `<data>/branches/<ns>__<name>` | nothing to do |
| Own pod | A separate ADB process that reads the parent over Flight | `branch_place` (§5) |

Callers don't change anything for a pod branch: the org forwards `BRANCH` statements to
the pod.

## 3. Fork (clone) and tags

A fork copies a space's current `main` records into a new, empty space. Unlike a branch
it depends on nothing afterwards: a full copy at the time of the call.

| Action | Request body | Answer |
|---|---|---|
| `space_clone` | `{namespace, into}` | `{namespace: <into>, records: n, from: {namespace, branch: "main"}}` |
| `tag_create` | `{namespace, name, created_by?}`; marks the space's current point | the Tag object |
| `tag_list` | `{namespace?}` | `{tags: [Tag]}` |
| `tag_drop` | `{namespace, name}` | `{dropped: true \| false}` |

Tag object:

```json
{ "id": "tag:acme--compliance/v1", "namespace": "acme--compliance", "name": "v1",
  "point_offset": 2301, "point_ts": 1793836900000000, "created_by": "ops@acme.com" }
```

- Copied: SEMANTIC, PROCEDURAL and TOOLS current records, each as a new first version in
  `into`. Not copied: EPISODIC history, WORKING memory, links.
- `into` must hold no records, or the call is refused.
- Compaction keeps every version a tag points at. A tag can't be dropped while a layer
  pins it.
- Not built: cloning from a tag or a past point. A clone always reads `main` now.

## 4. Merge (promote)

There is no merge back into the parent. **Merge = promote**: the branch's view becomes a
new agent space in production; the original agent is untouched; traffic moves to the new
agent by deployment.

| Action | Request body | Answer |
|---|---|---|
| `branch_promote` | `{namespace, name, into}` | `{namespace: <into>, records: n, from: {namespace, branch: <name>}}` |

- Copied: SEMANTIC, PROCEDURAL and TOOLS records as the branch sees them (parent at the
  branch point + the branch's changes − what it forgot). EPISODIC history stays with the
  original agent; WORKING is never copied; links are not copied.
- `into` must be a new, empty namespace, never the source. Written through the org's
  sequencer, 500 records per log record, in one call (no progress, no resume).
- Works the same for a branch in its own pod (the org reads the view from the pod).
- The branch is kept; drop it with `branch_drop` when done.

UI flow "promote this branch as a new agent":

1. Choose the new agent's namespace, e.g. `acme--bot-v2`.
2. `branch_promote {namespace: "acme--bot", name: "try-new-recall", into: "acme--bot-v2"}`.
3. Create the agent pointing at `acme--bot-v2` (§6).
4. Optionally `branch_drop` the branch.

## 5. Layers

A space can read other spaces as layers, e.g. a shared compliance space every trading
agent inherits. One level deep: a layer can't have layers itself.

| Action | Request body | Answer |
|---|---|---|
| `layers_set` | `{namespace, layers: [Layer]}`; replaces the whole list, `[]` removes all | the Layers object |
| `layers_get` | `{namespace}` | `{layers: Layers \| null}` |

```json
{
  "id": "layers:acme--trading",
  "namespace": "acme--trading",
  "layers": [
    { "space": "acme--compliance", "mode": "pin", "tag": "v1",
      "memories": ["SEMANTIC", "PROCEDURAL"], "sealed": true }
  ]
}
```

| Field | Values |
|---|---|
| `mode` | `mount`: the layer space now; `pin`: at `tag` |
| `tag` | Required for `pin`; an existing tag of `space` |
| `memories` | Any of `SEMANTIC`, `PROCEDURAL`, `TOOLS`; never `EPISODIC` or `WORKING` |
| `sealed` | `true`: the space can't write or shadow a key the layer holds |

**Reads**, per key, first match wins: (1) sealed layers, (2) the space's own records,
(3) unsealed layers. Layer records come back with ids in the reading namespace. RECALL
and LIKE see layers; FOLLOW LINKS, history and PATTERN reads don't yet.

**Writes**: a STORE or FACT on `main` whose key a sealed layer holds is refused. A
FORGET in the layer space reaches every space that reads it.

## 6. Agents (FlowR)

The UI manages agents through FlowR, not ADB. Agent CRUD exists; agent-level branch,
fork and promote endpoints don't, so build against the planned shapes below.

### Today: the `a3t-agents` flow

`execute_flow` on `/message` (signed in), `{name: "a3t-agents", namespace: "a3t", input}`;
see `agents` in `a3t/ui/src/lib/services/a3t.ts`.

| `input.action` | Other input fields |
|---|---|
| `list` | `org_id` |
| `get` | `id`, `org_id` |
| `create` | `org_id`, `user_id`, `timestamp`, `data: {id, name, template_id, template_version, namespace, mode, variables, resource_bindings}` |
| `update` | `id`, `org_id`, `timestamp`, `data` (fields to change) |
| `delete` | `id`, `org_id` |

An agent's memory space is `<project>--<agent>`: agent `support-bot` in project `acme`
uses `acme--support-bot`. That is the `namespace` in every ADB call above.

### Planned: agent branch endpoints (not built)

Spec §4.7: REST under `/agents/<id>/branches` plus the same as MCP tools. FlowR resolves
the agent to its namespace and calls the ADB action; bodies and answers are ADB's.

| Method and path | ADB action | Body |
|---|---|---|
| `GET /agents/{id}/branches` | `branch_list` | none |
| `POST /agents/{id}/branches` | `branch_create` | `{name, parent?}`; `created_by` = the signed-in user |
| `DELETE /agents/{id}/branches/{name}` | `branch_drop` | none |
| `POST /agents/{id}/branches/{name}/promote` | `branch_promote` | `{new_agent: {id, name}}`; FlowR derives `into` and creates the agent |
| `POST /agents/{id}/fork` | `space_clone` | `{new_agent: {id, name}}` |
| `GET` / `POST /agents/{id}/tags`, `DELETE /agents/{id}/tags/{name}` | `tag_list`, `tag_create`, `tag_drop` | `{name}` for create |
| `GET` / `PUT /agents/{id}/layers` | `layers_get`, `layers_set` | `{layers: [Layer]}`, `space` given as an agent id |
| a run with `memory_branch` | `BRANCH` on every statement | run field (spec §4.3) |

Cedar actions (planned): `memory:branch`, `memory:promote`, `memory:fork`, `memory:tag`,
`memory:mount`, `memory:pin`.

## 7. Branch pod protocol (internal)

Between the org ADB and a branch pod, or from the A3T controller. A UI never calls
these; it only shows their effect (`placement`, `pending_forgets`).

| Action | From → to | Request body | Answer |
|---|---|---|---|
| `branch_place` | controller → org | `{namespace, name, url}`: the pod's Flight URL | Branch, with `placement` |
| `branch_parent_rows` | pod → org | `{memory, namespace, branch?, at}`; `at` = branch point, µs | `{rows: [StoredRow]}` current at `at` |
| `branch_forget` | org → pod | `{memory, lineages: [id]}` | `{erased: n}` |
| `branch_pending` | pod → org | `{namespace, name}` | `{forgets: [{memory, lineages}]}` |
| `branch_pending_ack` | pod → org | `{namespace, name, count}` | `{acked: n}` |
| `aql` | org → pod | `{query, variables?}`: forwarded `BRANCH` statements | `{success, data}` |

Lifecycle of a pod branch:

1. `branch_create` on the org.
2. Start a pod from `ADB/deploy/k8s/adb-branch.yaml` with `ADB_BRANCH_OF=<org Flight URL>`,
   `ADB_BRANCH=<namespace>/<name>`, `ADB_BRANCH_TOKEN`.
3. `branch_place`. Only a branch with no hosted engine yet and no child branches.
4. On every start the pod reads its Branch, applies `branch_pending`, acks, then serves.
5. Hibernate = scale to 0; FORGETs on `main` meanwhile wait in `pending_forgets`.
6. `branch_drop` removes only the row of a pod branch; delete the pod's volume for its data.

## 8. Errors, status and limits

| Message (pattern) | Cause |
|---|---|
| `namespace "<ns>" already has a branch "<name>"` | Name taken |
| `branch name "<name>": use 1–64 letters, digits, '-', '_' or '.'` | Invalid name |
| `no branch "<name>" in namespace "<ns>"` | Unknown branch, or `BRANCH` in the wrong namespace |
| `BRANCH "<name>" needs NAMESPACE: a branch belongs to one memory space` | Statement missing `NAMESPACE` |
| `branch "<name>" has branches of its own: drop them first` | Drop of a branch with children |
| `namespace "<into>" already holds n records: promote into a new one` | Promote or clone target not empty |
| `promote into a new namespace, not "<ns>" itself` | Promote into the source |
| `tag "<name>" of "<ns>" is pinned by "<space>": change its layers first` | Tag drop while pinned |
| `"<space>" has layers of its own: layers are one level deep (B7)` | Nested layers |
| `a layer carries SEMANTIC, PROCEDURAL or TOOLS, not EPISODIC` | Wrong memory type in a layer |
| `<key> belongs to the sealed layer "<space>" of "<ns>"…` | Write that would shadow a sealed layer |

Status, 5 Nov 2026:

| Piece | State |
|---|---|
| ADB: branch, fork, promote, tags, layers, pod protocol | Built; on vc-dev (`adb:2026-11-05-b7`) |
| FlowR agent branch endpoints and MCP tools (§6) | Not built |
| Branch runs (`memory_branch`) and the gateway side-effect guard | Not built |
| A3T controller: create, hibernate, route branch pods | Not built |
| Agent config fields `memory_space`, `layers` | Not built |

Limits: no branch of a pod branch; a hosted branch can't move into a pod; FOLLOW LINKS,
history and PATTERN reads don't see layers; AQL `MERGE` (entity merge) can't reach
records in a namespace.
