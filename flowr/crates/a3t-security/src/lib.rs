//! `a3t-security` — A3T's security layer (formerly the AESOP crates and the
//! `flowr-aesop` glue), built into the server with `--features ee`.
//!
//! - [`authz`]: principals, actions, resources, decisions and the Cedar
//!   [`AuthzEngine`] (deny by default; every Deny is audited).
//! - [`hook`]: [`CedarHook`], the Cedar implementation of `flowr_policy::PolicyHook`
//!   (the default build ships `AllowAll`; `security.policy_hook = "cedar"` installs
//!   this one).
//! - [`policy_files`]: org, role and agent policy files compiled to Cedar.
//! - [`oidc`]: OIDC/JWT verification against an issuer's JWKS.
//! - [`oidc_admin`]: the identity provider's admin API (Keycloak): users, roles,
//!   password login.
//!
//! Policy never lives in application code: callers build a request, the engine
//! decides from the policy set.

pub mod authz;
pub mod hook;
pub mod oidc;
pub mod oidc_admin;
pub mod policy_files;

pub use authz::*;
pub use hook::*;
