// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Authorization gate (mirrors src/core/authz.js).
// Role→capability map, command→capability, SoD warnings, authz gate.

use crate::actor::{get_authz, get_roles};
use crate::money::BukioError;
use rusqlite::Connection;

pub const ROLES: &[&str] = &["owner", "bookkeeper", "payments", "tax", "assets", "readonly"];

pub const ROLE_CAPABILITIES: &[(&str, &[&str])] = &[
    ("owner", &[]),  // null → everything (checked in canAct)
    ("bookkeeper", &["admin.chart", "entry.draft", "entry.post", "contacts.manage", "invoice.manage", "bank.import", "bank.match", "vat.book", "assets.manage", "recurring.manage", "fx.manage", "report.read"]),
    ("payments", &["bank.import", "bank.match", "payments.sepa", "report.read"]),
    ("tax", &["vat.book", "vat.file", "close.month", "close.year", "export.manage", "report.read"]),
    ("assets", &["assets.manage", "report.read"]),
    ("readonly", &["report.read"]),
];

const CLI_CAPABILITIES: &[(&str, &str)] = &[
    ("init", "admin.company"),
    ("company update", "admin.company"),
    ("company logo", "admin.company"),
    ("update", "admin.company"),
    ("account add", "admin.chart"),
    ("account import", "admin.chart"),
    ("account deactivate", "admin.chart"),
    ("account reactivate", "admin.chart"),
    ("actor roles grant", "admin.actor"),
    ("actor roles revoke", "admin.actor"),
    ("actor roles", "admin.actor"),
    ("actor can", "admin.actor"),
    ("actor who-can", "admin.actor"),
    ("actor authz", "admin.actor"),
    ("actor enforce", "admin.actor"),
    ("actor list", "admin.actor"),
    ("actor revoke", "admin.actor"),
    ("backup", "admin.backup"),
    ("restore", "admin.backup"),
    ("attach add", "admin.backup"),
    ("attach remove", "admin.backup"),
    ("entry post", "entry.post"),
    ("entry reverse", "entry.post"),
    ("item add", "entry.draft"),
    ("item update", "entry.draft"),
    ("contact add", "contacts.manage"),
    ("contact update", "contacts.manage"),
    ("invoice create", "invoice.manage"),
    ("invoice finalize", "invoice.manage"),
    ("invoice pdf", "invoice.manage"),
    ("invoice ubl", "invoice.manage"),
    ("invoice credit", "invoice.manage"),
    ("invoice email", "invoice.manage"),
    ("invoice reminders", "invoice.manage"),
    ("invoice pay", "invoice.manage"),
    ("invoice peppol-send", "invoice.manage"),
    ("bank add", "bank.import"),
    ("bank import", "bank.import"),
    ("bank ignore", "bank.import"),
    ("bank unignore", "bank.import"),
    ("bank match auto", "bank.match"),
    ("bank match suggest", "bank.match"),
    ("bank match link", "bank.match"),
    ("bank match post", "bank.match"),
    ("payments payables add", "payments.sepa"),
    ("payments payables pay", "payments.sepa"),
    ("payments mandate add", "payments.sepa"),
    ("payments mandate remove", "payments.sepa"),
    ("payments batch create", "payments.sepa"),
    ("payments batch export", "payments.sepa"),
    ("payments batch delete", "payments.sepa"),
    ("vat book", "vat.book"),
    ("vat readout", "vat.book"),
    ("icp readout", "vat.book"),
    ("compliance status", "vat.book"),
    ("compliance mark", "vat.book"),
    ("vat file", "vat.file"),
    ("vat settle", "vat.file"),
    ("assets scheme add", "assets.manage"),
    ("assets add", "assets.manage"),
    ("assets run", "assets.manage"),
    ("assets register", "assets.manage"),
    ("assets dispose", "assets.manage"),
    ("assets pause", "assets.manage"),
    ("assets resume", "assets.manage"),
    ("depreciation add", "assets.manage"),
    ("recurring add", "recurring.manage"),
    ("recurring pause", "recurring.manage"),
    ("recurring resume", "recurring.manage"),
    ("recurring run", "recurring.manage"),
    ("month-end", "close.month"),
    ("year-end status", "close.year"),
    ("year-end close", "close.year"),
    ("year-end report", "close.year"),
    ("export xaf", "export.manage"),
    ("fx set", "fx.manage"),
    ("fx fetch", "fx.manage"),
    ("cost-center add", "admin.chart"),
    ("cost-center deactivate", "admin.chart"),
    ("cost-center reactivate", "admin.chart"),
    ("cost-center list", "report.read"),
    ("cost-center show", "report.read"),
    ("report trial-balance", "report.read"),
    ("report balance-sheet", "report.read"),
    ("report pnl", "report.read"),
    ("report journal", "report.read"),
    ("report aging", "report.read"),
    ("report sales", "report.read"),
    ("report cost-center", "report.read"),
    ("audit", "report.read"),
    ("audit verify", "report.read"),
    ("company show", "report.read"),
    ("attach list", "report.read"),
    ("attach show", "report.read"),
    ("account list", "report.read"),
    ("account show", "report.read"),
    ("entry list", "report.read"),
    ("entry show", "report.read"),
    ("item list", "report.read"),
    ("item show", "report.read"),
    ("contact list", "report.read"),
    ("contact statement", "report.read"),
    ("invoice list", "report.read"),
    ("invoice show", "report.read"),
    ("bank list", "report.read"),
    ("bank transactions", "report.read"),
    ("payments payables list", "report.read"),
    ("payments mandate list", "report.read"),
    ("payments batch list", "report.read"),
    ("payments batch show", "report.read"),
    ("vat codes", "report.read"),
    ("assets scheme list", "report.read"),
    ("assets list", "report.read"),
    ("assets show", "report.read"),
    ("recurring preview", "report.read"),
    ("recurring list", "report.read"),
    ("recurring show", "report.read"),
    ("fx list", "report.read"),
    ("fx show", "report.read"),
];

const SOD_PAIRS: &[(&[&str], &str)] = &[
    (&["bookkeeper", "payments"], "bookkeeper + payments: the same actor books AND authorises money out"),
    (&["bookkeeper", "tax"], "bookkeeper + tax: the same actor books AND files tax"),
    (&["payments", "tax"], "payments + tax: the same actor authorises money out AND files tax"),
];

const SOD_CAPABILITY_PAIR: (&[&str], &str) = (&["entry.post", "payments.sepa"], "entry.post + payments.sepa: the same actor moves the ledger AND authorises money out (strongest pair)");

const EXEMPT_CMDS: &[&str] = &["actor keygen", "actor unlock", "actor lock", "mcp", "server start", "server token"];

/// Check if a capability is in a role's set.
fn has_capability(role: &str, capability: &str) -> bool {
    for &(r, caps) in ROLE_CAPABILITIES {
        if r == role { return caps.contains(&capability); }
    }
    false
}

/// SoD warnings for a set of roles.
pub fn sod_warnings(roles: &[String]) -> Vec<String> {
    if roles.iter().any(|r| r == "owner") { return vec![]; }
    let role_set: std::collections::HashSet<&str> = roles.iter().map(|r| r.as_str()).collect();
    let mut caps = std::collections::HashSet::new();
    for r in roles {
        if r == "owner" { continue; }
        for &(role, role_caps) in ROLE_CAPABILITIES {
            if role == r.as_str() { role_caps.iter().for_each(|c| { caps.insert(*c); }); }
        }
    }
    let mut warnings = Vec::new();
    for &(pair_roles, msg) in SOD_PAIRS {
        if pair_roles.iter().all(|r| role_set.contains(*r)) { warnings.push(msg.to_string()); }
    }
    if SOD_CAPABILITY_PAIR.0.iter().all(|c| caps.contains(*c)) { warnings.push(SOD_CAPABILITY_PAIR.1.to_string()); }
    warnings
}

/// Map a command path to its capability. `entry add` is resolved by opts.post.
pub fn capability_of(cmd: &str, post: bool) -> Option<&'static str> {
    if cmd.starts_with("mcp:") {
        // MCP tools not yet mapped — return None (fail closed)
        return None;
    }
    if cmd == "entry add" { return Some(if post { "entry.post" } else { "entry.draft" }); }
    for &(c, cap) in CLI_CAPABILITIES {
        if c == cmd { return Some(cap); }
    }
    None
}

/// May this actor perform this capability?
pub fn can_act(db: &Connection, actor: &str, capability: &str) -> bool {
    if capability.is_empty() { return false; }
    for role in get_roles(db, actor) {
        if role == "owner" { return true; }
        if has_capability(&role, capability) { return true; }
    }
    false
}

/// Is this command exempt from authz checking?
pub fn is_authz_exempt(cmd: &str, has_target: bool) -> bool {
    if EXEMPT_CMDS.contains(&cmd) { return true; }
    if cmd == "actor register" { return true; }
    if cmd == "actor verify" { return true; }
    if cmd == "actor roles" && !has_target { return true; }
    if cmd == "actor can" && !has_target { return true; }
    if cmd == "actor revoke" && !has_target { return true; }
    false
}

/// The authz gate. Called after signature verification, before any mutation.
pub fn check_authz(db: &Connection, actor: &str, cmd: &str, has_target: bool, post: bool) -> Result<(), BukioError> {
    let is_owner_kill = cmd == "actor revoke" && has_target;
    if !is_owner_kill && is_authz_exempt(cmd, has_target) { return Ok(()); }
    if !get_authz(db) && !is_owner_kill { return Ok(()); }
    if is_owner_kill {
        let roles = get_roles(db, actor);
        if !roles.iter().any(|r| r == "owner") {
            return Err(BukioError::new("AUTHZ_DENIED", format!("actor {actor} needs the owner role to revoke another actor's key — ask the owner")));
        }
        return Ok(());
    }
    let capability = capability_of(cmd, post);
    if capability.is_none() || !can_act(db, actor, capability.unwrap_or("")) {
        let roles = get_roles(db, actor);
        return Err(BukioError::new("AUTHZ_DENIED",
            format!("actor {actor} has no capability '{}' in this company (roles: {}) — ask the owner to grant it",
                capability.unwrap_or("?"),
                if roles.is_empty() { "no roles".to_string() } else { roles.join(", ") }
            )
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sod_warnings_basic() {
        let w = sod_warnings(&["bookkeeper".into(), "payments".into()]);
        assert_eq!(w.len(), 2); // bookkeeper+payments + capability pair
    }

    #[test]
    fn sod_warnings_owner_exempt() {
        let w = sod_warnings(&["owner".into()]);
        assert!(w.is_empty());
    }

    #[test]
    fn capability_of_basic() {
        assert_eq!(capability_of("entry add", false), Some("entry.draft"));
        assert_eq!(capability_of("entry add", true), Some("entry.post"));
        assert_eq!(capability_of("entry post", false), Some("entry.post"));
        assert_eq!(capability_of("report trial-balance", false), Some("report.read"));
        assert_eq!(capability_of("nonexistent", false), None);
    }

    #[test]
    fn is_authz_exempt_basic() {
        assert!(is_authz_exempt("actor keygen", false));
        assert!(is_authz_exempt("actor register", false));
        assert!(is_authz_exempt("actor revoke", false)); // self-revoke
        assert!(!is_authz_exempt("actor revoke", true)); // target = other actor
    }
}
