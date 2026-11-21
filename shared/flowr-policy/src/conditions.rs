//! What rule conditions need from outside the policy engine: the org's time
//! zone, the project's last deploy, whether a call's arguments carry personal
//! data, and the counts A3T keeps (rate limits, lockouts, first use, tokens).
//!
//! The engine (`a3t-security`) asks the installed [`ConditionSource`]; FlowR
//! installs one backed by its platform tables and the decision log, so counts
//! hold across every FlowR. Without one, no count can be read, and a rule that
//! needs one refuses (fail closed) rather than guessing.

use async_trait::async_trait;
use std::sync::{Arc, OnceLock, RwLock};

/// Whose calls a count is about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CountKey {
    /// The rule's id (`<org>/<policy>/r<n>…`).
    pub rule: String,
    /// The person (the caller, or the one an agent acts for).
    pub person: Option<String>,
    /// The agent making the call.
    pub agent: Option<String>,
    /// The project the call touches.
    pub namespace: Option<String>,
    /// `<Type>::<id>` of what is called, and the action.
    pub resource: String,
    pub action: String,
}

#[async_trait]
pub trait ConditionSource: Send + Sync {
    /// Minutes to add to UTC for local time in the org of `namespace` (now).
    async fn utc_offset_minutes(&self, _namespace: Option<&str>) -> i32 {
        0
    }
    /// Seconds since the project's last deploy, if known.
    async fn since_deploy_s(&self, _namespace: Option<&str>) -> Option<i64> {
        None
    }
    /// The call's arguments carry personal data.
    fn args_pii(&self, _params: &serde_json::Value) -> bool {
        false
    }
    /// Is a counted condition (`{"kind": "rate", …}`) met for this call?
    async fn over(&self, _spec: &serde_json::Value, _key: &CountKey) -> Result<bool, String> {
        Err("A3T keeps no counts in this process".into())
    }
    /// Count this call (it was not over).
    async fn record(&self, _spec: &serde_json::Value, _key: &CountKey) {}
}

/// No source: time in UTC, no counts.
pub struct NoConditions;

#[async_trait]
impl ConditionSource for NoConditions {}

fn slot() -> &'static RwLock<Arc<dyn ConditionSource>> {
    static SLOT: OnceLock<RwLock<Arc<dyn ConditionSource>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(Arc::new(NoConditions)))
}

/// The installed source.
pub fn condition_source() -> Arc<dyn ConditionSource> {
    slot().read().unwrap().clone()
}

/// Install a source (FlowR, at start).
pub fn set_condition_source(s: Arc<dyn ConditionSource>) {
    *slot().write().unwrap() = s;
}

/// Unix seconds → (YYYYMMDD, ISO weekday 1 = Monday, minute of the day).
pub fn civil(unix_s: i64) -> (i64, i64, i64) {
    let days = unix_s.div_euclid(86_400);
    let secs = unix_s.rem_euclid(86_400);
    // 1970-01-01 was a Thursday (ISO 4).
    let weekday = (days + 3).rem_euclid(7) + 1;
    // Howard Hinnant's days_from_civil, inverted.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y * 10_000 + m * 100 + d, weekday, secs / 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn civil_dates() {
        assert_eq!(super::civil(0), (19_700_101, 4, 0));
        // 2026-11-21 22:30 UTC, a Saturday.
        assert_eq!(super::civil(1_795_300_200), (20_261_121, 6, 22 * 60 + 30));
        // 2024-02-29 (leap day), a Thursday.
        assert_eq!(super::civil(1_709_164_800), (20_240_229, 4, 0));
    }
}
