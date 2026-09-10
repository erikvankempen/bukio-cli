// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Authorization gate (mirrors src/core/authz.js).
// Role→capability map, command→capability, SoD warnings, authz gate.

use crate::actor::{get_authz, get_roles};
use crate::money::BukioError;
use rusqlite::Connection;

pub const ROLES: &[&str] = &[
    "owner",
    "bookkeeper",
    "payments",
    "tax",
    "assets",
    "readonly",
];

pub const ROLE_CAPABILITIES: &[(&str, &[&str])] = &[
    ("owner", &[]), // null → everything (checked in canAct)
    (
        "bookkeeper",
        &[
            "admin.chart",
            "entry.draft",
            "entry.post",
            "contacts.manage",
            "invoice.manage",
            "bank.import",
            "bank.match",
            "vat.book",
            "assets.manage",
            "recurring.manage",
            "fx.manage",
            "report.read",
        ],
    ),
    (
        "payments",
        &["bank.import", "bank.match", "payments.sepa", "report.read"],
    ),
    (
        "tax",
        &[
            "vat.book",
            "vat.file",
            "close.month",
            "close.year",
            "export.manage",
            "report.read",
        ],
    ),
    ("assets", &["assets.manage", "report.read"]),
    ("readonly", &["report.read"]),
];

pub const CLI_CAPABILITIES: &[(&str, &str)] = &[
    ("init", "admin.company"),
    ("company update", "admin.company"),
    ("company logo", "admin.company"),
    ("update", "admin.company"),
    // imports + vat enable: admin.company (JS parity — these were unmapped, so
    // under `authz on` they were denied to EVERYONE including the owner)
    ("import opening-balances", "admin.company"),
    ("import journal", "admin.company"),
    ("import xaf", "admin.company"),
    ("import invoice", "admin.company"),
    ("import contacts", "admin.company"),
    ("vat enable", "admin.company"),
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
    ("financial-statements report", "close.year"),
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

/// MCP tool name -> capability (JS: MCP_CAPABILITIES). `entry_add` is dynamic
/// and handled in capability_of. Unmapped tools fail closed.
const MCP_CAPABILITIES: &[(&str, &str)] = &[
    ("company_info", "report.read"),
    ("trial_balance", "report.read"),
    ("balance_sheet", "report.read"),
    ("pnl", "report.read"),
    ("journal", "report.read"),
    ("accounts", "report.read"),
    ("audit", "report.read"),
    ("compliance", "vat.book"),
    ("invoices", "report.read"),
    ("entry_post", "entry.post"),
    ("entry_reverse", "entry.post"),
    ("vat_book", "vat.book"),
    ("vat_readout", "vat.book"),
    ("icp_readout", "vat.book"),
    ("invoice_create", "invoice.manage"),
    ("invoice_finalize", "invoice.manage"),
    ("invoice_credit", "invoice.manage"),
    ("invoice_pay", "invoice.manage"),
    ("invoice_email", "invoice.manage"),
    ("invoice_import", "admin.company"),
    ("item_add", "entry.draft"),
    ("item_update", "entry.draft"),
    ("item_list", "report.read"),
    ("attachment_add", "admin.backup"),
    ("attachment_remove", "admin.backup"),
    ("attachment_list", "report.read"),
    ("report_aging", "report.read"),
    ("report_sales", "report.read"),
    ("payments_mandate_add", "payments.sepa"),
    ("payments_mandate_list", "report.read"),
    ("payments_batch_create", "payments.sepa"),
    ("payments_batch_export", "payments.sepa"),
    ("recurring_run", "recurring.manage"),
    ("year_end_close", "close.year"),
    ("year_end_status", "close.year"),
    ("fx_set", "fx.manage"),
    ("contact_add", "contacts.manage"),
    ("assets_register", "assets.manage"),
    ("asset_add", "assets.manage"),
    ("assets_run", "assets.manage"),
    ("asset_dispose", "assets.manage"),
];

const SOD_PAIRS: &[(&[&str], &str)] = &[
    (
        &["bookkeeper", "payments"],
        "bookkeeper + payments: the same actor books AND authorises money out",
    ),
    (
        &["bookkeeper", "tax"],
        "bookkeeper + tax: the same actor books AND files tax",
    ),
    (
        &["payments", "tax"],
        "payments + tax: the same actor authorises money out AND files tax",
    ),
];

const SOD_CAPABILITY_PAIR: (&[&str], &str) = (&["entry.post", "payments.sepa"], "entry.post + payments.sepa: the same actor moves the ledger AND authorises money out (strongest pair)");

const EXEMPT_CMDS: &[&str] = &[
    "actor keygen",
    "actor unlock",
    "actor lock",
    "mcp",
    "server start",
    "server token",
];

/// Check if a capability is in a role's set.
fn has_capability(role: &str, capability: &str) -> bool {
    for &(r, caps) in ROLE_CAPABILITIES {
        if r == role {
            return caps.contains(&capability);
        }
    }
    false
}

/// SoD warnings for a set of roles.
pub fn sod_warnings(roles: &[String]) -> Vec<String> {
    if roles.iter().any(|r| r == "owner") {
        return vec![];
    }
    let role_set: std::collections::HashSet<&str> = roles.iter().map(|r| r.as_str()).collect();
    let mut caps = std::collections::HashSet::new();
    for r in roles {
        if r == "owner" {
            continue;
        }
        for &(role, role_caps) in ROLE_CAPABILITIES {
            if role == r.as_str() {
                role_caps.iter().for_each(|c| {
                    caps.insert(*c);
                });
            }
        }
    }
    let mut warnings = Vec::new();
    for &(pair_roles, msg) in SOD_PAIRS {
        if pair_roles.iter().all(|r| role_set.contains(*r)) {
            warnings.push(msg.to_string());
        }
    }
    if SOD_CAPABILITY_PAIR.0.iter().all(|c| caps.contains(*c)) {
        warnings.push(SOD_CAPABILITY_PAIR.1.to_string());
    }
    warnings
}

/// Map a command path to its capability. `entry add` is resolved by opts.post.
pub fn capability_of(cmd: &str, post: bool) -> Option<&'static str> {
    if let Some(tool) = cmd.strip_prefix("mcp:") {
        // entry_add is the one dynamic tool (post flips the capability)
        if tool == "entry_add" {
            return Some(if post { "entry.post" } else { "entry.draft" });
        }
        return MCP_CAPABILITIES
            .iter()
            .find(|(t, _)| *t == tool)
            .map(|(_, c)| *c);
    }
    if cmd == "entry add" {
        return Some(if post { "entry.post" } else { "entry.draft" });
    }
    // Prefix match: "actor roles grant bookkeeper" matches "actor roles grant"
    let mut best: Option<(&str, &str)> = None;
    for &(c, cap) in CLI_CAPABILITIES {
        if c == cmd {
            return Some(cap);
        }
        if cmd.starts_with(c) && cmd.as_bytes().get(c.len()) == Some(&b' ') {
            if best.map_or(true, |(bc, _)| c.len() > bc.len()) {
                best = Some((c, cap));
            }
        }
    }
    best.map(|(_, cap)| cap)
}

/// May this actor perform this capability?
pub fn can_act(db: &Connection, actor: &str, capability: &str) -> bool {
    if capability.is_empty() {
        return false;
    }
    for role in get_roles(db, actor) {
        if role == "owner" {
            return true;
        }
        if has_capability(&role, capability) {
            return true;
        }
    }
    false
}

/// Is this command exempt from authz checking?
pub fn is_authz_exempt(cmd: &str, has_target: bool) -> bool {
    if EXEMPT_CMDS.contains(&cmd) {
        return true;
    }
    if cmd == "actor register" {
        return true;
    }
    if cmd == "actor verify" {
        return true;
    }
    if cmd == "actor roles" && !has_target {
        return true;
    }
    if cmd == "actor can" && !has_target {
        return true;
    }
    if cmd == "actor revoke" && !has_target {
        return true;
    }
    false
}

/// The authz gate. Called after signature verification, before any mutation.
pub fn check_authz(
    db: &Connection,
    actor: &str,
    cmd: &str,
    has_target: bool,
    post: bool,
) -> Result<(), BukioError> {
    // The exemption list and the owner-kill rule match the command PATH
    // (JS: commandPathOf) — `cmd` carries positional args, so
    // 'actor can entry add' is still 'actor can' and
    // 'actor revoke agent:x' is still 'actor revoke'.
    let path: String = cmd
        .split_whitespace()
        .take(2)
        .collect::<Vec<&str>>()
        .join(" ");
    let path: &str = if path.is_empty() { cmd } else { &path };
    let is_owner_kill = path == "actor revoke" && has_target;
    if !is_owner_kill && is_authz_exempt(path, has_target) {
        return Ok(());
    }
    if !get_authz(db) && !is_owner_kill {
        return Ok(());
    }
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

    // ── ported from test/authz.test.js ──

    /// Every real CLI command path (the JS suite carried the same list).
    const CLI_PATHS: &[&str] = &[
        "init",
        "account add",
        "account list",
        "account show",
        "account deactivate",
        "account reactivate",
        "account import",
        "actor keygen",
        "actor register",
        "actor list",
        "actor revoke",
        "actor enforce",
        "actor unlock",
        "actor lock",
        "actor verify",
        "assets scheme add",
        "assets scheme list",
        "assets add",
        "assets list",
        "assets show",
        "assets run",
        "assets register",
        "assets dispose",
        "assets pause",
        "assets resume",
        "attach add",
        "attach list",
        "attach show",
        "attach remove",
        "audit",
        "audit verify",
        "backup",
        "restore",
        "bank add",
        "bank list",
        "bank import",
        "bank transactions",
        "bank match auto",
        "bank match suggest",
        "bank match link",
        "bank match post",
        "bank ignore",
        "bank unignore",
        "company show",
        "company update",
        "company logo",
        "compliance status",
        "compliance mark",
        "contact add",
        "contact update",
        "contact list",
        "contact statement",
        "entry add",
        "entry post",
        "entry reverse",
        "entry list",
        "entry show",
        "export xaf",
        "fx fetch",
        "fx set",
        "fx show",
        "fx list",
        "icp readout",
        "import opening-balances",
        "import journal",
        "import contacts",
        "import xaf",
        "import invoice",
        "invoice create",
        "invoice finalize",
        "invoice list",
        "invoice show",
        "invoice pdf",
        "invoice ubl",
        "invoice credit",
        "invoice peppol-send",
        "invoice pay",
        "invoice email",
        "invoice reminders",
        "item add",
        "item list",
        "item show",
        "item update",
        "month-end",
        "payments payables add",
        "payments payables list",
        "payments payables pay",
        "payments mandate add",
        "payments mandate list",
        "payments mandate remove",
        "payments batch create",
        "payments batch list",
        "payments batch show",
        "payments batch delete",
        "payments batch export",
        "recurring add",
        "recurring list",
        "recurring show",
        "recurring pause",
        "recurring resume",
        "recurring preview",
        "recurring run",
        "report trial-balance",
        "report balance-sheet",
        "report pnl",
        "report journal",
        "report aging",
        "report sales",
        "update",
        "vat enable",
        "vat codes",
        "vat book",
        "vat readout",
        "vat file",
        "vat settle",
        "year-end close",
        "year-end status",
        "year-end report",
        "financial-statements report",
        "mcp",
        "server start",
        "server token",
    ];

    const MCP_MUTATING_TOOLS: &[&str] = &[
        "entry_add",
        "entry_post",
        "entry_reverse",
        "vat_book",
        "invoice_create",
        "invoice_finalize",
        "invoice_credit",
        "invoice_pay",
        "invoice_email",
        "invoice_import",
        "item_add",
        "item_update",
        "attachment_add",
        "attachment_remove",
        "payments_mandate_add",
        "payments_batch_create",
        "payments_batch_export",
        "recurring_run",
        "year_end_close",
        "fx_set",
        "contact_add",
        "asset_add",
        "assets_run",
        "asset_dispose",
    ];

    fn known_capabilities() -> std::collections::HashSet<&'static str> {
        let mut set = std::collections::HashSet::new();
        for (_, caps) in ROLE_CAPABILITIES {
            for c in *caps {
                set.insert(*c);
            }
        }
        for (_, c) in CLI_CAPABILITIES {
            set.insert(*c);
        }
        for (_, c) in MCP_CAPABILITIES {
            set.insert(*c);
        }
        set
    }

    fn fresh_db() -> Connection {
        let db = crate::db::open_db(":memory:").unwrap();
        db.execute("INSERT INTO company (id, name) VALUES (1, 'X')", [])
            .unwrap();
        db
    }

    fn turn_authz_on(db: &Connection) {
        db.execute(
            "UPDATE settings SET value = 'on' WHERE key = 'authz_mode'",
            [],
        )
        .unwrap();
    }

    #[test]
    fn every_cli_path_maps_or_is_exempt() {
        let missing: Vec<&str> = CLI_PATHS
            .iter()
            .copied()
            .filter(|c| !is_authz_exempt(c, false) && capability_of(c, false).is_none())
            .collect();
        assert!(
            missing.is_empty(),
            "real CLI commands without a capability mapping: {missing:?}"
        );
    }

    #[test]
    fn every_mcp_mutating_tool_maps() {
        let missing: Vec<&str> = MCP_MUTATING_TOOLS
            .iter()
            .copied()
            .filter(|t| capability_of(&format!("mcp:{t}"), false).is_none())
            .collect();
        assert!(
            missing.is_empty(),
            "MCP mutating tools without a capability mapping: {missing:?}"
        );
    }

    #[test]
    fn unmapped_commands_fail_closed() {
        assert_eq!(capability_of("totally made-up", false), None);
        assert_eq!(capability_of("mcp:made_up_tool", false), None);
        assert_eq!(capability_of("", false), None);
    }

    #[test]
    fn role_capabilities_reference_real_capabilities() {
        let known = known_capabilities();
        for (role, caps) in ROLE_CAPABILITIES {
            assert!(ROLES.contains(role), "unknown role {role}");
            for c in *caps {
                assert!(
                    known.contains(*c),
                    "role {role} references unknown capability {c}"
                );
            }
        }
    }

    #[test]
    fn sod_map_stays_in_sync() {
        let known = known_capabilities();
        for (roles, msg) in SOD_PAIRS {
            for r in *roles {
                assert!(ROLES.contains(r), "unknown role {r} in an SoD pair");
            }
            assert!(msg.len() > 10);
        }
        for c in SOD_CAPABILITY_PAIR.0 {
            assert!(
                known.contains(*c),
                "unknown capability {c} in the SoD capability pair"
            );
        }
    }

    #[test]
    fn sod_warnings_pairs_and_clean_sets() {
        assert!(sod_warnings(&["bookkeeper".into()]).is_empty());
        assert!(sod_warnings(&["readonly".into()]).is_empty());
        assert_eq!(
            sod_warnings(&["bookkeeper".into(), "payments".into()]),
            vec![
                "bookkeeper + payments: the same actor books AND authorises money out".to_string(),
                "entry.post + payments.sepa: the same actor moves the ledger AND authorises money out (strongest pair)".to_string(),
            ]
        );
        assert_eq!(
            sod_warnings(&["bookkeeper".into(), "tax".into()]),
            vec!["bookkeeper + tax: the same actor books AND files tax".to_string()]
        );
        assert_eq!(
            sod_warnings(&["payments".into(), "tax".into()]),
            vec!["payments + tax: the same actor authorises money out AND files tax".to_string()]
        );
        // the owner may do everything — no warning
        assert!(sod_warnings(&[
            "owner".into(),
            "bookkeeper".into(),
            "payments".into(),
            "tax".into()
        ])
        .is_empty());
    }

    #[test]
    fn is_authz_exempt_self_service_versus_owner_actions() {
        assert!(is_authz_exempt("actor keygen", false));
        assert!(is_authz_exempt("actor unlock", false));
        assert!(is_authz_exempt("actor lock", false));
        assert!(is_authz_exempt("mcp", false));
        assert!(is_authz_exempt("actor register", false));
        assert!(is_authz_exempt("actor verify", false));
        assert!(is_authz_exempt("actor roles", false)); // self
        assert!(is_authz_exempt("actor can", false)); // self
        assert!(is_authz_exempt("actor revoke", false)); // self-revoke
                                                         // aimed at OTHERS these are owner territory (has_target)
        assert!(!is_authz_exempt("actor roles", true));
        assert!(!is_authz_exempt("actor can", true));
        assert!(!is_authz_exempt("actor revoke", true));
        // everything else is checked
        assert!(!is_authz_exempt("entry add", false));
        assert!(!is_authz_exempt("report trial-balance", false));
        assert!(!is_authz_exempt("actor who-can", false));
        assert!(!is_authz_exempt("actor list", false));
        assert!(!is_authz_exempt("mcp:entry_add", false));
    }

    #[test]
    fn can_act_deny_by_default_and_role_scoping() {
        let db = fresh_db();
        assert!(!can_act(&db, "agent:nobody", "report.read"));
        assert!(!can_act(&db, "agent:nobody", "entry.draft"));

        crate::actor::grant_role(&db, "human:erik", "owner", "human:erik").unwrap();
        crate::actor::grant_role(&db, "agent:invoicing", "bookkeeper", "human:erik").unwrap();
        crate::actor::grant_role(&db, "agent:pay", "payments", "human:erik").unwrap();
        crate::actor::grant_role(&db, "agent:auditor", "readonly", "human:erik").unwrap();

        // owner passes everything
        assert!(can_act(&db, "human:erik", "admin.actor"));
        assert!(can_act(&db, "human:erik", "payments.sepa"));
        assert!(can_act(&db, "human:erik", "report.read"));

        assert!(can_act(&db, "agent:invoicing", "entry.post"));
        assert!(can_act(&db, "agent:invoicing", "invoice.manage"));
        assert!(!can_act(&db, "agent:invoicing", "payments.sepa"));
        assert!(!can_act(&db, "agent:invoicing", "vat.file"));

        assert!(can_act(&db, "agent:pay", "payments.sepa"));
        assert!(can_act(&db, "agent:pay", "bank.import"));
        assert!(!can_act(&db, "agent:pay", "entry.post"));

        assert!(can_act(&db, "agent:auditor", "report.read"));
        assert!(!can_act(&db, "agent:auditor", "entry.draft"));

        // fail closed — an empty capability is never granted
        assert!(!can_act(&db, "human:erik", ""));
    }

    #[test]
    fn check_authz_off_by_default_never_refuses() {
        let db = fresh_db();
        assert!(check_authz(&db, "agent:nobody", "entry add", false, false).is_ok());
        assert!(check_authz(&db, "agent:nobody", "actor roles grant", false, false).is_ok());
    }

    #[test]
    fn check_authz_unmapped_denies_when_on() {
        let db = fresh_db();
        turn_authz_on(&db);
        crate::actor::grant_role(&db, "human:erik", "owner", "human:erik").unwrap();
        let err = check_authz(&db, "human:erik", "made-up cmd", false, false).unwrap_err();
        assert_eq!(err.code, "AUTHZ_DENIED");
    }

    #[test]
    fn check_authz_denial_names_actor_capability_and_roles() {
        let db = fresh_db();
        turn_authz_on(&db);
        crate::actor::grant_role(&db, "agent:invoicing", "bookkeeper", "human:erik").unwrap();
        let err = check_authz(&db, "agent:invoicing", "vat file", false, false).unwrap_err();
        assert_eq!(err.code, "AUTHZ_DENIED");
        assert!(err.message.contains("agent:invoicing"));
        assert!(err.message.contains("'vat.file'"));
        assert!(err.message.contains("bookkeeper"));
    }

    #[test]
    fn check_authz_resolves_by_actual_mutation() {
        let db = fresh_db();
        turn_authz_on(&db);
        crate::actor::grant_role(&db, "agent:invoicing", "bookkeeper", "human:erik").unwrap();
        // entry add --post needs entry.post, which bookkeeper holds
        assert!(check_authz(&db, "agent:invoicing", "entry add", false, true).is_ok());
        // vat file needs vat.file, which bookkeeper does not
        let err = check_authz(&db, "agent:invoicing", "vat file", false, false).unwrap_err();
        assert_eq!(err.code, "AUTHZ_DENIED");
    }

    #[test]
    fn check_authz_owner_kill_needs_owner_regardless_of_mode() {
        let db = fresh_db();
        crate::actor::grant_role(&db, "agent:invoicing", "bookkeeper", "human:erik").unwrap();
        // authz is OFF, but the owner-kill is still owner-only (D8)
        let err = check_authz(&db, "agent:invoicing", "actor revoke", true, false).unwrap_err();
        assert_eq!(err.code, "AUTHZ_DENIED");
        crate::actor::grant_role(&db, "human:erik", "owner", "human:erik").unwrap();
        assert!(check_authz(&db, "human:erik", "actor revoke", true, false).is_ok());
    }

    #[test]
    fn capability_of_resolves_entry_add_by_the_actual_mutation() {
        assert_eq!(capability_of("entry add", false), Some("entry.draft"));
        assert_eq!(capability_of("entry add", true), Some("entry.post"));
    }

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
        assert_eq!(
            capability_of("report trial-balance", false),
            Some("report.read")
        );
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
