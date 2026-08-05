//! Balans (balance sheet) as of a date, grouped by RGS hoofdgroep.
//! Integrity invariant: total assets == total liabilities + equity + result.
//! Port of the Node `src/report/balans.js`.

use crate::core::chart::rgs_label;
use crate::error::Result;
use rusqlite::Connection;
use serde::Serialize;

const BALANCE_TYPES: [&str; 3] = ["asset", "liability", "equity"];
const ASSET_GROUPS: [&str; 5] = ["BMVA.02", "BFVA.03", "BVRD.30", "BVOR.11", "BLIM.10"];
const PASSIVA_GROUPS: [&str; 4] = ["BEIV.05", "BVRZ.07", "BLAS.08", "BSCH.12"];

fn net_per_account(conn: &Connection, as_of: &str, types: &[&str]) -> Result<Vec<(String, String, String, Option<String>, i64)>> {
    let placeholders = types.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT a.id, a.code, a.name, a.type, a.rgs_code,\n\
                COALESCE(SUM(p.amount_cents), 0) AS net_cents\n\
         FROM accounts a\n\
         LEFT JOIN (\n\
           SELECT p.account_id, p.amount_cents\n\
           FROM postings p\n\
           JOIN journal_entries e ON e.id = p.entry_id\n\
           WHERE e.state = 'posted' AND e.date <= ?\n\
         ) p ON p.account_id = a.id\n\
         WHERE a.type IN ({placeholders})\n\
         GROUP BY a.id\n\
         ORDER BY a.code"
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(as_of.to_string())];
    for t in types {
        args.push(Box::new(t.to_string()));
    }
    let param_refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(param_refs), |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct BalansAccount {
    pub code: String,
    pub name: String,
    #[serde(rename = "type")]
    pub account_type: String,
    pub balance_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct BalansSection {
    pub rgs_code: Option<String>,
    pub label: String,
    pub accounts: Vec<BalansAccount>,
    pub total_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct BalansSide {
    pub total_cents: i64,
    pub sections: Vec<BalansSection>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct BalansPassiva {
    pub total_cents: i64,
    pub sections: Vec<BalansSection>,
    pub result_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Balans {
    pub as_of: String,
    pub assets: BalansSide,
    pub liabilities_and_equity: BalansPassiva,
    pub balanced: bool,
}

fn sectionize(rows: &[(String, String, String, Option<String>, i64)], groups: &[&str], types: &[&str]) -> Vec<BalansSection> {
    let mut sections: Vec<BalansSection> = Vec::new();
    for code in groups {
        let accounts: Vec<BalansAccount> = rows
            .iter()
            .filter(|(_, _, t, rgs, _)| types.contains(&t.as_str()) && rgs.clone().unwrap_or_else(|| "overig".to_string()) == *code)
            .map(|(c, n, t, _, net)| BalansAccount {
                code: c.clone(),
                name: n.clone(),
                account_type: t.clone(),
                balance_cents: if t == "asset" { *net } else { -net },
            })
            .filter(|a| a.balance_cents != 0)
            .collect();
        if !accounts.is_empty() {
            let total = accounts.iter().map(|a| a.balance_cents).sum();
            sections.push(BalansSection {
                rgs_code: Some(code.to_string()),
                label: rgs_label(Some(code)),
                accounts,
                total_cents: total,
            });
        }
    }
    let known: Vec<&str> = groups.to_vec();
    let leftover: Vec<BalansAccount> = rows
        .iter()
        .filter(|(_, _, t, rgs, _)| {
            types.contains(&t.as_str()) && !known.contains(&rgs.clone().unwrap_or_else(|| "overig".to_string()).as_str())
        })
        .map(|(c, n, t, _, net)| BalansAccount {
            code: c.clone(),
            name: n.clone(),
            account_type: t.clone(),
            balance_cents: if t == "asset" { *net } else { -net },
        })
        .filter(|a| a.balance_cents != 0)
        .collect();
    if !leftover.is_empty() {
        let total = leftover.iter().map(|a| a.balance_cents).sum();
        sections.push(BalansSection {
            rgs_code: None,
            label: "Overig".to_string(),
            accounts: leftover,
            total_cents: total,
        });
    }
    sections
}

pub fn balans(conn: &Connection, as_of: &str) -> Result<Balans> {
    let rows = net_per_account(conn, as_of, &BALANCE_TYPES)?;

    let result_cents: i64 = conn.query_row(
        "SELECT COALESCE(SUM(p.amount_cents), 0)\n\
         FROM postings p\n\
         JOIN journal_entries e ON e.id = p.entry_id\n\
         JOIN accounts a ON a.id = p.account_id\n\
         WHERE e.state = 'posted' AND e.date <= ?1 AND a.type IN ('income','expense')",
        [as_of],
        |r| r.get(0),
    )?;
    let result_cents = -result_cents;

    let asset_sections = sectionize(&rows, &ASSET_GROUPS, &["asset"]);
    let passiva_sections = sectionize(&rows, &PASSIVA_GROUPS, &["liability", "equity"]);
    let total_assets: i64 = asset_sections.iter().map(|s| s.total_cents).sum();
    let total_passiva: i64 = passiva_sections.iter().map(|s| s.total_cents).sum::<i64>() + result_cents;

    Ok(Balans {
        as_of: as_of.to_string(),
        assets: BalansSide { total_cents: total_assets, sections: asset_sections },
        liabilities_and_equity: BalansPassiva { total_cents: total_passiva, sections: passiva_sections, result_cents },
        balanced: total_assets == total_passiva,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::accounts::seed_default_chart;
    use crate::core::db::open_in_memory;
    use crate::core::entries::{create_entry, parse_posting_specs, post_entry};
    use crate::report::journal;
    use crate::report::pnl;
    use crate::report::trial_balance::trial_balance;

    fn setup() -> Connection {
        let conn = open_in_memory().unwrap();
        seed_default_chart(&conn).unwrap();
        let specs = |s: &str| parse_posting_specs(&[s.to_string()]).unwrap();
        // startkapitaal
        let e1 = create_entry(&conn, "2026-01-01", "Kapitaal", &specs("1100:100000.00,3000:-100000.00"), "manual", None, "human").unwrap();
        post_entry(&conn, e1.meta.id, "human").unwrap();
        // omzet + kosten
        let e2 = create_entry(&conn, "2026-06-01", "Omzet", &specs("1100:1210.00,8000:-1000.00,2100:-210.00"), "manual", None, "human").unwrap();
        post_entry(&conn, e2.meta.id, "human").unwrap();
        conn
    }

    #[test]
    fn trial_balance_sums() {
        let conn = setup();
        let tb = trial_balance(&conn, Some("2026")).unwrap();
        assert!(tb.balanced);
        assert_eq!(tb.total_debit_cents, tb.total_credit_cents);
        let bank = tb.accounts.iter().find(|a| a.code == "1100").unwrap();
        assert_eq!(bank.net_cents, 10121000);
        let omzet = tb.accounts.iter().find(|a| a.code == "8000").unwrap();
        assert_eq!(omzet.net_cents, -100000);
        assert_eq!(omzet.credit_cents, 100000);
        // year filter excludes nothing here; a 2025 entry would be excluded
        let tb25 = trial_balance(&conn, Some("2025")).unwrap();
        assert!(tb25.accounts.is_empty());
    }

    #[test]
    fn balans_balances() {
        let conn = setup();
        let b = balans(&conn, "2026-12-31").unwrap();
        assert!(b.balanced);
        assert_eq!(b.assets.total_cents, 10121000);
        assert_eq!(b.liabilities_and_equity.total_cents, 10121000);
        // result = omzet 1000 - kosten 0 = 1000 (100000 cents)
        assert_eq!(b.liabilities_and_equity.result_cents, 100000);
    }

    #[test]
    fn pnl_result() {
        let conn = setup();
        let p = pnl::pnl(&conn, "2026-01-01", "2026-12-31").unwrap();
        assert_eq!(p.revenue_cents, 100000);
        assert_eq!(p.result_cents, 100000);
        let omzet_section = p.sections.iter().find(|s| s.rgs_code.as_deref() == Some("WOMZ.80")).unwrap();
        assert_eq!(omzet_section.total_cents, 100000);
    }

    #[test]
    fn journal_rows() {
        let conn = setup();
        let rows = journal::journal(&conn, "2026-01-01", "2026-12-31").unwrap();
        // 2 entries; entry 1 has 2 postings, entry 2 has 3
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].entry_id, 1);
        assert_eq!(rows[4].entry_id, 2);
    }
}
