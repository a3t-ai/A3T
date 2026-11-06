//! Loading a flow with its base (UX ST-1, AC-2/AC-3): `metadata.a3t.extends` resolved
//! through the registry, at a pinned version. Every place that runs a flow loads it here,
//! so a base can't be skipped by the way a flow is started; `create_flow` loads it too,
//! so a flow that overrides a lock is refused when it is saved.

use flowr_storage::StorageBackend;
use flowr_types::swf_parser::{extends, WorkflowDefinition};
use flowr_types::WorkflowRecord;
use serde_json::{json, Value};

/// A chain deeper than this is a mistake: system → A3T → product is three.
const MAX_DEPTH: usize = 8;

/// `record`'s definition with its base chain applied. A base that is missing or broken
/// is an error: a flow never runs without the base it declares.
pub async fn load_definition(storage: &dyn StorageBackend, record: &WorkflowRecord) -> Result<WorkflowDefinition, String> {
    let first = record.parse().map_err(|e| e.to_string())?;
    let mut chain = vec![first];
    let mut seen = vec![format!("{}/{}", record.namespace(), record.name())];
    while let Some((ns, name, version)) = extends::extends_of(chain.last().expect("never empty"))? {
        let id = format!("{ns}/{name}");
        if seen.contains(&id) {
            return Err(format!("{} extends itself through {id}", seen[0]));
        }
        if chain.len() >= MAX_DEPTH {
            return Err(format!("{}: more than {MAX_DEPTH} bases", seen[0]));
        }
        let base = storage.workflows().get_version(&ns, &name, &version).await
            .map_err(|e| format!("{} extends {id}@{version}, which can't be loaded: {e}", seen[0]))?;
        chain.push(base.parse().map_err(|e| format!("base {id}@{version}: {e}"))?);
        seen.push(id);
    }
    let mut resolved = chain.pop().expect("never empty");
    while let Some(child) = chain.pop() {
        resolved = extends::apply_base(child, &resolved)?;
    }
    Ok(resolved)
}

/// AC-4: a released version never changes. Saving the same version with other content is
/// refused (the message starts with the stable code `version_exists`); saving it with
/// the same content changes nothing and is fine, so a repeated deploy stays harmless.
pub async fn check_version(storage: &dyn StorageBackend, record: &WorkflowRecord) -> Result<(), String> {
    match storage.workflows().get_version(record.namespace(), record.name(), record.version()).await {
        Ok(old) if old.raw_yaml != record.raw_yaml => Err(format!(
            "version_exists: {}/{} version {} is already released with other content: give this one a new version",
            record.namespace(),
            record.name(),
            record.version()
        )),
        _ => Ok(()),
    }
}

/// The namespaces whose flows must be on a base.
pub const PLATFORM_NAMESPACES: &[&str] = &["a3t", "system"];

/// Which platform flows are on a base. `covered`: extends one and loads. `uncovered`: no
/// base, and not exempt (a base itself, or `conformanceExempt: <why>`). `broken`: declares
/// a base but can't load.
pub async fn conformance(storage: &dyn StorageBackend) -> Value {
    let (mut covered, mut uncovered, mut exempt, mut broken) = (vec![], vec![], vec![], vec![]);
    for ns in PLATFORM_NAMESPACES {
        let flows = match storage.workflows().list(ns).await {
            Ok(f) => f,
            Err(e) => {
                broken.push(json!({ "flow": format!("{ns}/*"), "why": e.to_string() }));
                continue;
            }
        };
        for f in flows {
            let id = format!("{ns}/{}", f.name);
            let record = match storage.workflows().get(ns, &f.name).await {
                Ok(r) => r,
                Err(e) => {
                    broken.push(json!({ "flow": id, "why": e.to_string() }));
                    continue;
                }
            };
            let def = match record.parse() {
                Ok(d) => d,
                Err(e) => {
                    broken.push(json!({ "flow": id, "why": e.to_string() }));
                    continue;
                }
            };
            let a3t = def.metadata.as_ref().and_then(|m| m.get("a3t")).cloned().unwrap_or(Value::Null);
            if a3t.get("extends").is_some() {
                match load_definition(storage, &record).await {
                    Ok(_) => covered.push(id),
                    Err(why) => broken.push(json!({ "flow": id, "why": why })),
                }
            } else if a3t.get("base").and_then(|b| b.as_bool()) == Some(true) || a3t.get("conformanceExempt").is_some() {
                exempt.push(id);
            } else {
                uncovered.push(id);
            }
        }
    }
    json!({ "covered": covered, "uncovered": uncovered, "exempt": exempt, "broken": broken })
}
