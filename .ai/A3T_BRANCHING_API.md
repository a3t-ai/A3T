# A3T memory branches and agents: API and message reference

5 Nov 2026. For whoever builds the console and Studio screens for branch, fork, merge
(promote), tags, layers and agents. Design: `.ai/A3T_AGENTS_AS_REPOS_SPEC.md`; ADB
build notes: `ADB/.ai/BRANCHING.md`. §9 gives the user flows per persona, §10 what good
looks like, §11 the APIs those flows still need (BR-n).

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

## 9. UX: user flows per persona

The console has four personas (`.ai/A3T_UI_LIMITATIONS.md`, 3 Nov): the learner or agent
owner, the developer, the security reviewer, and the SRE. They share the same screens and
differ only in where they start. Branching adds no persona-specific screens. It adds one
control to the agent hub, a few dialogs, and a branch scope that every existing tab
honours.

### 9.1 Screens

| # | Screen | What it shows |
|---|---|---|
| S1 | **Branch switcher** in the agent header: `main ▾` | Branches of the agent, newest first; "New branch…"; "Compare with main" |
| S2 | **Branch scope** on every agent tab (Overview, Activity, Memory, Chat, Health) | A coloured banner: "Branch `try-new-recall` · cut 2 h ago from main · isolated: nothing reaches production, outbound calls blocked". The URL carries `?branch=` so a link opens the same view |
| S3 | **Agent → Versions** tab: Branches · Tags · Layers | One list per kind, with owner, age, size and placement (hosted / own pod / hibernating); row actions |
| S4 | **Compare** (branch vs main) | Memory diff by type (added, changed, forgotten), main's changes since the branch point on the same keys, and the flow diff once flows branch too |
| S5 | **Promote dialog** | New agent name, what is carried (SEMANTIC, PROCEDURAL, TOOLS) and what stays behind (EPISODIC, WORKING, links), then progress and a link to the new agent and its Deploy tab |
| S6 | **Fork dialog** (agent ⋯ menu → "Fork agent") | New name, a "copies N records now" estimate, what is not copied |
| S7 | **Inherited memory** on Controls | Layers this agent reads (mount / pin@tag, sealed, memory types) and, on a layer space, the agents that read it |
| S8 | **Lineage** on Overview | "Forked from X on 5 Nov", "Promoted from X / try-new-recall", "Template crawl v3, upgrade v4 available", "Inherits compliance@v1 (sealed)" |

### 9.2 Learner or agent owner: try a change safely

Starts from chat home ("Try a change without touching production") or the agent's S1.

| Step | What the user does and sees | API | Status |
|---|---|---|---|
| 1 | Opens S1 → "New branch…". The name is suggested (`try-5nov`) and checked as they type; parent is `main` | `POST /agents/{id}/branches` | planned (§6) |
| 2 | Lands on the agent in branch scope (S2). The banner says what isolation means in one line | branch object | planned |
| 3 | Chats with the agent on the branch | chat / run with `memory_branch` | **BR-4** |
| 4 | Memory tab shows only what the branch learned ("+12 facts, 2 changed, 1 forgotten") and lets them toggle "branch only / everything the branch sees" | memory read on a branch | **BR-14** (UI-1 + branch) |
| 5 | Compare (S4): plain language first ("The branch knows 12 new things and changed 2"), detail on demand | branch diff | **BR-2** |
| 6a | Keeps it: Promote (S5) → new agent → Deploy tab to move traffic | `POST …/promote` | planned; **BR-5**, **BR-7** |
| 6b | Throws it away: "Discard branch", one confirm, says what is deleted | `DELETE …/branches/{name}` | planned |
| — | Forks someone's agent to own a copy (S6) | `POST /agents/{id}/fork` | planned |

### 9.3 Developer: branch, run, diff, promote (Studio)

Starts from Agent → Code ("Open in Studio") or the CLI (`a3t agent branch …`).

| Step | What the user does and sees | API | Status |
|---|---|---|---|
| 1 | Studio Versions panel lists branches beside flow versions; "Branch from here" on any version or tag | `branch_list`, `tag_list` | planned |
| 2 | Runs and debugs with a run target selector `memory: main ▾ / try-new-recall`; steps that would call out show "blocked on branch" or "mocked" | run with `memory_branch`; side-effect classes | **BR-4**, **BR-6** |
| 3 | Edits flows on the branch: draft flow versions held under the branch | flow branch (registry versions) | **BR-6** |
| 4 | Split-view diff: flows on the left, procedures and tools on the right, conflicts flagged ("main changed `check_trade` since your branch point") | branch diff with parent changes | **BR-2** |
| 5 | Rebase when main moved: "Main has 40 changes since your branch point; 3 touch keys you changed" → Rebase | rebase | **BR-9** |
| 6 | Promote → new agent → canvas Deploy; the old agent keeps serving until traffic moves | promote + create agent | planned; **BR-5** |
| 7 | Tags the release (`v2`) so its memory point is kept for audit and rollback | `POST /agents/{id}/tags` | planned |
| 8 | Drops merged or stale branches; the list nudges branches untouched for 14 days | `branch_drop`; last activity | planned; **BR-3** |

### 9.4 Security reviewer or layer owner: inherit and govern

Starts from chat home ("Review security") or agent → Controls.

| Step | What the user does and sees | API | Status |
|---|---|---|---|
| 1 | Builds a shared space (a compliance agent), tags it `v1` | `tag_create` | planned |
| 2 | On each consuming agent, Controls → Inherited memory (S7) → "Add layer": pick the agent, mount or pin@tag, memory types, sealed | `PUT /agents/{id}/layers` | planned |
| 3 | On the layer agent, sees who reads it and at which tag before changing anything | reverse layer lookup | **BR-10** |
| 4 | Sees a refused write in the Decision log, readable ("tried to overwrite sealed key `limit:eu` from compliance@v1") | refusal as an audit record | **BR-13** |
| 5 | Reviews who branched, forked or promoted which agent and what memory left with it | branch, fork and promote in the audit log | **BR-13** |
| 6 | Sets who may branch production memory and which agents may mount which layers | Cedar `memory:*` | planned (§6); **BR-12** |
| 7 | Rule change: tags `v2`, A3T branches each consumer with `v2`, replays, shows an impact report per agent | governance replay | **BR-15** (spec phase 5) |
| 8 | "What did the agent know at 14:32 on 3 March": a run's memory point opens the Memory tab as of that point | memory point on runs; clone or read at a point | **BR-8**, **BR-14** |

### 9.5 SRE: placement, cost, cleanup, rollback

Starts from chat home ("Check health") or agent → Health and Runtimes.

| Step | What the user does and sees | API | Status |
|---|---|---|---|
| 1 | Runtimes lists branch engines: hosted or own pod, running or hibernating, size, last activity | branch stats and runtime state | **BR-3**, **BR-16** |
| 2 | Sees branches holding up erasure ("2 FORGETs waiting on hibernating branch") and wakes or drops them | `pending_forgets`; wake | available (field); **BR-16** |
| 3 | Moves a heavy branch into its own pod, hibernates it, wakes it | controller | **BR-16** (controller not built) |
| 4 | Rolls back after a promote: redeploy the original agent (untouched by promote) from the Deploy tab | deployments | available |
| 5 | Cleans up: bulk drop of stale branches, refused while a branch has children or a tag is pinned, with the reason shown | `branch_drop`, `tag_drop` | planned |
| 6 | Watches a long promote finish without holding the page open | promote as a job | **BR-5** |

## 10. What good looks like

The bar the flows above aim for, from the 3 Nov persona review and from how branching
products people already know behave (git hosting, database branching, preview
deployments):

1. **Safe by default, and visibly so.** A branch can't touch production memory or call
   out. The S2 banner says so on every screen, not only at creation.
2. **One place, one gesture.** Branching starts from the agent header (S1), not a
   separate page. Every existing tab works in branch scope; nothing is rebuilt for
   branches.
3. **Know before you commit.** Promote, fork and drop each show what moves, what stays
   and what is deleted, in counts and plain words, before the confirm button.
4. **Diff is the centre.** "What did the branch change, and what changed on main
   meanwhile" is answered before promote, in plain language first and detail on demand.
5. **Lineage is always visible.** Every agent says where it came from (template version,
   fork, promote) and what it inherits; upgrades and pinned tags show where they apply.
6. **No dead ends.** Actions the user may not take are shown disabled with the reason
   (policy, children, pinned tag), not failed after the click. Errors name the next step.
7. **Long work is a job.** Promote and fork show progress, survive a page reload and
   notify when done.
8. **Cheap to explore, easy to clean.** Branches are cheap, so the UI encourages them,
   and it surfaces stale ones and their cost so they don't pile up.
9. **Audited without asking.** Every branch, fork, promote, tag and layer change is in
   the Decision log with who, when and what memory moved.
10. **Same screens for everyone.** Personas differ only in where they start.

## 11. APIs the UX still needs

Beyond §6 (FlowR agent endpoints, planned): what the flows in §9 need and no planned API
gives yet. The UI shows a named gap (BR-n) until each exists, as with UI-n.

| # | Need | Used by | Proposed API | Size |
|---|---|---|---|---|
| BR-1 | The §6 FlowR endpoints themselves, with `created_by` from the session, and agent ids (not namespaces) in requests and answers | every flow | §6 as planned | M |
| BR-2 | **Branch diff**: per memory type, rows added, changed (before/after) and forgotten on the branch; parent changes since the point on the same keys (conflicts); counts first, rows paged | S4, promote review, rebase | ADB `branch_diff {namespace, name, memory?, cursor?}` → `{counts: {SEMANTIC: {added, changed, forgotten}, …}, conflicts: n, rows: [...], next}`; `GET /agents/{id}/branches/{name}/diff` | M |
| BR-3 | **Branch stats** in the list: records and bytes on the branch, last write, last read | S1, S3, stale nudges, SRE | Add `{records, bytes, last_write_ts, last_read_ts}` to the Branch object | S |
| BR-4 | **Talk to and run a branch**: `/chat` and `execute_flow` take `memory_branch`; runs record it | learner step 3, developer step 2 | Spec §4.3; `memory_branch` on run, chat and session; `list_runs?branch=` | L (gateway guard) |
| BR-5 | **Promote and fork as jobs**: id, progress (records written / total), cancel, result; FlowR creates the agent only after the copy succeeds, and removes the half-written space if it fails | S5, S6, SRE step 6 | `POST …/promote` → `202 {job}`; `GET /jobs/{id}` (SSE); ADB batches report progress | M |
| BR-6 | **Flow branches and side-effect classes**: draft flow versions under a branch; each step's class (`none`, `read`, `idempotent`, `effect`) visible in the run view | developer steps 2–3 | Spec §4.1 (registry versions) and §4.3 | L |
| BR-7 | **Lineage on the agent**: `forked_from {agent, at}`, `promoted_from {agent, branch, at}`, template version, layers; children listed | S8, Agents list | Agent record fields written by fork and promote; `GET /agents/{id}/lineage` → `{parents, children}` | S |
| BR-8 | **Fork or read at a tag or a past point** ("restore yesterday", audit) | security step 8 | `space_clone {namespace, into, at?: tag \| ts}`; AQL `AS OF` on namespace reads through the API | M |
| BR-9 | **Rebase**: move the branch point, report keys both sides changed | developer step 5 | ADB `branch_rebase {namespace, name, to?}` → `{conflicts: [...]}` (spec §3.8) | M |
| BR-10 | **Who reads this layer**: spaces that mount or pin a space, and at which tag | S7, security step 3 | ADB `layers_dependents {space}` → `[{namespace, mode, tag, sealed}]` | S |
| BR-11 | **Releases**: a tag that ties the memory point to the flow versions live at that moment | developer step 7, rollback | Tag gains `flows: [{name, version}]`; `POST /agents/{id}/tags` fills it | S |
| BR-12 | **Can I?** before the click: Cedar check for `memory:*` on this agent, in one call | every action button | `POST /authz/check {actions: [...], resource}` → `{action: {allowed, reason}}` | S |
| BR-13 | **Audit and events**: branch, fork, promote, tag, layer and sealed-refusal records in the Decision log; console notices when a job ends | security steps 4–5; notifications | Records on the FlowR audit stream (with UI-3 filtering); `job.done` on the console SSE | S |
| BR-14 | **Memory on a branch or at a point**: UI-1 with `branch=` and `scope=branch-only \| view`; a run's memory point (`{space, branch, offset, layers}`) on the run record | learner step 4, security step 8 | `GET /agents/{id}/memory?branch=&scope=&at=`; spec §3.10 | M (after UI-1) |
| BR-15 | **Rule-change replay and impact report** | security step 7 | Spec §4.4, phase 5 | L |
| BR-16 | **Branch runtime**: state (hosted, pod running, hibernating), wake, hibernate, move to a pod | SRE steps 1–3 | A3T controller: `GET/POST /agents/{id}/branches/{name}/runtime` | L |
| BR-17 | **Error codes**: refusals carry a stable code beside the sentence (`name_taken`, `has_children`, `target_not_empty`, `tag_pinned`, `sealed_key`, …), so the UI can offer the fix | every dialog | gRPC status detail / FlowR `{code, message}` | S |

**Order for the UI.** BR-1, BR-3, BR-7, BR-12 and BR-17 are small and unlock S1, S3,
S5–S8 with honest states. BR-2 and BR-4 make the learner and developer loops real and
should come next. BR-5 matters once spaces are large. BR-6, BR-15 and BR-16 follow the
spec's phases 3–5.
