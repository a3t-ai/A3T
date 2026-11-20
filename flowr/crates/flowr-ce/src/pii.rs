//! Personal data and secrets in text, found and redacted (RT-10).
//!
//! Found: email addresses, phone numbers, payment card numbers (Luhn-checked),
//! IBANs (mod-97-checked), US social security numbers, and common API key
//! shapes. Each is replaced by `[redacted:<kind>]`.
//!
//! Where it applies: the decision log's free text (reasons and notes) and
//! FlowR's own log lines that quote tool output.

use std::sync::OnceLock;

use regex::Regex;

struct Patterns {
    email: Regex,
    card: Regex,
    iban: Regex,
    ssn: Regex,
    phone: Regex,
    key: Regex,
}

fn p() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| Patterns {
        email: Regex::new(r"(?i)\b[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}\b").unwrap(),
        card: Regex::new(r"\b(?:\d[ -]?){12,18}\d\b").unwrap(),
        iban: Regex::new(r"\b[A-Z]{2}\d{2}(?: ?[A-Z0-9]){11,30}\b").unwrap(),
        ssn: Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap(),
        phone: Regex::new(r"(?:\+\d{1,3}[ .-]?)?(?:\(\d{2,4}\)[ .-]?)?\b\d{2,4}[ .-]\d{3,4}[ .-]\d{3,4}\b").unwrap(),
        key: Regex::new(r"\b(?:sk-[A-Za-z0-9_-]{16,}|ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|AKIA[0-9A-Z]{16}|xox[baprs]-[A-Za-z0-9-]{10,}|a3tk_[0-9a-f]{20,})\b").unwrap(),
    })
}

fn luhn(digits: &str) -> bool {
    let d: Vec<u32> = digits.chars().filter_map(|c| c.to_digit(10)).collect();
    if d.len() < 13 || d.len() > 19 {
        return false;
    }
    let sum: u32 = d.iter().rev().enumerate().map(|(i, &x)| if i % 2 == 1 { let y = x * 2; if y > 9 { y - 9 } else { y } } else { x }).sum();
    sum % 10 == 0
}

fn iban_ok(s: &str) -> bool {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if s.len() < 15 || s.len() > 34 {
        return false;
    }
    let moved = format!("{}{}", &s[4..], &s[..4]);
    let mut rem: u64 = 0;
    for c in moved.chars() {
        let v = match c { '0'..='9' => c as u64 - '0' as u64, 'A'..='Z' => c as u64 - 'A' as u64 + 10, _ => return false };
        rem = if v >= 10 { (rem * 100 + v) % 97 } else { (rem * 10 + v) % 97 };
    }
    rem == 1
}

/// `text` with personal data and secrets replaced, and the kinds found.
pub fn redact(text: &str) -> (String, Vec<&'static str>) {
    let mut found = Vec::new();
    let mut out = text.to_string();
    let pat = p();
    let mut sub = |re: &Regex, kind: &'static str, check: &dyn Fn(&str) -> bool, out: &mut String| {
        let next = re.replace_all(out, |c: &regex::Captures| {
            let m = &c[0];
            if check(m) { found.push(kind); format!("[redacted:{kind}]") } else { m.to_string() }
        }).into_owned();
        *out = next;
    };
    sub(&pat.key, "secret", &|_| true, &mut out);
    sub(&pat.email, "email", &|_| true, &mut out);
    sub(&pat.iban, "iban", &iban_ok, &mut out);
    sub(&pat.card, "card", &luhn, &mut out);
    sub(&pat.ssn, "ssn", &|_| true, &mut out);
    sub(&pat.phone, "phone", &|m| m.chars().filter(|c| c.is_ascii_digit()).count() >= 9, &mut out);
    found.sort();
    found.dedup();
    (out, found)
}

/// Just the redacted text.
pub fn clean(text: &str) -> String {
    redact(text).0
}

/// Every string in a value, redacted.
pub fn clean_value(v: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::String(t) => Value::String(clean(t)),
        Value::Array(a) => Value::Array(a.iter().map(clean_value).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), clean_value(x))).collect()),
        other => other.clone(),
    }
}

/// Whether a tool writes to memory (remember, store, learn… a fact or note).
pub fn memory_write(tool: &str) -> bool {
    let t = tool.to_ascii_lowercase();
    ["remember", "memorize", "memory_store", "memory_write", "memory_add", "store_fact", "store_memory", "learn", "save_note", "add_note"]
        .iter().any(|v| t.contains(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pii_never_lands_unredacted() {
        let t = "Mail jane.doe@acme.com or call +44 20 7946 0958; card 4111 1111 1111 1111, IBAN GB82 WEST 1234 5698 7654 32, SSN 123-45-6789, key sk-abcdefghijklmnop1234 and ghp_abcdefghijklmnopqrstuvwxyz12";
        let (out, kinds) = redact(t);
        for leak in ["jane.doe@acme.com", "4111 1111 1111 1111", "GB82 WEST", "123-45-6789", "sk-abcdefghijklmnop1234", "ghp_abc", "7946 0958"] {
            assert!(!out.contains(leak), "{leak} leaked: {out}");
        }
        assert_eq!(kinds, vec!["card", "email", "iban", "phone", "secret", "ssn"]);
    }

    #[test]
    fn memory_tool_inputs_are_redacted_whole() {
        assert!(memory_write("memory_remember") && memory_write("tool_crm_store_fact") && !memory_write("list_flows"));
        let v = clean_value(&serde_json::json!({ "fact": "Jane is jane.doe@acme.com", "tags": ["call +44 20 7946 0958"], "n": 3 }));
        assert!(!v.to_string().contains("jane.doe@acme.com") && !v.to_string().contains("7946 0958"));
        assert_eq!(v["n"], 3);
    }

    #[test]
    fn ordinary_text_and_ids_stay() {
        let t = "Run 01a1fa9f-cfee-7bb3 finished at 2026-11-20 in 1234 ms, order 4111111111111112 (not a card)";
        assert_eq!(clean(t), t);
    }
}
