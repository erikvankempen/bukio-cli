//! Winst-en-verliesrekening (profit & loss) for a period, grouped by RGS
//! hoofdgroep. Port of the Node `src/report/pnl.js`.

use crate::core::chart::rgs_label;
use crate::error::Result;
use rusqlite::{params, Connection};
use serde::Serialize;

const PNL_GROUPS: [&str; 7] = ["WOMZ.80", "WOVB.82", "WKPR.70", "WPER.40", "WAFS.41", "WBED.42", "WFBE.84"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PnlAccount {
    pub code: String,
    pub name: String,
    #[serde(rename = "type")]
    pub account_type: String,
    pub amount_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PnlSection {
    pub rgs_code: Option<String>,
    pub label: String,
    pub accounts: Vec<PnlAccount>,
    pub total_cents: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Pnl {
    pub from: String,
    pub to: String,
    pub sections: Vec<PnlSection>,
    pub revenue_cents: i64,
    pub costs_cents: i64,
    pub result_cents: i64,
}

pub fn pnl(conn: &Connection, from: &str, to: &str) -> Result<Pnl> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.code, a.name, a.type, a.rgs_code,\n\
                COALESCE(SUM(p.amount_cents), 0) AS net_cents\n\
         FROM accounts a\n\
         LEFT JOIN (\n\
           SELECT p.account_id, p.amount_cents\n\
           FROM postings p\n\
           JOIN journal_entries e ON e.id = p.entry_id\n\
           WHERE e.state = 'posted' AND e.source != 'closing' AND e.date >= ?1 AND e.date <= ?2\n\
         ) p ON p.account_id = a.id\n\
         WHERE a.type IN ('income','expense')\n\
         GROUP BY a.id\n\
         ORDER BY a.code",
    )?;
    let rows: Vec<(String, String, String, Option<String>, i64)> = stmt
        .query_map(params![from, to], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut sections: Vec<PnlSection> = Vec::new();
    for code in PNL_GROUPS {
        let accounts: Vec<PnlAccount> = rows
            .iter()
            .filter(|(_, _, _t, rgs, _)| rgs.clone().unwrap_or_else(|| "overig".to_string()) == *code)
            .map(|(c, n, t, _, net)| PnlAccount {
                code: c.clone(),
                name: n.clone(),
                account_type: t.clone(),
                amount_cents: if t == "income" { -net } else { *net },
            })
            .filter(|a| a.amount_cents != 0)
            .collect();
        if !accounts.is_empty() {
            let total = accounts.iter().map(|a| a.amount_cents).sum();
            sections.push(PnlSection {
                rgs_code: Some(code.to_string()),
                label: rgs_label(Some(code)),
                accounts,
                total_cents: total,
            });
        }
    }
    let leftover: Vec<PnlAccount> = rows
        .iter()
        .filter(|(_, _, _, rgs, _)| !PNL_GROUPS.contains(&rgs.clone().unwrap_or_else(|| "overig".to_string()).as_str()))
        .map(|(c, n, t, _, net)| PnlAccount {
            code: c.clone(),
            name: n.clone(),
            account_type: t.clone(),
            amount_cents: if t == "income" { -net } else { *net },
        })
        .filter(|a| a.amount_cents != 0)
        .collect();
    if !leftover.is_empty() {
        let total = leftover.iter().map(|a| a.amount_cents).sum();
        sections.push(PnlSection {
            rgs_code: None,
            label: "Overig".to_string(),
            accounts: leftover,
            total_cents: total,
        });
    }

    let revenue: i64 = sections
        .iter()
        .filter(|s| s.rgs_code.as_deref() == Some("WOMZ.80") || s.rgs_code.as_deref() == Some("WOVB.82"))
        .map(|s| s.total_cents)
        .sum();
    let costs: i64 = sections
        .iter()
        .filter(|s| s.rgs_code.as_deref() != Some("WOMZ.80") && s.rgs_code.as_deref() != Some("WOVB.82"))
        .map(|s| s.total_cents)
        .sum();

    Ok(Pnl {
        from: from.to_string(),
        to: to.to_string(),
        sections,
        revenue_cents: revenue,
        costs_cents: costs,
        result_cents: revenue - costs,
    })
}
