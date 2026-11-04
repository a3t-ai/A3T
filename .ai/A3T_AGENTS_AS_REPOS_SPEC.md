# Agents as repos — spec (A3T, FlowR, ADB)

Source: the design post *Agents as Repos: Inheritance, Memory Branching and Governed
Evolution in A3T* (blog.a3t.io/agents-as-repos, 3 Nov 2026). This spec turns it into
build steps, after a review of what ADB and FlowR do today (§2). Status: **draft, 3 Nov
2026** — decisions marked *(default)* stand until the user changes them.

An agent is a pair: **flows** (CNCF Serverless Workflow + agent code, in the FlowR
registry) and **memory** (in ADB). Five operations act on the pair:

| Operation | Flows | Memory |
|---|---|---|
| **Template** | the flows, as a versioned template | schema and seed procedures, no data |
| **Clone** | a copy, linked to its source version | a snapshot of the source's memory (a fork, §4.6) |
| **Branch** | a draft of the flows | a copy-on-write overlay of memory at a commit point |
| **Merge** (promote) | a reviewed change to the parent | reviewed procedure diffs; data only on request |
| **Inherit** | `extends` a parent template version | mount / pin / fork another agent's memory layer |

## 1. Scope

**In (v1):** template lineage (flows); ADB branches of an agent's memory with overlay
reads and writes; branch runs with side effects guarded by the gateway; diff, rebase
and promote for flows and procedures; inheritance by mount and pin with sealed layers;
rule-change replay with an impact report; point-in-time audit.

**Out of v1:** automatic merge of semantic memory (reviewed and manual only); branch
flattening (a threshold is measured first); caching and isolation for hot shared
layers; memory crossing partitions (one partition today, ADB S1); external gateways.

## 2. Review: the post against ADB and FlowR today

### 2.1 What ADB already has

| Post says | ADB today | Consequence |
|---|---|---|
| One commit log across memory types | **Yes.** One sequencer, one WAL per partition; every write of every memory type (and platform tables, FlowR engine spaces) gets one offset and one `commit_ts`. | A branch point is one log offset → one consistent cut across all memory, graph included. |
| Snapshots: any committed version readable | **Yes, per row, not per file.** Every row version carries `valid_from` / `valid_to` / `superseded_by` (transaction time) and facts carry `valid_from` / `valid_to` in their value (valid time). AQL v0.10 `AS OF t` returns what ADB held at `t`; `WITH HISTORY` every version (`adb-store/src/rows.rs` `is_current_at`). | A branch can read its parent **as of the branch point** with no copy and no manifest. |
| Immutable segments + manifest per commit | **Not the model.** The store is Lance tables with a MemWAL; ADB does not read through Lance versions, and compaction (W5) rewrites the table and deletes old Lance versions. The table is shared by every tenant in the partition. | Branching at the Lance level would fork *every tenant's* memory at once. Branching is a **row-level overlay** instead (§3). "Flush before branching" is not needed: a log offset is already a consistent cut. |
| Retention pins | **Missing.** Compaction keeps superseded versions for **90 days** (`compaction_row`, W5 decision 27 Oct), then drops them. | A branch older than 90 days would lose its parent's view. Needs per-space pins (§3.6). |
| Episodic is append-only | **Not quite.** `UPDATE` on EPISODIC / WORKING / TOOLS patches the row **in place** (`Effect::Patch`); only SEMANTIC / PROCEDURAL supersede. | A parent's patch after the branch point would leak into the branch's `AS OF` view. Patches must version while a branch covers the space (§3.5). |
| Memory types: working, episodic, semantic, procedural | ADB also has **TOOLS** (tool registrations). | TOOLS is treated like procedural (§3.4). |
| Forget | `FORGET` erases **physically and immediately** (log redacted, table rewritten, W5). | Erasure must reach every branch and every mount; a branch from before a forget no longer reproduces it (by design). |
| An agent's memory | **No agent key.** FlowR keys an agent's memory by **namespace** (`agent_memory.rs`: recall `NAMESPACE <ns>`), shared by the agent's users; user facts carry `owner`. Rows also carry `tenant_id`, `scope`, `visibility` (Private / Shared / Cluster) and `provenance.agent_id`. | A **memory space** must be defined before anything can be branched (§3.1). |
| Graph | Links (`PutEdge`) and facts are rows like the others; traversal (`FOLLOW LINKS`, DEPTH, INBOUND/BOTH) runs in one partition. | The overlay applies to edges and facts the same way; traversal applies it at every hop. |
| Indexes | Vector index (`LIKE` recall), hot cache (W10), live-key index. | Branch delta is small: searched directly and merged with the parent's ANN result filtered `AS OF` (§3.7). |

### 2.2 What FlowR / A3T already has

| Need | Today |
|---|---|
| Versioned templates | `template_registry` rows have `version`, `digest`, `signed_by`, `published`. |
| Agent → template version | `agent_configs` has `template_id`, `template_version` (generation 1 of lineage). Templates cannot extend templates. |
| Flow versions | `document.version` — pushing the same version **overwrites silently** (Studio backend gap, `A3T_STUDIO_ARCHITECTURE.md` §14). |
| Side-effect classes | The journal classifies steps (`journal_hook::class_of`: pure / read / idempotent / effect, `metadata.a3t.stepClass`). Not declared per handler; debug runs execute real side effects (L18). |
| One place every outbound call passes | The gateway pool (cluster design §10): runtimes send `call:http` / `call:mcp` / `call:llm` there; it decides (placeholder) and records. |
| Replay | Re-run from a checkpoint (L19); the journal holds each step's input and output. |

## 2.3 Revision, 4 Nov 2026 (user) — supersedes §3.2–3.8 placement and §3.8 promote

**A branch is isolated from the production agent.** The first build put branch rows in
the org's table, through the org's sequencer: test and dev traffic still loaded
production ADB. Instead each branch has **its own log, its own sequencer and its own
small delta table** (a separate ADB engine). No Iggy: the branch log is ADB's own WAL.

| | Log | Sequencer | Storage | Compute |
|---|---|---|---|---|
| main (every agent in the org) | the org partition's WAL (Raft) | the org's | the org's Lance table | shared ADB nodes |
| branch | its own WAL, from the branch point | its own | its own delta table + the parent read `AS OF` the point | **either** hosted in the main ADB nodes (an engine beside the org's) **or** its own pod (a hibernating Sandbox); same code |
| new agent (promote) | the org's WAL | the org's | rows in the org table, new namespace | shared ADB nodes |

- The org sees only a branch's **control records** (created at point X, dropped) and
  answers **reads `AS OF` the point** for its parent rows; it gets no branch writes, no
  branch rows and no branch compaction.
- Reads on a branch: the overlay of §3.3 (nearest level wins per lineage), with the
  branch's rows from its delta table and the parent's from the org table `AS OF t_B`.
- **Merge = promote as a new agent** (user, 4 Nov): there is no merge into the parent's
  memory. Promote materialises the branch's view into a **new agent space in production**
  (a new namespace, written through the org's sequencer in batched records) together
  with the branch's flow versions as the new agent's; the original agent is untouched
  and traffic moves by deployment. *(default)* memory carried: SEMANTIC, PROCEDURAL,
  TOOLS of the branch view; EPISODIC history stays with the original agent; WORKING
  never. The §3.8 "apply overrides to the parent" and semantic merge are dropped.
- **Fork / clone** is the same materialisation without a branch (a space at a point into
  a new space), so branch and clone are different operations with different costs:
  a branch is cheap and depends on the org's retention pins; a clone is a copy and
  depends on nothing.
- Retention pins (§3.6) stay at the org. FORGET on main reaches branches: the org logs
  the erasure, a branch engine applies it to its delta table (and drops the parent row
  from its view) before serving, also when it wakes from hibernation.

## 3. ADB: branching

### 3.1 Memory space

A **memory space** is what gets branched, mounted and pinned: an agent's memory.
*(default)* space = `(tenant_id, namespace)` — the agent's namespace, as FlowR uses it
today. An agent that needs a space of its own gets its own namespace (A3T creates one
per agent deployment: `<project>/agents/<agent>` → namespace `<project>--<agent>`). A
space id is `ms:<tenant>/<namespace>`.

### 3.2 Branches

A branch is a row in a new ADB control table `branches` (logged like any write):

| Field | Meaning |
|---|---|
| `id` | `br:<space>/<name>`; `main` is implicit for every space |
| `space` | the memory space |
| `parent` | parent branch id (`main` or another branch) |
| `point_offset`, `point_ts` | the branch point: a log offset and its `commit_ts` |
| `state` | `open` · `promoted` · `discarded` |
| `created_by`, `created_at`, `policy` | who, when; the side-effect and access policy it runs under |

Creating a branch writes one record and copies nothing. Every data row gets a new
column **`branch`** (schema v5; existing rows `main`). A row written on a branch carries
that branch id.

### 3.3 Reads

A read on branch `B` (parent `P`, point `t_B`) sees:

1. rows with `branch = B` that are current now, plus
2. rows of `P` that were current at `t_B` (`is_current_at(t_B, valid)`), recursively for
   `P`'s own parent at `P`'s point,
3. minus the parent lineages `B` **overrides** or **deletes**.

Overrides: a write on `B` that updates or forgets a parent lineage stores a branch row
with `overrides = <parent lineage>` (copy-on-write) or a branch tombstone; the parent's
row is never touched. `AS OF t` on a branch applies the same rule with `t` for the
branch rows. Facts keep their valid-time (`valid_from` / `valid_to`) unchanged.

The planner rewrites every read (RECALL, LOOKUP, FOLLOW LINKS, aggregates, WATCH) into
this filter. Branch depth > 1 walks the chain; flattening a deep branch into a new
branch from the same point is v2 (threshold measured first).

AQL: `... BRANCH <name>` on a statement, or `SET BRANCH <name>` for a session; the
Flight and HTTP APIs take `branch`. Without one, reads and writes are on `main`.

### 3.4 Writes, per memory type

| Memory | On a branch | Promote |
|---|---|---|
| **Working** | not branched: a branch run writes its own session's working memory (sessions are new) | discarded |
| **Episodic** | parent episodes readable up to the point; new episodes are the branch's own stream | kept as branch lineage; never merged |
| **Semantic + graph** (facts, entities, links) | copy-on-write overrides, tombstones for deletes | reviewed, manual (v1) |
| **Procedural** | copy-on-write (small) | merged as reviewed diffs |
| **Tools** | as procedural | as procedural |

### 3.5 Patches

`Effect::Patch` (EPISODIC / WORKING / TOOLS `UPDATE`) edits in place today. While any
open branch or pin covers the space, a patch on `main` becomes a **versioned patch**
(close the old version, write the new, as `Supersede` does), so `AS OF t_B` still sees
the old value. A patch on a branch of a parent row is an override (§3.3).

### 3.6 Retention pins and compaction

Compaction keeps a superseded version while any open branch or pin needs it: the
history horizon of a space becomes `min(now − 90 days, oldest point_ts of its open
branches and pins)`. `compaction_row` gets the space of each row and the pin table.
Discarding or promoting a branch releases its pin. A branch that pins history past a
configured age (*(default)* 180 days) is reported to its owner.

**FORGET wins:** a forget on `main` erases the record from every branch, pin and mount
(the overlay cannot resurrect it: the erasure removes the parent rows the branch would
read, and branch overrides of that lineage are erased too). A branch from before the
forget no longer reproduces it; this is recorded in the branch's audit trail.

### 3.7 Indexes and cache

`LIKE` recall on a branch: ANN on the parent filtered `AS OF t_B` and excluding the
overridden lineages, plus an exact search over the branch's own rows (small), merged by
score. A branch index is built when the delta passes *(default)* 5,000 rows. The hot
cache and live-key index are keyed by branch. Promote rebuilds indexes on the result.

### 3.8 Tags, pins and diffs

- **Tag**: a named point (`tag:<space>/<name>` → offset), e.g. `compliance/v2`.
- **Diff** `B` vs `P`: `B`'s rows (new, overrides, tombstones) + `P`'s changes since
  `t_B` on the lineages `B` touches (the conflicts a rebase would meet).
- **Rebase** `B` onto a newer point: moves `point_offset`; reports lineages both sides
  changed; `B`'s overrides keep winning unless the user drops them.
- **Promote**: for procedural / tools rows, applies `B`'s overrides to `P` as new
  versions in one record (reviewed in A3T first); semantic only on an explicit,
  reviewed data promotion; episodic never.

### 3.9 Mounted and pinned layers (inheritance)

An agent's space can list **layers**: `{space, mode: mount | pin(tag) | fork, memories:
[semantic, procedural, tools], sealed: true}`.

- **Mount**: reads also see the layer's `main`, current. **Pin**: as of the tag's offset.
  **Fork**: a branch of the layer with no upstream link, owned by the agent.
- Layers carry only the memory types listed; episodic and working never cross.
- **Sealed**: writes to a lineage that belongs to a sealed layer are refused; a write
  that would shadow it (same `key` / same (subject, predicate) for facts) is refused.
  Calling an inherited procedure is allowed.
- **Precedence** between the agent's own facts and a layer's is a per-agent policy
  order (*(default)* sealed layers first, then own), not layer stacking.
- Cross-tenant layers (another org's compliance agent) need `visibility: shared` on the
  layer and a policy grant (§4.5); reads across tenants are otherwise refused.

### 3.10 Point-in-time audit

Every agent turn and every gateway decision records its **memory point**: `{space,
branch, offset}` plus `{layer, tag or offset}` for each layer read. "What did the agent
know at 14:32 on 3 March" = reads `AS OF` that point on that branch with those layers.
History older than the retention horizon is gone unless a tag pins it: regulated
deployments tag every production release (§4.4).

## 4. FlowR and A3T

### 4.1 Prerequisite: registry versions

Flows, templates, agents and resources get immutable versions (a push of an existing
version is refused; a new version is a new row; one is **active**). This is also the
Studio's open backend gap (§14 of the Studio spec) — one piece of work for both.

### 4.2 Lineage (phase 1, flows only)

- A template may declare `extends: {name, version}`; it inherits its parent's flows,
  extensions, locked fields and defaults, and overrides what it names.
- `agent_configs` keeps `template_id` + `template_version` (exists) and adds
  `memory_space` and `layers` (§3.9).
- **Upgrade**: when a parent publishes a version, children see "upgrade available";
  `a3t agent upgrade <agent> --to <version>` (and in the Studio) shows the flow diff and
  applies it; locked fields of the parent cannot be overridden by the child.

### 4.3 Branch runs

- A run carries `memory_branch` (run row, principal context). `call:a3t:agent` and
  `call:adb` pass it to ADB (`BRANCH`). The default is `main`.
- **Side effects off by default**: on a non-`main` branch the gateway's Decide refuses
  or mocks outbound calls unless the branch's policy allows the target. Every handler
  declares a side-effect class (`none` · `read` · `idempotent` · `effect`); undeclared
  = `effect`. Steps that do not leave A3T but write elsewhere (`call:lancedb`, state,
  `emit`) are scoped to the branch or refused. Mocks: a recorded response from the
  journal of the run being replayed (§4.4), or a declared fixture.
- Flows on a branch run the branch's draft flow versions (a flow branch is a set of
  unpublished flow versions, held in the registry under the branch id).

### 4.4 Governance: rule changes

1. The layer owner tags a new version (`compliance/v2`) and marks it mandatory with a
   grace period, or optional.
2. A3T creates, per agent that mounts or pins the layer, a branch with the layer pinned
   at `v2`.
3. A3T replays the agent's recent runs on that branch in shadow mode: inputs from the
   journal, outbound calls answered from the recorded responses, nothing leaves A3T;
   plus the agent's regression flows if it declares them.
4. The **impact report** per agent: runs whose decisions or outputs differ, with the
   steps and memory reads that changed them.
5. On promotion, agents move to `v2`; after a mandatory deadline, runs of an agent still
   on `v1` are refused at admission (router check) until it upgrades.

### 4.5 Policy (Cedar actions)

`memory:branch`, `memory:promote`, `memory:promote_data`, `memory:mount`, `memory:pin`,
`memory:fork`, `memory:tag`; resource attributes: space owner org, memory types carried,
`sealed`, production or not. "Who may branch production memory" and "which agents may
mount which layers" are policies, checked when the branch or layer is created and at
every read through the gateway once the policy layer is in.

### 4.6 Clone and template

- **Template** from an agent: its flows (versions) + memory schema (memory types used,
  fact predicates, procedure names) + seed procedures; no data.
- **Clone**: flows + a fork (§3.9) of the source's memory at a point; across orgs only
  procedures and schema, never episodes or client data (policy, §4.5).

### 4.7 Interfaces

CLI (`a3t agent branch | diff | rebase | promote | upgrade | tag`), the same as MCP
tools and REST under `/agents/<id>/branches`, and the Studio (Versions panel: branches
beside versions; diff in the split view).

## 5. Phases

| # | Phase | Repos | Ends with |
|---|---|---|---|
| 0 | **Prerequisites**: registry versions (§4.1); memory space per agent (§3.1) | FlowR, ADB | Existing behaviour unchanged; conformance and FlowR suites pass |
| 1 | **Lineage** (§4.2) | FlowR / A3T | Three-generation chat lineage (system → A3T → crawl): a fix in the root reaches the others as an upgrade |
| 2 | **ADB branches as isolated engines** (§2.3, ADB `.ai/BRANCHING.md` B1–B6): control records on the org log, branch engines (own WAL, sequencer, delta table; hosted or own pod), overlay reads, branch writes, pins, FORGET reaching branches, promote as a new agent, clone | ADB | Branch traffic never reaches the org's sequencer or table; a branch reads its parent `AS OF` the point plus its own writes; compaction keeps pinned history; FORGET reaches branches; a promoted agent equals the branch view |
| 3 | **Branch runs** (§4.3): run → branch, side-effect classes, gateway guard, CLI branch / diff / promote | FlowR | The developer loop of the post end to end: branch, run, diff, rebase, promote; no outbound call leaves A3T from a branch |
| 4 | **Inheritance** (§3.9): layers, sealed, precedence, cross-tenant grants | ADB, FlowR | The trading agent mounts the compliance layer, calls `check_trade`, cannot shadow it, never sees compliance episodes |
| 5 | **Governance** (§4.4) and **audit** (§3.10) | FlowR, ADB | Compliance v2 → per-agent impact reports from replay; grace period enforced; a decision's memory point reproduces what the agent read |

Phase 1 needs no ADB work and pays off with empty memory (the post's "first thing we
build"). Phases 2 and 3 are the developer case; 4 and 5 the governance case.

## 6. Decisions to take (defaults in use until changed)

1. Memory space = the agent's namespace, one agent per namespace for branching *(default)*;
   or a new explicit `space` key on rows.
2. Branching as a row-level overlay *(default)*, not Lance-level table forks — §2.1.
3. Patches version only while a branch or pin covers the space *(default)*, or always.
4. Pinned history beyond 90 days: allowed, reported after 180 days *(default)*.
5. FORGET reaches every branch and mount *(default; GDPR)*.
6. Precedence: sealed layers before the agent's own facts *(default)*.
7. Side effects on branches: blocked unless allowed by the branch policy *(default)*; mocks
   from recorded responses when replaying.
8. Order: phase 1 (lineage) first *(default)*, or the developer case (phases 2–3) first —
   the post's own question 3.

## 7. Risks and limitations (known now)

- **AB-1** Overlay reads cost more than plain reads (the shadow set, the chain); measure
  on the conformance suite and on recall latency before phase 3.
- **AB-2** Semantic merge is manual in v1; conflicting facts between branches are listed,
  not resolved.
- **AB-3** Mocking is only as complete as the side-effect classes; anything undeclared is
  blocked on branches, which may block legitimate test runs.
- **AB-4** A hot mounted layer is read by every agent that mounts it; no caching or
  isolation yet.
- **AB-5** One partition (ADB S1): spaces, branches and layers live in one table; a large
  tenant's branches compete with everyone's compaction.
- **AB-6** Replay fidelity: a run whose decisions depend on the clock or on calls without
  recorded responses cannot be replayed exactly.
