//! The actions FlowR checks are exactly the actions the Cedar schema declares.
//! An action missing from the schema makes every check of it a policy-engine
//! fault (a deny with no reason); one missing from `actions::ALL` is invisible to
//! the console's policy tools.

use std::collections::BTreeSet;

#[test]
fn every_checked_action_is_declared_and_listed() {
    let declared: BTreeSet<String> = flowr_aesop::A3T_SCHEMA
        .lines()
        .filter_map(|l| l.trim().strip_prefix("action \""))
        .filter_map(|rest| rest.split('"').next())
        .map(str::to_string)
        .collect();
    let listed: BTreeSet<String> = flowr_policy::actions::ALL.iter().map(|a| a.to_string()).collect();
    assert_eq!(declared, listed, "schema actions (left) vs actions::ALL (right)");
}
