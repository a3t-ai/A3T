# Archive

Old flows, kept for reference. Nothing calls them. On vc-dev they live in project
`system-archive` (org `system`, private), moved there on 12 Nov 2026.

| Flow | Was | Why archived |
|---|---|---|
| `agent_dispatch.yaml` | `a3t/a3t-agent-dispatch` | Calling an agent is now its data plane (`/chat`) |
| `examples/agents/*` | `a3t/ops-agent`, `a3t/support-agent` | Early example agents on the old engine |
| `examples/integrations/*` | `a3t/create-ticket`, `http-fetch`, `query-sql`, `search-lancedb`, `send-email`, `web-scrape` | Early integration samples |

Also archived on vc-dev, with no source here: `a3t/ops-ping` (and its trigger `ops-ping-hook`,
removed) and `a3t/st1-fault` (a test fault flow). The exact definitions taken off vc-dev,
the removed trigger and the deleted agent `a3t-docs-helper` are in
`.ai/archive-2026-11-12/`.

Still in use, archived once their replacements run (plan: "A3T control plane as A3T
agents"): `a3t/a3t-chat-agent`, `system/agent-template`, `system/agent-turn`, template rows
`tpl-system-shared-agent-engine` and `tpl-a3t-shared-agent-engine-*`, and every flow with an
old-style name.
