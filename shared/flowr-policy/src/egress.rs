//! Where calls out of A3T may go (RT-1).
//!
//! Every outbound call a project's flow or agent makes — `call:http`, a remote
//! MCP server, an outgoing webhook — is checked here before it leaves:
//!
//! - **Never** to a private, loopback, link-local (cloud metadata lives at
//!   169.254.169.254), shared-address or multicast address, by IP or by a name
//!   that resolves to one (SSRF) — unless the org's egress file names that
//!   host or range under `private`.
//! - Only `http` and `https`.
//! - **Allowlist:** an org with an enforced egress file (`egress/<name>.yaml`,
//!   `kind: a3t/egress`, `hosts: [api.github.com, "*.slack.com"]`) reaches only
//!   those hosts; an agent whose own policy has `egress: [...]` reaches only the
//!   hosts both lists allow.
//!
//! The rules come from FlowR ([`set_rules`]); the platform's own flows
//! (`a3t`, `system`, …) are A3T's code calling its own services and are not
//! checked. A refusal is logged as `egress:deny` in the decision log.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, OnceLock};

/// What a project (and agent) may reach.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rules {
    /// Host patterns the org allows (`api.github.com`, `*.slack.com`); `None`:
    /// any public host.
    pub hosts: Option<Vec<String>>,
    /// The agent's own list, if it has one: it narrows `hosts`.
    pub agent_hosts: Option<Vec<String>>,
    /// Private hosts or ranges the org lets its flows reach (an MCP server on
    /// its own network): host names, IPs or CIDRs.
    pub private: Vec<String>,
}

#[async_trait::async_trait]
pub trait EgressRules: Send + Sync {
    async fn rules(&self, project: &str, agent: Option<&str>) -> Rules;
}

static RULES: OnceLock<Arc<dyn EgressRules>> = OnceLock::new();

pub fn set_rules(r: Arc<dyn EgressRules>) {
    let _ = RULES.set(r);
}

/// Namespaces whose flows are the platform's own: not checked.
pub const PLATFORM: &[&str] = &["a3t", "system", "system-gateway", "system-archive"];

/// May a call from `project` (by `agent`, if an agent made it) go to `url`?
/// The reason when it may not.
pub async fn check(url: &str, project: &str, agent: Option<&str>) -> Result<(), String> {
    if PLATFORM.contains(&project) {
        return Ok(());
    }
    let (scheme, host, port) = split_url(url).ok_or_else(|| format!("{url:?} is not a URL A3T calls out to"))?;
    if scheme != "http" && scheme != "https" {
        return Err(format!("{scheme}: calls out of A3T are http or https only"));
    }
    let rules = match RULES.get() {
        Some(r) => r.rules(project, agent).await,
        None => Rules::default(),
    };
    allowed_host(&rules, &host)?;
    // The address it goes to: by IP, or every address its name resolves to.
    let ips: Vec<IpAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![ip],
        Err(_) => match tokio::net::lookup_host((host.as_str(), port)).await {
            Ok(a) => a.map(|s| s.ip()).collect(),
            Err(e) => return Err(format!("{host} doesn't resolve ({e}), so it was not called")),
        },
    };
    let named_private = rules.private.iter().any(|p| matches_host(p, &host));
    for ip in ips {
        if internal(&ip) && !named_private && !rules.private.iter().any(|p| in_cidr(p, &ip)) {
            return Err(format!(
                "{host} is a private address ({ip}): flows and agents don't call private, loopback or cloud-metadata addresses unless the org's egress file names it under `private`"
            ));
        }
    }
    Ok(())
}

/// The host allowlists, without resolving anything.
pub fn allowed_host(rules: &Rules, host: &str) -> Result<(), String> {
    if ["metadata.google.internal", "metadata", "instance-data"].contains(&host.to_ascii_lowercase().as_str())
        && !rules.private.iter().any(|p| matches_host(p, host))
    {
        return Err(format!("{host} is a cloud metadata address: never called"));
    }
    if let Some(list) = &rules.hosts {
        if !list.iter().any(|p| matches_host(p, host)) {
            return Err(format!("{host} is not in the org's egress allowlist"));
        }
    }
    if let Some(list) = &rules.agent_hosts {
        if !list.iter().any(|p| matches_host(p, host)) {
            return Err(format!("{host} is not in this agent's egress list"));
        }
    }
    Ok(())
}

/// `api.github.com` matches itself; `*.slack.com` matches any subdomain (not
/// `slack.com` itself); `*` matches anything.
pub fn matches_host(pattern: &str, host: &str) -> bool {
    let (p, h) = (pattern.trim().to_ascii_lowercase(), host.trim().trim_end_matches('.').to_ascii_lowercase());
    if p == "*" || p == h {
        return true;
    }
    match p.strip_prefix("*.") {
        Some(suffix) => h.ends_with(&format!(".{suffix}")),
        None => false,
    }
}

/// `(scheme, host, port)` of a URL.
pub fn split_url(url: &str) -> Option<(String, String, u16)> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let authority = rest.split(['/', '?', '#']).next()?;
    let hostport = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    let (host, port) = if let Some(v6) = hostport.strip_prefix('[') {
        let (h, after) = v6.split_once(']')?;
        (h.to_string(), after.strip_prefix(':').and_then(|p| p.parse().ok()))
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => (h.to_string(), p.parse().ok()),
            _ => (hostport.to_string(), None),
        }
    };
    if host.is_empty() {
        return None;
    }
    let port = port.unwrap_or(if scheme == "https" { 443 } else { 80 });
    Some((scheme, host.to_ascii_lowercase(), port))
}

/// Private, loopback, link-local, shared, documentation, multicast or
/// unspecified: not on the public internet.
pub fn internal(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => internal_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return internal_v4(&v4);
            }
            let s = v6.segments();
            v6.is_loopback() || v6.is_unspecified() || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (s[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (s[0] == 0x64 && s[1] == 0xff9b) // NAT64 (may reach IPv4 internals)
                || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
        }
    }
}

fn internal_v4(ip: &Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast()
        || ip.is_documentation()
        || o[0] == 0
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // shared address space 100.64/10
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF protocol assignments
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // benchmarking 198.18/15
        || o[0] >= 240 // reserved
}

/// `10.0.0.0/8`-style range (or one IP) containing `ip`.
pub fn in_cidr(range: &str, ip: &IpAddr) -> bool {
    let (net, bits) = match range.split_once('/') {
        Some((n, b)) => (n, b.parse::<u32>().ok()),
        None => (range, None),
    };
    let Ok(net) = net.trim().parse::<IpAddr>() else { return false };
    match (net, ip) {
        (IpAddr::V4(n), IpAddr::V4(i)) => {
            let b = bits.unwrap_or(32).min(32);
            let mask = if b == 0 { 0 } else { u32::MAX << (32 - b) };
            (u32::from(n) & mask) == (u32::from(*i) & mask)
        }
        (IpAddr::V6(n), IpAddr::V6(i)) => {
            let b = bits.unwrap_or(128).min(128);
            let mask = if b == 0 { 0 } else { u128::MAX << (128 - b) };
            (u128::from(n) & mask) == (u128::from(*i) & mask)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_split() {
        assert_eq!(split_url("https://api.github.com/repos"), Some(("https".into(), "api.github.com".into(), 443)));
        assert_eq!(split_url("http://u:p@10.0.0.1:8080/x"), Some(("http".into(), "10.0.0.1".into(), 8080)));
        assert_eq!(split_url("http://[::1]:9/"), Some(("http".into(), "::1".into(), 9)));
        assert_eq!(split_url("nonsense"), None);
    }

    #[test]
    fn private_addresses_are_internal() {
        for ip in ["10.1.2.3", "172.16.0.1", "192.168.1.1", "127.0.0.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "::1", "fd00::1", "fe80::1", "::ffff:10.0.0.1"] {
            assert!(internal(&ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "140.82.112.3", "2606:4700::1111"] {
            assert!(!internal(&ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn hosts_and_ranges_match() {
        assert!(matches_host("*.slack.com", "hooks.slack.com"));
        assert!(!matches_host("*.slack.com", "slack.com"));
        assert!(!matches_host("*.slack.com", "evilslack.com"));
        assert!(matches_host("API.github.com", "api.github.com."));
        assert!(in_cidr("10.0.0.0/8", &"10.9.9.9".parse().unwrap()));
        assert!(!in_cidr("10.0.0.0/8", &"11.0.0.1".parse().unwrap()));
        assert!(in_cidr("fd00::/8", &"fd12::1".parse().unwrap()));
    }

    #[test]
    fn allowlists_narrow() {
        let r = Rules { hosts: Some(vec!["api.github.com".into(), "*.slack.com".into()]), agent_hosts: Some(vec!["api.github.com".into()]), private: vec![] };
        assert!(allowed_host(&r, "api.github.com").is_ok());
        assert!(allowed_host(&r, "hooks.slack.com").unwrap_err().contains("agent"));
        assert!(allowed_host(&r, "example.com").unwrap_err().contains("org"));
        assert!(allowed_host(&Rules::default(), "metadata.google.internal").is_err());
    }

    #[tokio::test]
    async fn private_ips_are_refused_and_the_platform_is_not_checked() {
        assert!(check("http://169.254.169.254/latest/meta-data", "acme-web", None).await.unwrap_err().contains("private"));
        assert!(check("http://127.0.0.1:3002/x", "acme-web", None).await.is_err());
        assert!(check("ftp://example.com/", "acme-web", None).await.is_err());
        assert!(check("http://127.0.0.1:3002/x", "a3t", None).await.is_ok());
    }
}
