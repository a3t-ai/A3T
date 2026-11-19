//! `a3t.schema.json` is `a3t.cedarschema` in Cedar's JSON form, kept in step by this
//! test. The merged registry schema starts from the JSON one: out of step (actions
//! the policies use missing), every merged schema was rejected and registry schema
//! fragments were silently dropped (vc-dev, 18 Nov: `flow:deploy` unrecognized).
//!
//! After changing `a3t.cedarschema`: `A3T_SCHEMA_JSON_WRITE=1 cargo test -p a3t-security
//! --test schema_json` writes the JSON form.

#[test]
fn the_json_schema_is_the_cedar_schema() {
    let (fragment, _) = cedar_policy::SchemaFragment::from_cedarschema_str(a3t_security::A3T_SCHEMA).expect("a3t.cedarschema parses");
    let want = fragment.to_json_value().expect("as JSON");
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../a3t/policies/a3t.schema.json");
    if std::env::var("A3T_SCHEMA_JSON_WRITE").is_ok() {
        std::fs::write(path, serde_json::to_string_pretty(&want).unwrap() + "\n").unwrap();
    }
    let have: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).expect("a3t.schema.json is JSON");
    assert_eq!(have, want, "a3t.schema.json is out of step with a3t.cedarschema: regenerate it (see this file's header)");
}
