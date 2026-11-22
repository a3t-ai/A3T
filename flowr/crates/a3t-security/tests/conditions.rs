//! Rule conditions and rule kinds (governance round 2): exact role levels,
//! people only / agents acting for a role, time, dates, end dates, address,
//! channel, sign-in, groups, people, destructive tools, call arguments,
//! untrusted input, personal data, "allow only when", exceptions, approvers,
//! rules that only log, and counted rules (rate, first use) — compiled from
//! YAML and decided by the hook.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use a3t_security::policy_files::{compile_org_set, compile_policy, OrgFile};
use a3t_security::CedarHook;
use flowr_policy::conditions::{civil, ConditionSource, CountKey};
use flowr_policy::{actions, AgentId, ExecutionRequest, PolicyDecision, PolicyHook, Principal, Resource, SessionFacts};

/// A condition source for tests: UTC, a deploy 10 minutes ago, personal data
/// when an argument holds an '@', and counts kept in memory.
#[derive(Default)]
struct Fake {
    hits: Mutex<HashMap<String, u64>>,
    seen: Mutex<HashSet<String>>,
}

#[async_trait::async_trait]
impl ConditionSource for Fake {
    async fn since_deploy_s(&self, _ns: Option<&str>) -> Option<i64> {
        Some(600)
    }
    fn args_pii(&self, params: &serde_json::Value) -> bool {
        params.to_string().contains('@')
    }
    async fn over(&self, spec: &serde_json::Value, key: &CountKey) -> Result<bool, String> {
        let who = key.person.clone().unwrap_or_default();
        match spec["kind"].as_str() {
            Some("rate") => Ok(*self.hits.lock().unwrap().get(&format!("{}|{who}", key.rule)).unwrap_or(&0) >= spec["n"].as_u64().unwrap()),
            Some("first_use") => Ok(!self.seen.lock().unwrap().contains(&format!("{}|{who}|{}", key.rule, key.resource))),
            Some("refusals") => Ok(who == "locked-out"),
            Some("tokens") => Ok(key.agent.as_deref() == Some("spender")),
            _ => Err("unknown count".into()),
        }
    }
    async fn record(&self, spec: &serde_json::Value, key: &CountKey) {
        let who = key.person.clone().unwrap_or_default();
        match spec["kind"].as_str() {
            Some("rate") => *self.hits.lock().unwrap().entry(format!("{}|{who}", key.rule)).or_default() += 1,
            Some("first_use") => { self.seen.lock().unwrap().insert(format!("{}|{who}|{}", key.rule, key.resource)); }
            _ => {}
        }
    }
}

fn install() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| flowr_policy::conditions::set_condition_source(Arc::new(Fake::default())));
}

fn person(id: &str, roles: &[&str], s: Option<SessionFacts>) -> Principal {
    Principal::Human { session: s.map(Box::new), memory_branch: None, impersonated_by: None, user_id: id.into(), roles: roles.iter().map(|r| r.to_string()).collect() }
}

fn agent(id: &str, for_whom: &str, roles: &[&str], s: Option<SessionFacts>) -> Principal {
    Principal::Agent { session: s.map(Box::new), memory_branch: None, agent_id: AgentId(id.into()), on_behalf_of: Some(for_whom.into()),
        flow_id: None, roles: roles.iter().map(|r| r.to_string()).collect(), owner: Some(for_whom.into()) }
}

fn call(p: Principal, tool: &str) -> ExecutionRequest {
    ExecutionRequest { principal: p, action: actions::tool_invoke(), resource: Resource::tool(tool),
        context: Default::default(), attributes: Default::default(), params: serde_json::json!({}) }
}

fn with_params(mut r: ExecutionRequest, v: serde_json::Value) -> ExecutionRequest {
    r.params = v;
    r
}

fn rules(yaml_rules: &str) -> CedarHook {
    install();
    let y = format!("kind: a3t/policy\nname: t\napplies_to: {{ org: true }}\nrules:\n{yaml_rules}");
    let c = compile_policy("acme", &y).unwrap_or_else(|e| panic!("compiles: {e}\n{y}"));
    CedarHook::with_registry_policies(&[c.cedar], None, &[]).expect("loads")
}

fn err(yaml_rules: &str) -> String {
    compile_policy("acme", &format!("kind: a3t/policy\nname: t\napplies_to: {{ org: true }}\nrules:\n{yaml_rules}")).unwrap_err()
}

fn refused(d: &PolicyDecision) -> bool {
    matches!(d, PolicyDecision::Deny { .. })
}

fn asked(d: &PolicyDecision) -> Option<flowr_policy::RequiredApproval> {
    match d { PolicyDecision::Ask { required: Some(r), .. } => Some(r.clone()), _ => None }
}

const OP: &[&str] = &["Operator", "Viewer"];
const ADMIN: &[&str] = &["Admin", "Operator", "Viewer"];

fn mfa() -> SessionFacts {
    SessionFacts { second_factor: true, ..Default::default() }
}

#[tokio::test]
async fn a_role_means_that_level_only() {
    let h = rules("  - who: { roles: [Viewer] }\n    cannot: [tools.tidy_*]\n    reason: Viewers don't delete.\n");
    assert!(refused(&h.check(&call(person("v", &["Viewer"], None), "tidy_x")).await));
    assert_eq!(h.check(&call(person("o", OP, None), "tidy_x")).await, PolicyDecision::Allow, "an Operator also holds Viewer, but is not a Viewer");
    assert_eq!(h.check(&call(person("a", ADMIN, None), "tidy_x")).await, PolicyDecision::Allow);
}

#[tokio::test]
async fn people_only_and_agents_acting_for_a_role() {
    let p = rules("  - who: { roles: [Operator], people_only: true }\n    cannot: [tools.tidy_*]\n    reason: r\n");
    assert!(refused(&p.check(&call(person("o", OP, None), "tidy_x")).await));
    assert_eq!(p.check(&call(agent("ag", "o", OP, None), "tidy_x")).await, PolicyDecision::Allow);
    let a = rules("  - who: { roles: [Operator], agents: acting }\n    cannot: [tools.tidy_*]\n    reason: r\n");
    assert!(refused(&a.check(&call(agent("ag", "o", OP, None), "tidy_x")).await));
    assert_eq!(a.check(&call(person("o", OP, None), "tidy_x")).await, PolicyDecision::Allow);
    assert_eq!(a.check(&call(agent("ag", "a", ADMIN, None), "tidy_x")).await, PolicyDecision::Allow, "acting for an Admin");
    assert!(err("  - who: { agents: acting }\n    cannot: [tools.x]\n    reason: r\n").contains("acting needs roles"));
}

fn now_parts() -> (i64, i64, i64) {
    civil(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64)
}

fn hm(m: i64) -> String {
    let m = m.rem_euclid(1440);
    format!("{:02}:{:02}", m / 60, m % 60)
}

#[tokio::test]
async fn time_of_day_dates_and_end_dates() {
    let (day, _, minute) = now_parts();
    // A window around now (all days), and one that ended an hour ago.
    let around = format!("  - cannot: [tools.deploy]\n    when: [{{ hours: {{ days: mon-sun, from: \"{}\", to: \"{}\" }} }}]\n    reason: r\n", hm(minute - 30), hm(minute + 30));
    if minute >= 30 && minute < 1410 {
        assert!(refused(&rules(&around).check(&call(person("o", OP, None), "deploy")).await));
        let past = format!("  - cannot: [tools.deploy]\n    when: [{{ hours: {{ from: \"{}\", to: \"{}\" }} }}]\n    reason: r\n", hm(minute - 30), hm(minute - 10));
        assert_eq!(rules(&past).check(&call(person("o", OP, None), "deploy")).await, PolicyDecision::Allow);
    }
    let d = |n: i64| format!("{}-{:02}-{:02}", n / 10_000, n / 100 % 100, n % 100);
    let freeze = format!("  - cannot: [tools.deploy]\n    when: [{{ dates: {{ from: \"{}\", to: \"{}\" }} }}]\n    reason: Freeze.\n", d(day), d(day));
    assert!(refused(&rules(&freeze).check(&call(person("o", OP, None), "deploy")).await));
    let ended = "  - cannot: [tools.deploy]\n    until: \"2020-01-01\"\n    reason: Old.\n";
    assert_eq!(rules(ended).check(&call(person("o", OP, None), "deploy")).await, PolicyDecision::Allow, "past its end date a rule is inert");
    let open = "  - cannot: [tools.deploy]\n    until: \"2999-12-31\"\n    reason: Still.\n";
    assert!(refused(&rules(open).check(&call(person("o", OP, None), "deploy")).await));
    assert!(err("  - cannot: [tools.x]\n    when: [{ hours: { from: \"09:00\", to: \"18:00\", tz: Europe/Berlin } }]\n    reason: r\n").contains("time zone"));
    let deploy = "  - cannot: [tools.restart]\n    when: [{ after_deploy: { min: 30 } }]\n    reason: Settle first.\n";
    assert!(refused(&rules(deploy).check(&call(person("o", OP, None), "restart")).await), "deployed 10 minutes ago");
}

#[tokio::test]
async fn allowed_only_when_with_a_second_factor() {
    let h = rules("  - only: [tools.tidy_*]\n    allowed_only_when: [mfa]\n    reason: Deleting needs a second factor.\n");
    assert!(refused(&h.check(&call(person("o", OP, None), "tidy_x")).await), "no session facts: refused");
    assert!(refused(&h.check(&call(person("o", OP, Some(SessionFacts::default())), "tidy_x")).await));
    assert_eq!(h.check(&call(person("o", OP, Some(mfa())), "tidy_x")).await, PolicyDecision::Allow);
    assert_eq!(h.check(&call(agent("ag", "o", OP, Some(mfa())), "tidy_x")).await, PolicyDecision::Allow, "an agent carries its caller's sign-in");
    assert_eq!(h.check(&call(person("o", OP, None), "list_x")).await, PolicyDecision::Allow, "other calls are untouched");
}

#[tokio::test]
async fn otherwise_ask_asks_the_named_approver() {
    let h = rules("  - only: [tools.tidy_*]\n    allowed_only_when: [mfa]\n    otherwise: ask\n    approver: Admin\n    timeout: 4h\n    reason: r\n");
    let a = asked(&h.check(&call(agent("ag", "o", OP, None), "tidy_x")).await).expect("an agent waits");
    assert_eq!((a.timeout_secs, a.approver.as_deref()), (Some(14_400), Some("Admin")));
    assert!(refused(&h.check(&call(person("o", OP, None), "tidy_x")).await), "a person's own call is refused");
    assert_eq!(h.check(&call(agent("ag", "o", OP, Some(mfa())), "tidy_x")).await, PolicyDecision::Allow);
    assert!(err("  - cannot: [tools.x]\n    approver: Admin\n    reason: r\n").contains("approver"));
    assert!(err("  - only: [tools.x]\n    reason: r\n").contains("allowed_only_when"));
}

#[tokio::test]
async fn exceptions_groups_people_and_acting_as() {
    let h = rules("  - cannot: [tools.deploy]\n    unless: [{ group: { name: oncall } }, { person: { email: Jane@Acme.com } }]\n    reason: Freeze.\n");
    let s = |groups: &[&str], email: &str| Some(SessionFacts { groups: groups.iter().map(|g| g.to_string()).collect(), email: Some(email.into()), ..Default::default() });
    assert!(refused(&h.check(&call(person("o", OP, s(&["staff"], "sam@acme.com")), "deploy")).await));
    assert_eq!(h.check(&call(person("o", OP, s(&["oncall"], "sam@acme.com")), "deploy")).await, PolicyDecision::Allow);
    assert_eq!(h.check(&call(person("j", OP, s(&[], "jane@acme.com")), "deploy")).await, PolicyDecision::Allow);
    let imp = rules("  - cannot: [tools.tidy_*]\n    when: [acting_as]\n    reason: Not while acting as someone.\n");
    let mut p = person("o", OP, None);
    if let Principal::Human { impersonated_by, .. } = &mut p { *impersonated_by = Some("root".into()); }
    assert!(refused(&imp.check(&call(p, "tidy_x")).await));
    assert_eq!(imp.check(&call(person("o", OP, None), "tidy_x")).await, PolicyDecision::Allow);
}

#[tokio::test]
async fn address_networks_and_channel() {
    install();
    let files = vec![
        OrgFile { path: "networks/office.yaml".into(), content: "kind: a3t/network\nname: office\nranges: [10.20.0.0/16, 192.0.2.7]\n".into() },
        OrgFile { path: "policies/net.yaml".into(), content: "kind: a3t/policy\nname: net\napplies_to: { org: true }\nrules:\n  - only: [tools.change_*]\n    allowed_only_when: [{ network: { name: office } }]\n    reason: From the office.\n  - cannot: [tools.export_*]\n    when: [{ via: { via: api-key } }, { ip: { cidr: 203.0.113.0/24 } }]\n    reason: r\n".into() },
    ];
    let set = compile_org_set("acme", &["p1".into()], &files).expect("compiles");
    let cedar: Vec<String> = set.policies.iter().map(|(_, _, c)| c.clone()).collect();
    let h = CedarHook::with_registry_policies(&cedar, None, &[]).unwrap();
    let at = |ip: &str, ch: &str| Some(SessionFacts { ip: Some(ip.into()), channel: Some(ch.into()), ..Default::default() });
    assert_eq!(h.check(&call(person("o", OP, at("10.20.3.4", "console")), "change_x")).await, PolicyDecision::Allow);
    assert_eq!(h.check(&call(person("o", OP, at("192.0.2.7", "console")), "change_x")).await, PolicyDecision::Allow);
    assert!(refused(&h.check(&call(person("o", OP, at("8.8.8.8", "console")), "change_x")).await));
    assert!(refused(&h.check(&call(person("o", OP, at("203.0.113.9", "api-key")), "export_x")).await));
    assert_eq!(h.check(&call(person("o", OP, at("203.0.113.9", "console")), "export_x")).await, PolicyDecision::Allow);
    let bad = compile_org_set("acme", &["p1".into()], &[OrgFile { path: "policies/x.yaml".into(),
        content: "kind: a3t/policy\nname: x\napplies_to: { org: true }\nrules:\n  - only: [tools.x]\n    allowed_only_when: [{ network: { name: vpn } }]\n    reason: r\n".into() }]).unwrap_err();
    assert!(bad[0].contains("no networks/ file"), "{bad:?}");
}

#[tokio::test]
async fn what_is_in_the_call() {
    let h = rules("  - cannot: [tools.pay_*]\n    when: [{ arg_over: { arg: amount, n: 1000 } }]\n    reason: Big payments.\n  - cannot: [tools.send_*]\n    when: [pii]\n    reason: No personal data out.\n  - cannot: [tools.*]\n    when: [destructive]\n    reason: No destructive tools.\n");
    let pay = |v: serde_json::Value| with_params(call(person("o", OP, None), "pay_invoice"), v);
    assert!(refused(&h.check(&pay(serde_json::json!({ "amount": 1500 }))).await));
    assert!(refused(&h.check(&pay(serde_json::json!({ "arguments": { "amount": 1000.5 } }))).await));
    assert_eq!(h.check(&pay(serde_json::json!({ "amount": 500 }))).await, PolicyDecision::Allow);
    assert_eq!(h.check(&pay(serde_json::json!({}))).await, PolicyDecision::Allow, "no such argument: not over");
    let send = |v: serde_json::Value| with_params(call(person("o", OP, None), "send_mail"), v);
    assert!(refused(&h.check(&send(serde_json::json!({ "to": "jane@acme.com" }))).await));
    assert_eq!(h.check(&send(serde_json::json!({ "to": "the team" }))).await, PolicyDecision::Allow);
    let mut d = call(person("o", OP, None), "wipe");
    d.resource = Resource::tool("wipe").with_attr("destructive", true);
    assert!(refused(&h.check(&d).await));
    let u = rules("  - needs_approval: [tools.send_*]\n    when: [untrusted]\n    reason: After untrusted input a person checks.\n");
    let mut r = call(agent("ag", "o", OP, None), "send_mail");
    r.attributes.insert("context".into(), serde_json::json!({ "untrusted_seen": true }));
    assert!(asked(&u.check(&r).await).is_some());
    assert_eq!(u.check(&call(agent("ag", "o", OP, None), "send_mail")).await, PolicyDecision::Allow);
}

#[tokio::test]
async fn counted_rules_rate_and_first_use() {
    let h = rules("  - cannot: [tools.run_report]\n    when: [{ rate: { n: 2, per: hour, by: person } }]\n    reason: Two an hour.\n");
    let c = || call(person("rate-user", OP, None), "run_report");
    assert_eq!(h.check(&c()).await, PolicyDecision::Allow);
    assert_eq!(h.check(&c()).await, PolicyDecision::Allow);
    assert!(refused(&h.check(&c()).await), "the third in the hour is refused");
    assert_eq!(h.check(&call(person("someone-else", OP, None), "run_report")).await, PolicyDecision::Allow, "counted per person");
    let f = rules("  - needs_approval: [tools.*]\n    when: [first_use]\n    reason: A person sees an agent's first use of a tool.\n");
    assert!(asked(&f.check(&call(agent("ag", "first-user", OP, None), "fetch")).await).is_some());
    assert_eq!(f.check(&call(agent("ag", "first-user", OP, None), "fetch")).await, PolicyDecision::Allow, "asked once");
    let l = rules("  - cannot: [tools.*]\n    when: [{ refusals: { n: 3, mins: 10, block: 30 } }]\n    reason: Locked out.\n");
    assert!(refused(&l.check(&call(person("locked-out", OP, None), "x")).await));
    assert_eq!(l.check(&call(person("fine", OP, None), "x")).await, PolicyDecision::Allow);
    assert!(err("  - only: [tools.x]\n    allowed_only_when: [{ rate: { n: 1 } }]\n    reason: r\n").contains("counted"));
}

#[tokio::test]
async fn a_rule_that_only_logs_never_refuses() {
    let h = rules("  - cannot: [tools.tidy_*]\n    log_only: true\n    reason: Watching deletes.\n");
    assert_eq!(h.check(&call(person("o", OP, None), "tidy_x")).await, PolicyDecision::Allow);
    assert!(err("  - needs_approval: [tools.x]\n    log_only: true\n    reason: r\n").contains("only logs"));
}

#[test]
fn the_compiled_cedar_is_returned_per_file() {
    let set = compile_org_set("acme", &["p1".into()], &[OrgFile { path: "policies/x.yaml".into(),
        content: "kind: a3t/policy\nname: x\napplies_to: { org: true }\nrules:\n  - cannot: [tools.x]\n    reason: r\n".into() }]).unwrap();
    assert!(set.files[0]["cedar"].as_str().unwrap().contains("forbid("));
}

