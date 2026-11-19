//! The capability manifest (AC-16): what an agent version can reach, computed from its flows
//! and layers, and diffable, so a merge that adds an outbound host says so before it lands.

use super::*;

use std::collections::BTreeSet;

/// What a set of flow documents reaches. Dynamic targets (`${ … }`) are listed as written:
/// the manifest can't say where they go, only that one exists.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Reach {
    pub hosts: BTreeSet<String>,
    pub mcp: BTreeSet<String>,
    pub llm: BTreeSet<String>,
    pub memory: BTreeSet<String>,
    pub subflows: BTreeSet<String>,
    pub calls: BTreeSet<String>,
}

fn host_of(endpoint: &str) -> String {
    if endpoint.contains("${") {
        return endpoint.to_string();
    }
    let rest = endpoint.split_once("://").map(|(_, r)| r).unwrap_or(endpoint);
    rest.split(['/', '?', '#']).next().unwrap_or(rest).to_string()
}

/// Walk one flow document (as JSON) for what its tasks call.
pub fn scan(doc: &Value, into: &mut Reach) {
    match doc {
        Value::Array(items) => items.iter().for_each(|v| scan(v, into)),
        Value::Object(m) => {
            if let Some(call) = m.get("call").and_then(|c| c.as_str()) {
                into.calls.insert(call.to_string());
                let with = m.get("with").cloned().unwrap_or(Value::Null);
                let s = |k: &str| with.get(k).and_then(|v| v.as_str()).map(String::from);
                match call {
                    "http" => {
                        let endpoint = with.get("endpoint").and_then(|e| e.as_str().map(String::from).or_else(|| e.get("uri").and_then(|u| u.as_str()).map(String::from)));
                        if let Some(e) = endpoint {
                            into.hosts.insert(host_of(&e));
                        }
                    }
                    "mcp" => {
                        if let Some(server) = s("server").or_else(|| s("resource")) {
                            into.mcp.insert(server);
                        }
                    }
                    "llm" => {
                        if let Some(r) = s("resource").or_else(|| s("model")) {
                            into.llm.insert(r);
                        }
                    }
                    "adb" => {
                        if let Some(r) = s("_resource") {
                            into.memory.insert(r);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(w) = m.get("run").and_then(|r| r.get("workflow")) {
                let part = |k: &str| w.get(k).and_then(|v| v.as_str()).unwrap_or_default();
                into.subflows.insert(format!("{}/{}", part("namespace"), part("name")));
            }
            m.values().for_each(|v| scan(v, into));
        }
        _ => {}
    }
}

/// What `flows` (`[{namespace, name, version}]`) reach. A flow that can't be read is named, so
/// a gap can't look like an empty manifest.
pub async fn of_flows(s: &dyn StorageBackend, flows: &[Value]) -> Value {
    let mut reach = Reach::default();
    let mut unreadable = Vec::new();
    for f in flows {
        let (ns, name, version) = (col(f, "namespace"), col(f, "name"), col(f, "version"));
        let doc = s.workflows().get_version(&ns, &name, &version).await.ok().and_then(|r| serde_yaml::from_str::<Value>(&r.raw_yaml).ok());
        match doc {
            Some(d) => scan(&d, &mut reach),
            None => unreadable.push(format!("{ns}/{name}@{version}")),
        }
    }
    json!({
        "hosts": reach.hosts, "mcp_servers": reach.mcp, "llm": reach.llm, "memory": reach.memory,
        "subflows": reach.subflows, "calls": reach.calls, "unreadable": unreadable,
    })
}

/// Per list in a manifest, what `new` adds to `old` and removes.
pub fn diff(old: &Value, new: &Value) -> Value {
    let mut added = Map::new();
    let mut removed = Map::new();
    for k in ["hosts", "mcp_servers", "llm", "memory", "subflows", "calls", "layers"] {
        let items = |v: &Value| -> BTreeSet<String> { v.get(k).and_then(|x| x.as_array()).map(|a| a.iter().map(|i| i.as_str().map(String::from).unwrap_or_else(|| i.to_string())).collect()).unwrap_or_default() };
        let (a, b) = (items(old), items(new));
        let more: Vec<&String> = b.difference(&a).collect();
        let less: Vec<&String> = a.difference(&b).collect();
        if !more.is_empty() {
            added.insert(k.to_string(), json!(more));
        }
        if !less.is_empty() {
            removed.insert(k.to_string(), json!(less));
        }
    }
    json!({ "added": added, "removed": removed })
}

/// A stored version's manifest: its flows' reach plus its layers.
pub(super) async fn of_version(s: &dyn StorageBackend, version: &Value) -> Value {
    let flows = version.get("flows").and_then(|f| f.as_array()).cloned().unwrap_or_default();
    let mut m = of_flows(s, &flows).await;
    m["layers"] = version.get("layers").cloned().unwrap_or(json!([]));
    m
}

/// `GET /agents/{id}/versions/{n}/manifest?against=m`: `{manifest, diff?}`.
pub(super) async fn get_manifest(
    State(s): St, p: P, Path((id, version)): Path<(String, String)>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    both(async {
        agent_space(s.as_ref(), &who(p), &id, false).await?;
        let rows = version_rows(s.as_ref(), &id).await?;
        let find = |v: &str| rows.iter().find(|r| col(r, "version") == v).map(version_json).ok_or_else(|| err(StatusCode::NOT_FOUND, format!("agent {id:?} has no version {v:?}")));
        let manifest = of_version(s.as_ref(), &find(&version)?).await;
        let mut answer = json!({ "manifest": manifest });
        if let Some(other) = q.get("against") {
            answer["diff"] = diff(&of_version(s.as_ref(), &find(other)?).await, &answer["manifest"]);
        }
        ok(answer)
    }.await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flow_says_what_it_reaches() {
        let doc: Value = serde_yaml::from_str(r#"
do:
- a: { call: http, with: { method: get, endpoint: "https://api.example.com/v1/x?y=1" } }
- b: { call: http, with: { endpoint: { uri: "http://internal:8080/z" } } }
- c: { call: http, with: { endpoint: "${ .url }" } }
- d: { call: mcp, with: { server: a3t/tools, tool: t } }
- e: { call: llm, with: { resource: a3t/model } }
- f: { call: adb, with: { _resource: a3t/agent-memory, query: "RECALL" } }
- g: { run: { workflow: { namespace: t, name: child, version: "1" } } }
- nested: { for: { each: i, in: "${ .x }" }, do: [ { h: { call: http, with: { endpoint: "https://deep.example.com" } } } ] }
"#).unwrap();
        let mut r = Reach::default();
        scan(&doc, &mut r);
        let hosts: Vec<&str> = r.hosts.iter().map(String::as_str).collect();
        assert_eq!(hosts, ["${ .url }", "api.example.com", "deep.example.com", "internal:8080"]);
        assert!(r.mcp.contains("a3t/tools") && r.llm.contains("a3t/model") && r.memory.contains("a3t/agent-memory") && r.subflows.contains("t/child"));
    }

    #[test]
    fn a_diff_names_what_changed() {
        let old = json!({ "hosts": ["a.example.com"], "calls": ["http"] });
        let new = json!({ "hosts": ["a.example.com", "b.example.com"], "calls": [] });
        let d = diff(&old, &new);
        assert_eq!(d["added"]["hosts"], json!(["b.example.com"]));
        assert_eq!(d["removed"]["calls"], json!(["http"]));
        assert!(d["added"].get("calls").is_none());
    }
}
