//! VAT module (Phase 2) — enable + code list. The full booking/readout
//! semantics live in the vat milestone; here we provide what `init --vat on`
//! and the core engine need: the flag, the 1500/2500 accounts and the codes.

use crate::audit;
use crate::core::accounts::{create_account_from_seed, get_account_by_code};
use crate::core::chart::AccountSeed;
use crate::error::{AppError, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

pub const VAT_ACCOUNTS: &[AccountSeed] = &[
    AccountSeed { code: "1500", name: "Te vorderen omzetbelasting", account_type: "asset", normal_balance: "debit", rgs_code: "BVOR.11" },
    AccountSeed { code: "2500", name: "Te betalen omzetbelasting", account_type: "liability", normal_balance: "credit", rgs_code: "BSCH.12" },
];

#[derive(Debug, Clone, Copy)]
pub struct VatCodeSeed {
    pub code: &'static str,
    pub rate_bp: i64,
    pub vat_type: &'static str,
    pub eu_reverse: u8,
    pub description: &'static str,
}

pub const VAT_CODES: &[VatCodeSeed] = &[
    VatCodeSeed { code: "21", rate_bp: 2100, vat_type: "standard", eu_reverse: 0, description: "21% hoog tarief" },
    VatCodeSeed { code: "9", rate_bp: 900, vat_type: "standard", eu_reverse: 0, description: "9% laag tarief" },
    VatCodeSeed { code: "0", rate_bp: 0, vat_type: "standard", eu_reverse: 0, description: "0% nultarief" },
    VatCodeSeed { code: "V", rate_bp: 0, vat_type: "exempt", eu_reverse: 0, description: "Vrijgesteld" },
    VatCodeSeed { code: "R", rate_bp: 0, vat_type: "reverse", eu_reverse: 0, description: "Verlegd (binnenland)" },
    VatCodeSeed { code: "RE", rate_bp: 0, vat_type: "reverse", eu_reverse: 1, description: "Verlegd (EU)" },
    VatCodeSeed { code: "M", rate_bp: 0, vat_type: "margin", eu_reverse: 0, description: "Marge" },
    VatCodeSeed { code: "P", rate_bp: 0, vat_type: "private", eu_reverse: 0, description: "Privégebruik" },
];

pub fn is_vat_enabled(conn: &Connection) -> Result<bool> {
    let row: Option<(i64, i64)> = conn
        .query_row("SELECT vat_module, kor_flag FROM company", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    Ok(match row {
        Some((vat_module, kor_flag)) => vat_module == 1 && kor_flag == 0,
        None => false,
    })
}

fn require_vat(conn: &Connection) -> Result<()> {
    if !is_vat_enabled(conn)? {
        return Err(AppError::new(
            "VAT_MODULE_OFF",
            "the VAT module is not enabled for this company (enable with `bukio vat enable`)",
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct VatEnableResult {
    pub vat_module: u8,
    pub accounts: Vec<String>,
    pub codes: Vec<String>,
}

/// Enable the VAT module: flag + VAT accounts + VAT codes. Idempotent.
pub fn enable_vat_module(conn: &Connection, actor: &str) -> Result<VatEnableResult> {
    let kor_flag: i64 = conn.query_row("SELECT kor_flag FROM company", [], |r| r.get(0))?;
    if kor_flag == 1 {
        return Err(AppError::new(
            "KOR_ACTIVE",
            "this company uses the KOR (kleineondernemersregeling) — the VAT module cannot be enabled",
        ));
    }
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE company SET vat_module = 1 WHERE id = 1", [])?;
    for a in VAT_ACCOUNTS {
        if get_account_by_code(&tx, a.code)?.is_none() {
            create_account_from_seed(&tx, a)?;
        }
    }
    let insert_code = "INSERT OR IGNORE INTO vat_codes (code, rate_bp, type, eu_reverse, description) VALUES (?1, ?2, ?3, ?4, ?5)";
    for c in VAT_CODES {
        tx.execute(insert_code, params![c.code, c.rate_bp, c.vat_type, c.eu_reverse, c.description])?;
    }
    audit::record(&tx, actor, "vat.enable", Some("vat enable"), Some(&serde_json::json!({})), "ok", &[])?;
    tx.commit()?;
    Ok(VatEnableResult {
        vat_module: 1,
        accounts: VAT_ACCOUNTS.iter().map(|a| a.code.to_string()).collect(),
        codes: VAT_CODES.iter().map(|c| c.code.to_string()).collect(),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct VatCode {
    pub id: i64,
    pub code: String,
    pub rate_bp: i64,
    pub vat_type: String,
    pub eu_reverse: u8,
    pub description: Option<String>,
}

pub fn list_vat_codes(conn: &Connection) -> Result<Vec<VatCode>> {
    let mut stmt = conn.prepare("SELECT * FROM vat_codes ORDER BY rate_bp DESC, code")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(VatCode {
                id: row.get(0)?,
                code: row.get(1)?,
                rate_bp: row.get(2)?,
                vat_type: row.get(3)?,
                eu_reverse: row.get(4)?,
                description: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// VAT codes with their type labels (used by the readout later).
pub fn vat_type_of(conn: &Connection, code: &str) -> Result<Option<String>> {
    let t: Option<String> = conn
        .query_row("SELECT type FROM vat_codes WHERE code = ?1", [code], |r| r.get(0))
        .optional()?;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::accounts::{list_accounts, seed_default_chart};
    use crate::core::db::open_in_memory;

    fn company(conn: &Connection) {
        conn.execute(
            "INSERT INTO company (name, legal_form, vat_module, kor_flag) VALUES ('Test BV', 'bv', 0, 0)",
            [],
        )
        .unwrap();
        seed_default_chart(conn).unwrap();
    }

    #[test]
    fn enable_adds_accounts_and_codes() {
        let conn = open_in_memory().unwrap();
        company(&conn);
        assert!(!is_vat_enabled(&conn).unwrap());
        let res = enable_vat_module(&conn, "agent:test").unwrap();
        assert_eq!(res.accounts, vec!["1500", "2500"]);
        assert_eq!(res.codes.len(), 8);
        assert!(is_vat_enabled(&conn).unwrap());
        // idempotent
        enable_vat_module(&conn, "agent:test").unwrap();
        let accounts = list_accounts(&conn, None, false).unwrap();
        assert_eq!(accounts.len(), 30);
        let codes = list_vat_codes(&conn).unwrap();
        assert_eq!(codes.len(), 8);
    }

    #[test]
    fn kor_blocks_enable() {
        let conn = open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO company (name, legal_form, vat_module, kor_flag) VALUES ('KOR BV', 'bv', 0, 1)",
            [],
        )
        .unwrap();
        seed_default_chart(&conn).unwrap();
        let err = enable_vat_module(&conn, "human").unwrap_err();
        assert_eq!(err.code(), "KOR_ACTIVE");
    }
}
