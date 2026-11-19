//! Tools that run flows (TF-1, plan "A3T control plane as A3T agents").
//!
//! An agent's `tool_flows` maps a tool name to the flow it runs. Besides the
//! flow, an entry may carry:
//!
//! - `input`: fixed values merged over the caller's arguments (they win), so one
//!   CRUD flow serves many tools: `{input: {action: "list"}}`;
//! - `wrap`: the key the caller's arguments go under before that merge, for flows
//!   that read their arguments from one key (`.input.*`);
//! - `schema` and `description`: what the tool offers to an LLM and to a backend,
//!   instead of the flow's own input schema and description.
//!
//! The same entry serves the agent engine (an LLM's tool call) and the control
//! plane (`POST /agents/{id}/tools/{tool}`, AT-1), so both build the same input.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolFlow {
    pub namespace: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl ToolFlow {
    /// The flow's input for a call with `args`: the arguments (under `wrap`, when
    /// set), with the fixed `input` merged over them.
    pub fn call_input(&self, args: Value) -> Value {
        let args = match self.wrap.as_deref() {
            Some(k) if !k.is_empty() => json!({ k: args }),
            _ => args,
        };
        match &self.input {
            Some(fixed) => merge(args, fixed),
            None => args,
        }
    }
}

/// `over` merged into `base`: objects key by key, anything else replaced. `over` wins.
pub fn merge(base: Value, over: &Value) -> Value {
    match (base, over) {
        (Value::Object(mut b), Value::Object(o)) => {
            for (k, v) in o {
                let cur = b.remove(k).unwrap_or(Value::Null);
                b.insert(k.clone(), merge(cur, v));
            }
            Value::Object(b)
        }
        (_, o) => o.clone(),
    }
}

/// An agent's tools, from its record (`agent_configs.variables.tool_flows`).
/// Entries that don't parse are left out.
pub fn of_agent(row: &Value) -> BTreeMap<String, ToolFlow> {
    let vars = match row.get("variables") {
        Some(Value::String(s)) => serde_json::from_str::<Value>(s).unwrap_or(Value::Null),
        Some(v) => v.clone(),
        None => Value::Null,
    };
    let mut out = BTreeMap::new();
    if let Some(m) = vars.get("tool_flows").and_then(|v| v.as_object()) {
        for (name, entry) in m {
            if let Ok(t) = serde_json::from_value::<ToolFlow>(entry.clone()) {
                out.insert(name.clone(), t);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(v: Value) -> ToolFlow {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn plain_entry_passes_arguments_through() {
        let t = tool(json!({"namespace": "a3t", "name": "flow_a3t_x"}));
        assert_eq!(t.call_input(json!({"a": 1})), json!({"a": 1}));
    }

    #[test]
    fn fixed_input_wins_and_wrap_nests_the_arguments() {
        let t = tool(json!({"namespace": "a3t", "name": "flow_a3t_agents", "wrap": "input",
                             "input": {"input": {"action": "list"}}}));
        // The caller can't change the action; its own fields stay.
        assert_eq!(t.call_input(json!({"org_id": "o", "action": "delete"})),
                   json!({"input": {"org_id": "o", "action": "list"}}));
    }

    #[test]
    fn agent_tools_come_from_its_variables() {
        let row = json!({"variables": "{\"tool_flows\":{\"tool_a3t_agent_list\":{\"namespace\":\"a3t\",\"name\":\"flow_a3t_agents\"},\"bad\":1}}"});
        let tools = of_agent(&row);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools["tool_a3t_agent_list"].name, "flow_a3t_agents");
        assert!(of_agent(&json!({})).is_empty());
    }
}
