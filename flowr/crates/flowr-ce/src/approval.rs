//! Putting a permission question to the human in the chat, and waiting.
//!
//! When policy returns [`PolicyDecision::Ask`] the agent stops mid-turn, the
//! question is streamed to the conversation as an `authz-request` chunk, and the
//! run suspends on a `oneshot` until the user answers through
//! `POST /inputs/:id`. The answer is recorded in the session ledger, so it binds
//! the rest of the conversation without being asked twice.
//!
//! This reuses the `call:human` suspend/resume machinery rather than inventing a
//! parallel one — an approval IS human input, and routing it through the same
//! `InputManager` means it shows up in the pending-inputs surfaces for free.
//!
//! **Durable on a journaled run** (limitation L1): the question is recorded in
//! the engine store (`ApprovalRequested`, a wait), the run lets go, and the
//! answer — through the same `POST /inputs/:id`, on any console process —
//! journals `ApprovalDecided` and wakes the run, which replays the agent's
//! recorded calls and continues. It survives restarts and works for runs on
//! runtime pods; it waits `FLOWR_APPROVAL_TIMEOUT_SECS` (default a day).
//!
//! A run without a journal (ephemeral, or `FLOWR_STORE=lance`) still waits
//! in memory through `InputManager` for five minutes, as before.

use std::time::Duration;

use flowr_policy::grants::{self, Verdict};
use flowr_policy::{GrantScope, PolicyDecision, Principal};
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

/// How long a question waits before giving up. Long enough for someone to read
/// it and think; short enough that an abandoned tab does not pin a run forever.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

/// Scopes offered to the approver.
///
/// `Agent` is filtered out until persisted grants exist (Phase 4). Offering a
/// button that silently records something narrower than it promises is worse
/// than not offering it.
fn offerable(scopes: &[GrantScope]) -> Vec<GrantScope> {
    scopes
        .iter()
        .copied()
        .filter(|s| !matches!(s, GrantScope::Agent))
        .collect()
}

fn parse_scope(s: Option<&str>) -> GrantScope {
    match s {
        Some("once") => GrantScope::Once,
        Some("agent") => GrantScope::Agent,
        // The dev-time default: broad enough to be useful, gone when the
        // conversation ends.
        _ => GrantScope::Session,
    }
}

/// Ask the human, wait, record the answer, and return the resolved decision.
///
/// Returns `Allow` or `Deny` — never `Ask`, because by the time this returns the
/// question has been settled one way or another.
#[allow(clippy::too_many_arguments)]
pub async fn request_approval(
    session_id: &str,
    principal: &Principal,
    server: Option<&str>,
    tool: &str,
    arguments: &Value,
    reason: &str,
    scopes: &[GrantScope],
) -> PolicyDecision {
    request_approval_with(session_id, principal, server, tool, arguments, reason, scopes, &Ask::default()).await
}

/// What raised a question beyond the policy itself (GOV-7/GOV-8).
#[derive(Debug, Clone, Default)]
pub struct Ask {
    /// The agent approval rule that asked (`<agent>/<tool pattern>`), when one did.
    pub rule: Option<String>,
    /// How long it may wait, from that rule; else `FLOWR_APPROVAL_TIMEOUT_SECS`.
    pub timeout_secs: Option<u64>,
    /// The project the agent works in, for the inbox and the decision log.
    pub project: Option<String>,
    /// Who answers, as the rule names them (`Admin`, `securityadmin`,
    /// `group:finance`); `None`: the person the agent acts for, else its owner.
    pub approver: Option<String>,
}

/// May `p` answer this question (`context.approver`)? Its named person; or,
/// when a rule named a role or group (`rule:<who>`), anyone who holds it where
/// the agent works — never the person the agent acts for (no one approves
/// their own agent's call).
pub async fn may_answer(storage: &dyn flowr_storage::StorageBackend, p: &Principal, req: &Value) -> bool {
    let Some(approver) = req.pointer("/context/approver").and_then(|v| v.as_str()) else { return false };
    if approver == p.id() {
        return true;
    }
    let Some(spec) = approver.strip_prefix("rule:") else { return false };
    let me = flowr_policy::namespace::acting_user(p);
    let asker = req.pointer("/context/on_behalf_of").and_then(|v| v.as_str());
    if me.is_none() || me == asker {
        return false;
    }
    let ns = req.pointer("/context/namespace").and_then(|v| v.as_str());
    let has = |r: &str| p.roles().iter().any(|x| x == r);
    match spec {
        s if s.starts_with("group:") => p.session().is_some_and(|f| f.groups.iter().any(|g| g == &s[6..] || g.trim_start_matches('/') == &s[6..])),
        "SuperAdmin" | "SecurityAdmin" => has(spec),
        "Admin" | "Operator" | "Viewer" => match ns {
            Some(ns) => {
                let access = flowr_policy::namespace::namespace_guard().access(p, ns).await;
                flowr_policy::namespace::tier_roles(access).contains(&spec)
            }
            None => has(spec),
        },
        "admin" | "securityadmin" => {
            let Some(ns) = ns else { return false };
            let org = crate::handlers::call_lancedb::query_platform_table(storage, "projects",
                Some(format!("namespace = {} AND owner_kind = 'org'", crate::handlers::call_lancedb::sql_quote(ns))), 1).await.ok()
                .and_then(|r| r.into_iter().next()).and_then(|r| r.get("owner").and_then(|v| v.as_str()).map(str::to_string));
            let Some(org) = org else { return false };
            let role = crate::policy_changes::org_role(storage, p, &org).await;
            matches!(role.as_str(), "platform" | "securityadmin") || (spec == "admin" && role == "admin")
        }
        // An org's own role (`<org>/<role>`).
        other => has(other),
    }
}

/// Who answers a question raised by `principal`: the person the agent acts
/// for; with nobody behind the run (a trigger, a schedule), the agent's owner.
pub fn approver_for(principal: &Principal) -> Option<String> {
    principal.on_behalf_of().or_else(|| principal.owner()).map(str::to_string)
}

#[allow(clippy::too_many_arguments)]
pub async fn request_approval_with(
    session_id: &str,
    principal: &Principal,
    server: Option<&str>,
    tool: &str,
    arguments: &Value,
    reason: &str,
    scopes: &[GrantScope],
    ask: &Ask,
) -> PolicyDecision {
    let decision_id = flowr_mcp_core::mcp_decision_id(server, tool);
    let key = grants::decision_key(flowr_policy::actions::MCP_CALL, "Tool", &decision_id);
    let offered = offerable(scopes);

    // The approver is the human the agent acts for, else its owner; with
    // nobody at all there is no one to ask, so it is a denial, not a hang.
    // A rule that names its approver (a role or a group) asks them instead.
    let named = ask.approver.as_ref().map(|a| format!("rule:{a}"));
    let Some(approver) = named.or_else(|| approver_for(principal)) else {
        warn!(tool, "approval requested for a principal with no human behind it");
        return PolicyDecision::Deny {
            reason: "approval required, but this run has no interactive approver".into(),
            policy_ids: vec!["no-approver".into()],
        };
    };
    let q = Question { session_id, principal, approver: &approver, server, tool, arguments, reason, decision_id: &decision_id, offered: &offered, ask };

    let answer = match durable_answer(&q).await {
        Some(Durable::Answered(request_id, answer)) => Some((request_id, answer)),
        Some(Durable::Expired(request_id)) => return timed_out(&q, &request_id),
        Some(Durable::Waiting) => {
            // The run has let go; this result is never used.
            return PolicyDecision::Deny { reason: "waiting for approval".into(), policy_ids: vec!["approval-pending".into()] };
        }
        None => None,
    };
    let (request_id, answer) = match answer {
        Some(a) => a,
        None => match in_memory_answer(&q).await {
            Ok(a) => a,
            Err(request_id) => return timed_out(&q, &request_id),
        },
    };
    settle(session_id, &key, &decision_id, &approver, tool, &request_id, &answer)
}

/// One question, as asked.
struct Question<'a> {
    session_id: &'a str,
    principal: &'a Principal,
    approver: &'a str,
    server: Option<&'a str>,
    tool: &'a str,
    arguments: &'a Value,
    reason: &'a str,
    decision_id: &'a str,
    offered: &'a [GrantScope],
    ask: &'a Ask,
}

impl Question<'_> {
    fn timeout_secs(&self) -> i64 {
        self.ask.timeout_secs.map(|s| s as i64).unwrap_or_else(|| {
            std::env::var("FLOWR_APPROVAL_TIMEOUT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(86_400)
        }).max(1)
    }

    /// The question as streamed to the chat and listed in the approvals inbox.
    fn json(&self, request_id: &str, expires_at: chrono::DateTime<chrono::Utc>) -> Value {
        let p = self.principal;
        json!({
            "request_id": request_id,
            "request_type": "authz",
            "title": format!("Allow {}?", self.tool),
            "message": self.reason,
            "options": self.offered.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            "agent_id": p.id(),
            "created_at": chrono::Utc::now().to_rfc3339(),
            "expires_at": expires_at.to_rfc3339(),
            "context": {
                "tool": self.tool,
                "server": self.server,
                "arguments": self.arguments,
                "decision_id": self.decision_id,
                "reason": self.reason,
                // Who may answer. An approval is a security decision, so the
                // endpoint verifies the responder against this rather than
                // trusting whoever holds the request id.
                "approver": self.approver,
                "on_behalf_of": p.on_behalf_of(),
                "agent_owner": p.owner(),
                "session_id": self.session_id,
                "rule": self.ask.rule,
                "namespace": self.ask.project,
                "timeout_secs": self.timeout_secs(),
            },
        })
    }

    /// The question goes into the decision log when first asked (GOV-7).
    fn log(&self, request_id: &str, action: &str, note: &str) {
        flowr_policy::audit::event(self.principal, action, "Approval", request_id, self.ask.project.as_deref(), None, None, Some(note));
    }

    fn asked_note(&self) -> String {
        let mut n = format!("{} · answer by {}", self.tool, self.approver);
        if let Some(r) = &self.ask.rule { n.push_str(&format!(" · rule {r}")); }
        n.push_str(&format!(" · waits {}s", self.timeout_secs()));
        n
    }
}

fn publish_card(session_id: &str, request_id: &str, server: Option<&str>, tool: &str, arguments: &Value, reason: &str, offered: &[GrantScope]) {
    crate::token_stream::publish(
        session_id,
        "authz-request",
        json!({
            "request_id": request_id,
            "tool": tool,
            "server": server,
            "arguments": arguments,
            "reason": reason,
            "scopes": offered.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        })
        .to_string(),
    );
}

enum Durable {
    Answered(String, Value),
    Expired(String),
    Waiting,
}

/// On a journaled run: the recorded answer, or record the question and stop
/// the run until it comes. `None` when the run has no journal.
async fn durable_answer(q: &Question<'_>) -> Option<Durable> {
    if !swf_runtime::effects::journaled() {
        return None;
    }
    let fresh = !swf_runtime::effects::asked("ask");
    let request_id = Uuid::now_v7().to_string();
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(q.timeout_secs());
    let request = q.json(&request_id, expires_at);
    if fresh {
        // Shown once, when first asked; `ask` records it with this id.
        publish_card(q.session_id, &request_id, q.server, q.tool, q.arguments, q.reason, q.offered);
        q.log(&request_id, "approval:request", &q.asked_note());
        info!(session_id = q.session_id, tool = q.tool, request_id, "approval requested; the run waits for it durably");
    }
    match swf_runtime::effects::ask("ask", &request, expires_at).await? {
        Ok(swf_runtime::effects::Answer::Given { request_id, decision, .. }) => Some(Durable::Answered(request_id, decision)),
        Ok(swf_runtime::effects::Answer::Expired { request_id }) => Some(Durable::Expired(request_id)),
        Err(_) => Some(Durable::Waiting),
    }
}

/// Without a journal: wait in this process (`InputManager`), at most five
/// minutes (or the rule's timeout, if shorter).
async fn in_memory_answer(q: &Question<'_>) -> Result<(String, Value), String> {
    let request_id = Uuid::now_v7().to_string();
    let wait = Duration::from_secs((q.timeout_secs() as u64).min(APPROVAL_TIMEOUT.as_secs()));
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(wait.as_secs() as i64);
    let shown = q.json(&request_id, expires_at);
    let request = flowr_ce_agents::InputRequest {
        id: request_id.clone(),
        request_type: "authz".to_string(),
        channel: "chat".to_string(),
        title: Some(format!("Allow {}?", q.tool)),
        message: Some(q.reason.to_string()),
        options: Some(shown["options"].clone()),
        fields: None,
        schema: None,
        // Everything the person needs to judge the request: which tool, on which
        // server, with what arguments. Withholding the arguments would make the
        // prompt unanswerable in any considered way.
        context: Some(shown["context"].clone()),
        workflow: None,
        agent_id: Some(q.principal.id().to_string()),
        created_at: chrono::Utc::now().to_rfc3339(),
        expires_at: expires_at.to_rfc3339(),
        status: flowr_ce_agents::InputStatus::Pending,
    };

    let rx = flowr_ce_agents::input_manager().create_request(request, wait).await;

    // Stream the card into the conversation that raised it.
    publish_card(q.session_id, &request_id, q.server, q.tool, q.arguments, q.reason, q.offered);
    q.log(&request_id, "approval:request", &q.asked_note());
    info!(session_id = q.session_id, tool = q.tool, request_id, "approval requested, agent suspended");

    match tokio::time::timeout(wait, rx).await {
        Ok(Ok(resp)) => Ok((request_id, resp.data)),
        // Timed out, or the channel dropped.
        _ => Err(request_id),
    }
}

/// Nobody answered in time. The question is deliberately NOT recorded as a
/// refusal: remembering a non-answer would silently harden into a permanent
/// "no" the user never chose. This call fails; the next one asks again.
fn timed_out(q: &Question<'_>, request_id: &str) -> PolicyDecision {
    let (session_id, tool) = (q.session_id, q.tool);
    warn!(session_id, tool, request_id, "approval timed out unanswered");
    q.log(request_id, "approval:expired", &format!("{tool} · not answered by {} in {}s", q.approver, q.timeout_secs()));
    crate::token_stream::publish(
        session_id,
        "authz-resolved",
        json!({ "request_id": request_id, "allowed": false, "reason": "timed out" }).to_string(),
    );
    PolicyDecision::Deny {
        reason: format!("approval for {tool} was not answered in time"),
        policy_ids: vec!["approval-timeout".into()],
    }
}

/// Record the answer in the session ledger and turn it into a decision.
fn settle(session_id: &str, key: &str, decision_id: &str, approver: &str, tool: &str, request_id: &str, answer: &Value) -> PolicyDecision {
    let allowed = answer.get("allow").and_then(|v| v.as_bool()).unwrap_or(false);
    let scope = parse_scope(answer.get("scope").and_then(|v| v.as_str()));
    let note = answer
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or(if allowed { "approved in chat" } else { "declined in chat" });

    grants::record(
        session_id,
        grants::answered(
            key.to_string(),
            flowr_policy::actions::MCP_CALL,
            "Tool",
            decision_id,
            if allowed { Verdict::Granted } else { Verdict::Denied },
            scope,
            note,
            approver,
        ),
    );

    // Spend a one-time grant on the call that asked for it.
    //
    // This function returns `Allow` straight to the caller rather than going back
    // through `consult`, so without this the single use was still unspent and the
    // NEXT call consumed it — "just this once" quietly authorized two calls.
    if allowed && scope == GrantScope::Once {
        let _ = grants::consult(session_id, key);
    }

    crate::token_stream::publish(
        session_id,
        "authz-resolved",
        json!({
            "request_id": request_id,
            "allowed": allowed,
            "scope": scope.as_str(),
            "tool": tool,
        })
        .to_string(),
    );
    info!(session_id, tool, allowed, scope = scope.as_str(), "approval answered");

    if allowed {
        PolicyDecision::Allow
    } else {
        PolicyDecision::Deny {
            reason: format!("you declined {tool} in this conversation"),
            policy_ids: vec!["session-denial".into()],
        }
    }
}
