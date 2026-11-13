# Old flow name -> (namespace, new name, old file, new file). Step 1 of the agents plan.
FLOWS = [
    ("system", "a3t-base", "flow_system_base", "a3t_base.yaml"),
    ("system", "a3t-error-handler", "flow_system_error_handler", "a3t_error_handler.yaml"),
    ("system", "a3t-install", "flow_system_install", "a3t_install.yaml"),
    ("system", "a3t-schema-install", "flow_system_schema_install", "a3t_schema_install.yaml"),
    ("system", "a3t-projects-install", "flow_system_projects_install", "a3t_projects_install.yaml"),
    ("system", "a3t-deploy-init", "flow_system_deploy_init", "a3t_deploy_init.yaml"),
    ("system", "a3t-triggers-setup", "flow_system_triggers_setup", "a3t_triggers_setup.yaml"),
    ("system", "a3t-log-retention", "flow_system_log_retention", "a3t_log_retention.yaml"),
    ("system", "platform-move-resources", "flow_system_move_resources", "platform_move_resources.yaml"),
    ("system", "agent-control-install", "flow_system_agent_control_install", "agent_control_install.yaml"),
    ("system", "agent-engine-install", "flow_system_agent_engine_install", "agent_engine_install.yaml"),
    ("system", "agent-knowledge-ingest", "flow_system_knowledge_ingest", "agent_knowledge_ingest.yaml"),
    ("system", "agent-tools-sync", "flow_system_tools_sync", "agent_tools_sync.yaml"),
    ("system", "agent-turn", "flow_system_agent_turn", "agent_turn.yaml"),
    ("system", "agent-engine", "tmpl_system_agent_engine", "agent-engine.yaml"),
    ("a3t", "a3t-agent-control", "flow_a3t_agent_control", "a3t_agent_control.yaml"),
    ("a3t", "a3t-chat-engine", "tmpl_a3t_agent", "a3t_chat_engine.yaml"),
    ("a3t", "a3t-early-access", "flow_a3t_early_access", "a3t_early_access.yaml"),
    ("a3t", "a3t-error-log", "flow_a3t_error_log", "a3t_error_log.yaml"),
    ("a3t", "a3t-orgs", "flow_a3t_orgs", "a3t_orgs.yaml"),
    ("a3t", "a3t-profile", "flow_a3t_profile", "a3t_profile.yaml"),
    ("a3t", "a3t-projects", "flow_a3t_projects", "a3t_projects.yaml"),
    ("a3t", "a3t-releases", "flow_a3t_releases", "a3t_releases.yaml"),
    ("a3t", "a3t-share", "flow_a3t_share", "a3t_share.yaml"),
    ("a3t", "a3t-agents", "flow_a3t_agents", "agents_crud.yaml"),
    ("a3t", "a3t-audit", "flow_a3t_audit", "audit_crud.yaml"),
    ("a3t", "a3t-canvas", "flow_a3t_canvas", "canvas_crud.yaml"),
    ("a3t", "a3t-knowledge", "flow_a3t_knowledge", "knowledge_crud.yaml"),
    ("a3t", "a3t-resources", "flow_a3t_resources", "resources_crud.yaml"),
    ("a3t", "a3t-settings", "flow_a3t_settings", "settings_crud.yaml"),
    ("a3t", "a3t-templates", "flow_a3t_templates", "templates_crud.yaml"),
    ("a3t", "a3t-users", "flow_a3t_users", "users_crud.yaml"),
    ("a3t", "a3t-test-resource", "flow_a3t_resource_test", "test_resource.yaml"),
]
# Template and agent row ids.
IDS = [("tpl-agent-engine", "tmpl_system_agent_engine"), ("tpl-a3t-agent", "tmpl_a3t_agent")]
ARCHIVE = ["a3t/chat_agent.yaml", "system/agent-instance-schema.yaml"]

import re
def rename_text(s, extra=()):
    pairs = sorted([(o, n) for _, o, n, _ in FLOWS] + list(IDS) + list(extra), key=lambda p: -len(p[0]))
    for old, new in pairs:
        s = re.sub(r'(?<![\w-])' + re.escape(old) + r'(?![\w-])', new, s)
    return s
