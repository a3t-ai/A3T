# A3T

A3T runs AI agents and workflows. Workflows are written in the CNCF Serverless
Workflow DSL (1.0) and executed by **FlowR**. Agent memory and platform data live
in **ADB** and are queried with **AQL**.

Licensed under the [Elastic License 2.0](LICENSE) (ELv2); no licence key is needed
for any feature. See [NOTICE](NOTICE).

## Repository layout

| Path | What it is |
|---|---|
| `shared/` | Workflow DSL parser (`swf-parser`), runtime (`swf-runtime`), expressions (`flowr-expr`), types, storage traits, MCP core, policy contract, metrics |
| `flowr/` | The FlowR workspace: `flowr-ce` (engine, handlers, MCP tools), `flowr-ce-storage`, `flowr-ce-agents`, `flowr-server` (the HTTP server and binary), `flowr-canvas` (Kubernetes controller) and `a3t-security` (Cedar policy, OIDC; built with `--features ee`) |
| `a3t/ui/` | The A3T console and Studio (SvelteKit) |
| `a3t/backend/flows/` | A3T's own flows (installers, platform and agent flows) |
| `a3t/policies/` | Cedar schema, baseline policy and the policy catalogue |
| `a3t/wwwdocs/` | Developer docs (docs.a3t.io) |
| `claude-wrapper/` | The Claude CLI wrapper image |
| `deploy/` | Kubernetes manifests |
| `tests/` | Workflow test flows and runner, policy fixtures, the choke-point check |

## Building

FlowR builds next to sibling checkouts of [ADB](https://github.com/a3t-ai/ADB) and
[AQL](https://github.com/a3t-ai/AQL):

```
Dev/
  A3T/   this repository
  ADB/
  AQL/
```

```
cd flowr && cargo build --release -p flowr-server                  # local edition
cd flowr && cargo build --release -p flowr-server --features ee    # with a3t-security
```

`./deploy.sh` builds, packages and rolls the images (`./deploy.sh` with no arguments
prints its usage).
