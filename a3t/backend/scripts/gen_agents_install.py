#!/usr/bin/env python3
"""Generates a3t/backend/flows/system/flow_system_agents_install.yaml: the A3T
agents of the plan "A3T control plane as A3T agents", with their tools (TF-1
tool_flows). Run it after changing the table below; the flow is the output."""
import json, os

def crud(flow, obj, actions, wrap):
    """One tool per action of a CRUD flow. wrap=True: the flow reads .input.*"""
    out = {}
    for a in actions:
        e = {"namespace": "a3t", "name": flow}
        if wrap:
            e["wrap"] = "input"; e["input"] = {"input": {"action": a}}
        else:
            e["input"] = {"action": a}
        e["description"] = f"{a.replace('_', ' ').capitalize()} {obj.replace('_', ' ')}s (runs a3t/{flow} action {a})."
        out[f"tool_a3t_{obj}_{a}"] = e
    return out

def direct(ns, flow, desc):
    return {"namespace": ns, "name": flow, "description": desc}

AGENTS = [
    {"id": "agent_system_core", "name": "agent_system_core", "project": "system", "template": "tmpl_system_agent_engine",
     "prompt": "You are agent_system_core, the root of every A3T agent. You are the control plane of the A3T agents: you create, configure and inspect them, and you keep the shared knowledge and the tool catalogue.",
     "tools": {
        **{f"tool_system_agent_{k}": v for k, v in {
            "list": {"namespace": "a3t", "name": "flow_a3t_agents", "wrap": "input", "input": {"input": {"action": "list"}}, "description": "List agents."},
            "get": {"namespace": "a3t", "name": "flow_a3t_agents", "wrap": "input", "input": {"input": {"action": "get"}}, "description": "One agent's record."},
            "update": {"namespace": "a3t", "name": "flow_a3t_agents", "wrap": "input", "input": {"input": {"action": "update"}}, "description": "Change an agent's record."},
            "create": {"namespace": "a3t", "name": "flow_a3t_agent_control", "input": {"action": "create"}, "description": "Create an agent from a template or an agent."},
            "lineage": {"namespace": "a3t", "name": "flow_a3t_agent_control", "input": {"action": "lineage"}, "description": "An agent's ancestors and descendants."},
        }.items()},
        "tool_system_knowledge_ingest": direct("system", "flow_system_knowledge_ingest", "Load documents into the shared knowledge (SEMANTIC system-knowledge)."),
        "tool_system_tools_sync": direct("system", "flow_system_tools_sync", "Refresh the tool catalogue (PROCEDURAL system-tools)."),
        "tool_system_agents_install": direct("system", "flow_system_agents_install", "Create or update the A3T agents."),
        "tool_system_errors_list": {"namespace": "a3t", "name": "flow_a3t_error_log", "input": {"action": "list"}, "description": "The platform error log."},
     }},
    {"id": "agent_a3t_agents", "name": "agent_a3t_agents", "prompt": "You manage users' agents: create, change, fork, merge, evolve and share them, and read their lineage.",
     "tools": {**crud("flow_a3t_agents", "agent", ["list", "get", "update", "delete", "events"], True),
               "tool_a3t_agent_record_create": {"namespace": "a3t", "name": "flow_a3t_agents", "wrap": "input", "input": {"input": {"action": "create"}}, "description": "Write an agent record as given (prefer tool_a3t_agent_create)."},
               **{f"tool_a3t_agent_{a}": {"namespace": "a3t", "name": "flow_a3t_agent_control", "input": {"action": a},
                                          "description": f"Agent control plane: {a.replace('_', ' ')}."}
                  for a in ["create", "lineage", "relate", "compare", "merge", "evolve", "shares", "share", "unshare", "shared_with", "forget", "turns", "memory", "memory_put", "memory_forget", "memory_attach", "memory_detach", "block", "unblock"]},
               "tool_a3t_agent_describe": {"namespace": "a3t", "name": "flow_a3t_agent_control", "input": {"action": "get"}, "description": "An agent's record, relation, endpoints and shares."}}},
    {"id": "agent_a3t_templates", "name": "agent_a3t_templates", "prompt": "You manage templates: the code users' agents are made from.",
     "tools": crud("flow_a3t_templates", "template", ["list", "get", "create", "update", "delete"], True)},
    {"id": "agent_a3t_projects", "name": "agent_a3t_projects", "prompt": "You manage projects, orgs and their members, handles and users.",
     "tools": {**crud("flow_a3t_projects", "project", ["list", "create", "update", "delete", "move"], False),
               **crud("flow_a3t_orgs", "org", ["list", "create", "delete", "set_member", "remove_member", "set_quota", "set_runtime"], False),
               **crud("flow_a3t_profile", "profile", ["get", "claim"], False),
               **crud("flow_a3t_users", "user", ["list", "get", "create", "update", "delete", "change"], True)}},
    {"id": "agent_a3t_resources", "name": "agent_a3t_resources", "prompt": "You manage resources (models, databases, MCP servers, secrets) and settings.",
     "tools": {**crud("flow_a3t_resources", "resource", ["list", "get", "create", "update", "delete"], True),
               **crud("flow_a3t_settings", "setting", ["list", "get", "set", "delete"], True),
               "tool_a3t_resource_test": direct("a3t", "flow_a3t_resource_test", "Check that a resource answers.")}},
    {"id": "agent_a3t_runtime", "name": "agent_a3t_runtime", "prompt": "You manage where agents run: canvases, flow releases, publishing and cloning.",
     "tools": {**crud("flow_a3t_canvas", "canvas", ["list", "get", "create", "update", "delete", "events"], True),
               **crud("flow_a3t_releases", "release", ["versions", "activate"], False),
               "tool_a3t_share": direct("a3t", "flow_a3t_share", "Publish or clone a flow, resource or trigger between projects.")}},
    {"id": "agent_a3t_governance", "name": "agent_a3t_governance", "prompt": "You answer for security and audit: the audit log, the error log and early-access requests.",
     "tools": {**crud("flow_a3t_audit", "audit", ["list", "log", "query"], True),
               "tool_a3t_error_list": {"namespace": "a3t", "name": "flow_a3t_error_log", "input": {"action": "list"}, "description": "The platform error log."},
               **crud("flow_a3t_early_access", "early_access", ["list", "set"], True)}},
    {"id": "agent_a3t_knowledge", "name": "agent_a3t_knowledge", "prompt": "You manage the knowledge graph: ingest, look up, query and train.",
     "tools": crud("flow_a3t_knowledge", "knowledge", ["ingest", "lookup", "entities", "relations", "neighbors", "predict_head", "predict_tail", "score", "stats", "train", "from_semantic"], True)},
    {"id": "agent_a3t_assistant", "name": "agent_a3t_assistant", "template": "tmpl_a3t_agent",
     "prompt": "You are the A3T assistant in the A3T console. Help people build, run and govern agents. Answer from the platform docs you are given; use your tools to read the real state; when the docs don't cover something, say so.",
     "tools": {}, "mcp": ["a3t/agent-flows"], "temperature": 0.4},
]

specs = []
for a in AGENTS:
    values = {"system_prompt": a["prompt"], "temperature": a.get("temperature", 0),
              "target": "a3t/tmpl_a3t_agent", "llm_resource": "a3t/claude-cli-max"}
    if a["tools"]: values["tool_flows"] = a["tools"]
    if "mcp" in a: values["mcp_servers"] = a["mcp"]
    else: values["mcp_servers"] = []
    specs.append({"id": a["id"], "name": a["name"], "project": a.get("project", "a3t"),
                  "template": a.get("template", "tmpl_a3t_service"),
                  "inherit": a["id"] != "agent_system_core",
                  "share": a.get("project", "a3t") == "a3t",
                  "values": values})

tools = sum(len(a["tools"]) for a in AGENTS)
flow = f"""document:
  dsl: '1.0.3'
  namespace: system
  name: flow_system_agents_install
  version: '1.0.4'
  metadata:
    description: >
      Creates or updates the A3T agents (plan "A3T control plane as A3T agents"):
      agent_system_core, the root, and the A3T agents in project a3t, with their
      tools ({tools} tool_flows entries in all). New agents are made through
      a3t/flow_a3t_agent_control (relation: template); existing ones get their
      values again. Every A3T agent then inherits agent_system_core and is shared
      with everyone for calls, so any user's dash can call its tools. Safe to run
      again. GENERATED by a3t/backend/scripts/gen_agents_install.py: edit that.
      Input: {{ org_id?, user_id? }}. Output: {{ created, updated, failed }}.

metadata:
  a3t:
    extends: system/flow_system_base@1.0.0

do:
  - setup:
      set:
        _org: '${{ .org_id // "org-default" }}'
        _who: '${{ .user_id // "system" }}'
        _now: '${{ now | todateiso8601 }}'
        _specs: {json.dumps(json.dumps(specs))}
        created: []
        updated: []
        failed: []
      export:
        as: '${{ $context + . + {{_specs: (.["_specs"] | fromjson)}} }}'

  - agents:
      for:
        each: _a
        in: '${{ $context._specs }}'
        do:
          # A loop's item isn't in scope inside run: workflow inputs: keep it in the context.
          - pick:
              set:
                _cur: '${{ $_a }}'
              export:
                as: '${{ $context + . }}'
          - one:
              try:
                do:
                  - existing:
                      call: lancedb
                      with:
                        operation: query
                        table: agent_configs
                        filter: '${{ "id = ''" + $context._cur.id + "''" }}'
                        limit: 1
                        _resource: a3t/a3t-platform-db
                      export:
                        as: '${{ $context + {{_row: ((if type == "array" then . else (.rows // .data // []) end) | .[0] // null)}} }}'
                  - make:
                      if: '${{ $context._row == null }}'
                      run:
                        workflow:
                          namespace: a3t
                          name: flow_a3t_agent_control
                          input:
                            action: create
                            org_id: '${{ $context._org }}'
                            user_id: '${{ $context._who }}'
                            id: '${{ $context._cur.id }}'
                            name: '${{ $context._cur.name }}'
                            project: '${{ $context._cur.project }}'
                            agent_mode: session
                            from: '${{ {{kind: "template", id: $context._cur.template}} }}'
                            values: '${{ $context._cur.values }}'
                            resource_bindings: '${{ {{llm: $context._cur.values.llm_resource}} }}'
                      export:
                        as: '${{ $context + {{created: ($context.created + [$context._cur.id])}} }}'
                  - refresh:
                      if: '${{ $context._row != null }}'
                      call: lancedb
                      with:
                        operation: update
                        table: agent_configs
                        filter: '${{ "id = ''" + $context._cur.id + "''" }}'
                        values:
                          template_id: '${{ $context._cur.template }}'
                          variables: '${{ ((($context._row.variables // "{{}}") | try fromjson catch {{}}) * $context._cur.values) | tojson }}'
                          resource_bindings: '${{ {{llm: $context._cur.values.llm_resource}} | tojson }}'
                          namespace: '${{ $context._cur.project }}'
                          updated_at: '${{ $context._now }}'
                        _resource: a3t/a3t-platform-db
                      export:
                        as: '${{ $context + {{updated: ($context.updated + [$context._cur.id])}} }}'
                  - inherit:
                      if: '${{ $context._cur.inherit }}'
                      run:
                        workflow:
                          namespace: a3t
                          name: flow_a3t_agent_control
                          input:
                            action: relate
                            org_id: '${{ $context._org }}'
                            user_id: '${{ $context._who }}'
                            id: '${{ $context._cur.id }}'
                            parent: '${{ {{kind: "agent", id: "agent_system_core"}} }}'
                            relation: inherit
                  - share:
                      if: '${{ $context._cur.share }}'
                      run:
                        workflow:
                          namespace: a3t
                          name: flow_a3t_agent_control
                          input:
                            action: share
                            org_id: '${{ $context._org }}'
                            user_id: '${{ $context._who }}'
                            id: '${{ $context._cur.id }}'
                            target: '*'
                            rights: '${{ {{call: true, inherit: false, fork: false, merge_back: false}} }}'
                catch:
                  do:
                    - failed_one:
                        set:
                          failed: '${{ $context.failed + [{{id: $context._cur.id, error: ($context.error.detail // $context.error.title // "failed")}}] }}'
                        export:
                          as: '${{ $context + . }}'

  - report:
      set:
        created: '${{ $context.created }}'
        updated: '${{ $context.updated }}'
        failed: '${{ $context.failed }}'
"""
here = os.path.dirname(os.path.abspath(__file__))
out = os.path.join(here, "..", "flows", "system", "flow_system_agents_install.yaml")
open(out, "w").write(flow)
print(f"wrote {out}: {len(AGENTS)} agents, {tools} tools")
