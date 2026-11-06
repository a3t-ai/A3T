//! Inheriting from a base flow (UX ST-1, AC-2; spec §4.2).
//!
//! `metadata.a3t.extends: <namespace>/<name>@<version>` on a flow means: this flow runs
//! with the base's hooks (`use.extensions`) and envelope (`metadata.a3t`) around its own
//! tasks. Each flow's hooks cover only its own tasks and an uncaught failure goes to its
//! own `onError`, so inheriting has to be declared per flow; the conformance check lists
//! the flows that don't.
//!
//! The base binds the child:
//! - tool sets (`exposedTools`, `enabledTools`) intersect: a child can only narrow them;
//! - `metadata.a3t.locked` names what a child can't override: `a3t` keys, and hooks as
//!   `extension:<name>`. Overriding one is refused, at save and at run;
//! - anything else the child sets wins, and what it doesn't set comes from the base.
//!
//! `durability` is not inherited: the engine reads it from the flow's own document.

use serde_json::{Map, Value};

use crate::WorkflowDefinition;

/// Compose a template over the one it inherits, attenuate-only: the capability sets
/// intersect, so the result can narrow them but never widen them, and a set left out is
/// inherited. Other fields take `current`'s value, falling back to `inherited`'s. Used
/// for `extends` and when a flow runs a subflow.
pub fn compose_template(inherited: Option<Value>, current: Value) -> Value {
    let Some(parent) = inherited.as_ref().and_then(|v| v.as_object()) else {
        return current;
    };
    let mut result = current.as_object().cloned().unwrap_or_default();
    for key in ["exposedTools", "enabledTools"] {
        let cur = result.get(key).and_then(|v| v.as_array()).cloned();
        let par = parent.get(key).and_then(|v| v.as_array()).cloned();
        match (cur, par) {
            (Some(cur), Some(par)) => {
                let inter: Vec<Value> = cur.into_iter().filter(|x| par.contains(x)).collect();
                result.insert(key.to_string(), Value::Array(inter));
            }
            (None, Some(par)) => {
                result.insert(key.to_string(), Value::Array(par));
            }
            _ => {}
        }
    }
    for (k, v) in parent {
        result.entry(k.clone()).or_insert_with(|| v.clone());
    }
    Value::Object(result)
}

/// The `<namespace>, <name>, <version>` a flow extends, if it does. A malformed pin is an
/// error: guessing would leave the flow running without its base.
pub fn extends_of(def: &WorkflowDefinition) -> Result<Option<(String, String, String)>, String> {
    let Some(v) = def.metadata.as_ref().and_then(|m| m.pointer("/a3t/extends")) else {
        return Ok(None);
    };
    let bad = || format!("metadata.a3t.extends: {v} is not <namespace>/<name>@<version>");
    let s = v.as_str().ok_or_else(bad)?;
    let (path, version) = s.split_once('@').ok_or_else(bad)?;
    let (ns, name) = path.split_once('/').ok_or_else(bad)?;
    if ns.is_empty() || name.is_empty() || version.is_empty() {
        return Err(bad());
    }
    Ok(Some((ns.to_string(), name.to_string(), version.to_string())))
}

fn a3t_of(def: &WorkflowDefinition) -> Map<String, Value> {
    def.metadata.as_ref().and_then(|m| m.get("a3t")).and_then(|a| a.as_object()).cloned().unwrap_or_default()
}

fn locked_of(a3t: &Map<String, Value>) -> Vec<String> {
    a3t.get("locked").and_then(|l| l.as_array()).map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// An extension's name: `use.extensions` entries are single-key maps.
fn extension_name(e: &Value) -> Option<&str> {
    e.as_object().filter(|m| m.len() == 1).and_then(|m| m.keys().next()).map(String::as_str)
}

/// `child` with `base`'s hooks and envelope applied. `base` is already resolved.
pub fn apply_base(mut child: WorkflowDefinition, base: &WorkflowDefinition) -> Result<WorkflowDefinition, String> {
    let base_a3t = a3t_of(base);
    let child_a3t = a3t_of(&child);
    let locked = locked_of(&base_a3t);
    let flow = format!("{}/{}", child.document.namespace, child.document.name);
    let from = format!("{}/{}", base.document.namespace, base.document.name);

    // Locked keys. Tool sets can be narrowed under any lock: they only intersect.
    for key in locked.iter().filter(|k| !k.starts_with("extension:") && !matches!(k.as_str(), "exposedTools" | "enabledTools")) {
        if let (Some(c), Some(b)) = (child_a3t.get(key), base_a3t.get(key)) {
            if c != b {
                return Err(format!("{flow} can't override {key}: {from} locks it"));
            }
        }
    }

    // Hooks: the base's first, so they wrap the child's; a child's of the same name
    // replaces an unlocked one in its place.
    let child_use = child.use_block.take();
    let base_ext = base.use_block.as_ref().map(|u| u.extensions.clone()).unwrap_or_default();
    let mut own = child_use.as_ref().map(|u| u.extensions.clone()).unwrap_or_default();
    let mut merged = Vec::with_capacity(base_ext.len() + own.len());
    for b in base_ext {
        let name = extension_name(&b).map(str::to_string);
        let same = name.as_deref().and_then(|n| own.iter().position(|e| extension_name(e) == Some(n)));
        match (same, name) {
            (Some(_), Some(n)) if locked.iter().any(|l| l.strip_prefix("extension:") == Some(n.as_str())) => {
                return Err(format!("{flow} can't override hook {n}: {from} locks it"));
            }
            (Some(i), _) => merged.push(own.remove(i)),
            (None, _) => merged.push(b),
        }
    }
    merged.extend(own);
    if let Some(mut u) = child_use.or_else(|| base.use_block.clone()) {
        u.extensions = merged;
        child.use_block = Some(u);
    }

    // The envelope.
    let mut inherited = base_a3t;
    inherited.remove("durability");
    inherited.remove("extends");
    inherited.remove("base"); // being a base isn't inherited
    let mut a3t = compose_template(Some(Value::Object(inherited)), Value::Object(child_a3t));
    // Locks accumulate down the chain.
    let mut all = locked;
    if let Some(m) = a3t.as_object() {
        for l in locked_of(m) {
            if !all.contains(&l) {
                all.push(l);
            }
        }
    }
    if !all.is_empty() {
        a3t["locked"] = Value::from(all);
    }
    let mut meta = child.metadata.take().unwrap_or_else(|| Value::Object(Map::new()));
    meta["a3t"] = a3t;
    child.metadata = Some(meta);
    Ok(child)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_yaml;
    use serde_json::json;

    fn flow(yaml_rest: &str) -> WorkflowDefinition {
        parse_yaml(&format!("document: {{ dsl: 1.0.3, namespace: a3t, name: child, version: 1.0.0 }}\n{yaml_rest}\ndo:\n- x:\n    set: {{ a: 1 }}\n")).unwrap()
    }
    const BASE: &str = r#"
use:
  extensions:
  - audit: { extend: all, after: [] }
  - wording: { extend: all, after: [] }
metadata:
  a3t:
    base: true
    onError: system/a3t-error-handler
    policyRef: strict
    durability: transient
    exposedTools: [read, write]
    locked: [onError, policyRef, 'extension:audit']
"#;
    fn names(d: &WorkflowDefinition) -> Vec<String> {
        d.use_block.as_ref().unwrap().extensions.iter().map(|e| extension_name(e).unwrap().to_string()).collect()
    }

    #[test]
    fn a_child_gets_the_hooks_and_envelope_and_can_only_narrow() {
        let child = flow("use:\n  extensions:\n  - mine: { extend: all, after: [] }\nmetadata:\n  a3t:\n    extends: system/base@1\n    exposedTools: [read, delete]\n    durability: ephemeral");
        let d = apply_base(child, &flow(BASE)).unwrap();
        assert_eq!(names(&d), ["audit", "wording", "mine"], "the base's hooks come first");
        let a = &d.metadata.as_ref().unwrap()["a3t"];
        assert_eq!(a["onError"], "system/a3t-error-handler");
        assert_eq!(a["exposedTools"], json!(["read"]), "delete can't be added");
        assert_eq!(a["durability"], "ephemeral");
        assert_eq!(a["locked"].as_array().unwrap().len(), 3);
        assert!(a.get("base").is_none());
    }

    #[test]
    fn durability_is_not_inherited_and_silence_keeps_the_tools() {
        let d = apply_base(flow("metadata:\n  a3t:\n    extends: system/base@1"), &flow(BASE)).unwrap();
        let a = &d.metadata.as_ref().unwrap()["a3t"];
        assert!(a.get("durability").is_none());
        assert_eq!(a["exposedTools"], json!(["read", "write"]));
    }

    #[test]
    fn locked_items_can_not_be_overridden() {
        let e = apply_base(flow("metadata:\n  a3t:\n    onError: a3t/mine"), &flow(BASE)).unwrap_err();
        assert!(e.contains("can't override onError"), "{e}");
        let e = apply_base(flow("use:\n  extensions:\n  - audit: { extend: all, after: [] }"), &flow(BASE)).unwrap_err();
        assert!(e.contains("can't override hook audit"), "{e}");
        // Restating the locked value isn't overriding it, and an open hook can be replaced.
        let ok = flow("use:\n  extensions:\n  - wording: { extend: call, after: [] }\nmetadata:\n  a3t:\n    onError: system/a3t-error-handler");
        let d = apply_base(ok, &flow(BASE)).unwrap();
        assert_eq!(names(&d), ["audit", "wording"]);
        assert_eq!(d.use_block.as_ref().unwrap().extensions[1]["wording"]["extend"], "call");
    }

    #[test]
    fn extends_is_parsed_strictly() {
        let some = flow("metadata:\n  a3t:\n    extends: system/a3t-base@1.0.0");
        assert_eq!(extends_of(&some).unwrap().unwrap(), ("system".into(), "a3t-base".into(), "1.0.0".into()));
        assert_eq!(extends_of(&flow("")).unwrap(), None);
        for bad in ["system/a3t-base", "a3t-base@1", "/x@1", "system/x@"] {
            assert!(extends_of(&flow(&format!("metadata:\n  a3t:\n    extends: {bad}"))).is_err(), "{bad}");
        }
    }
}
