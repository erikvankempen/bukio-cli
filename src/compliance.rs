// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Compliance calendar (mirrors src/compliance/index.js).
// Deadline rules for 31 jurisdictions + filing status tracking.

use crate::accounts::resolve_profile;
use crate::audit::{record, RecordArgs};
use crate::money::{BukioError, Result};
use crate::year_end::is_year_closed;
use rusqlite::Connection;
use serde_json::{json, Value};

fn compliance_error(code: &'static str, msg: impl Into<String>) -> BukioError {
    BukioError::new(code, msg.into())
}

/// Quarter deadline: Q1→Apr 30, Q2→Jul 31, Q3→Oct 31, Q4→Jan 31 next year.
pub fn quarter_deadline(period: &str) -> Result<(&str, String)> {
    let parts: Vec<&str> = period.split("-Q").collect();
    if parts.len() != 2 {
        return Err(compliance_error(
            "INVALID_PERIOD",
            format!("period '{period}' must be YYYY-Qn"),
        ));
    }
    let year: i32 = parts[0]
        .parse()
        .map_err(|_| compliance_error("INVALID_PERIOD", format!("bad year in '{period}'")))?;
    let qn: u32 = parts[1]
        .parse()
        .map_err(|_| compliance_error("INVALID_PERIOD", format!("bad quarter in '{period}'")))?;
    if qn < 1 || qn > 4 {
        return Err(compliance_error(
            "INVALID_PERIOD",
            format!("quarter must be 1-4, got '{qn}'"),
        ));
    }
    let month_day = match qn {
        1 => "04-30",
        2 => "07-31",
        3 => "10-31",
        _ => "01-31",
    };
    let y = if qn == 4 { year + 1 } else { year };
    Ok((period, format!("{y}-{month_day}")))
}

/// Last day of month N months after the fiscal year end month.
fn months_after_fy_end(fiscal_year_end: &str, year: i32, n: i32) -> String {
    let parts: Vec<&str> = fiscal_year_end.split('-').collect();
    let mm: i32 = parts[parts.len() - 2].parse().unwrap_or(12);
    let total = mm + n;
    let y = year + (total - 1).div_euclid(12);
    let m = ((total - 1).rem_euclid(12) + 1) as u32;
    // the last day of month m: the day before the 1st of the next month
    // (the old code read the 1st's day-of-month and always returned 01)
    let first_of_next = if m == 12 {
        chrono::NaiveDate::from_ymd_opt(y + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(y, m + 1, 1)
    };
    let last_day = first_of_next
        .map(|d| {
            (d - chrono::Duration::days(1))
                .format("%d")
                .to_string()
                .parse::<u32>()
                .unwrap_or(28)
        })
        .unwrap_or(28);
    format!("{y}-{m:02}-{last_day:02}")
}

/// Jaarrekening deadline: 13 months after FY end (art. 2:394 BW).
pub fn jaarrekening_deadline(fiscal_year_end: &str, year: i32) -> String {
    months_after_fy_end(fiscal_year_end, year, 13)
}

/// Quarter deadline on offset: q*3+offset months, day D.
fn quarter_deadline_on_offset(period: &str, offset: i32, day: u32) -> Result<String> {
    let parts: Vec<&str> = period.split("-Q").collect();
    if parts.len() != 2 {
        return Err(compliance_error(
            "INVALID_PERIOD",
            format!("expected YYYY-Qn, got '{period}'"),
        ));
    }
    let year: i32 = parts[0]
        .parse()
        .map_err(|_| compliance_error("INVALID_PERIOD", "bad year"))?;
    let qn: i32 = parts[1]
        .parse()
        .map_err(|_| compliance_error("INVALID_PERIOD", "bad quarter"))?;
    let m = qn * 3 + offset;
    let y = year + if m > 12 { 1 } else { 0 };
    let month = ((m - 1).rem_euclid(12) + 1) as u32;
    Ok(format!("{y}-{month:02}-{day:02}"))
}

/// Day D of the month following YYYY-MM.
fn day_of_next_month(period: &str, day: u32) -> Result<String> {
    let parts: Vec<&str> = period.split('-').collect();
    if parts.len() != 2 {
        return Err(compliance_error(
            "INVALID_PERIOD_SHAPE",
            format!("expected YYYY-MM, got '{period}'"),
        ));
    }
    let y: i32 = parts[0]
        .parse()
        .map_err(|_| compliance_error("INVALID_PERIOD", "bad year"))?;
    let m: u32 = parts[1]
        .parse()
        .map_err(|_| compliance_error("INVALID_PERIOD", "bad month"))?;
    if m < 1 || m > 12 {
        return Err(compliance_error("INVALID_PERIOD", "month must be 1-12"));
    }
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    Ok(format!("{ny}-{nm:02}-{day:02}"))
}

/// Is this filing type/period already filed?
pub fn is_filed(db: &Connection, filing_type: &str, period: &str) -> Result<bool> {
    if filing_type == "OB" {
        let result = db.query_row(
            "SELECT 1 FROM vat_returns WHERE type = 'OB' AND period = ?1 AND status = 'filed'",
            [period],
            |_| Ok(true),
        );
        return Ok(result.unwrap_or(false));
    }
    let result = db.query_row(
        "SELECT 1 FROM filings WHERE type = ?1 AND period = ?2",
        rusqlite::params![filing_type, period],
        |_| Ok(true),
    );
    Ok(result.unwrap_or(false))
}

/// Mark a filing as filed.
pub fn mark_filed(
    db: &Connection,
    filing_type: &str,
    period: &str,
    date: Option<&str>,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let profile = resolve_profile(db)?;
    let ft_arr = profile["compliance"]["filingTypes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let known_types: Vec<&str> = ft_arr.iter().filter_map(|ft| ft["type"].as_str()).collect();
    if !known_types.contains(&filing_type) {
        return Err(compliance_error(
            "INVALID_TYPE",
            format!("type must be one of {}", known_types.join(", ")),
        ));
    }
    if filing_type == "OB" {
        return Err(compliance_error(
            "INVALID_TYPE",
            "OB filings are recorded with 'bukio vat readout --mark-filed'",
        ));
    }
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let today_owned = today;
    let default_date = today_owned.clone();
    let filed_date = date.unwrap_or(&default_date);
    if dry_run {
        return Ok(
            json!({ "action": "compliance.mark", "type": filing_type, "period": period, "filed_at": filed_date, "dryRun": true }),
        );
    }
    db.execute(
        "INSERT INTO filings (type, period, filed_at, created_by) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(type, period) DO UPDATE SET filed_at = excluded.filed_at",
        rusqlite::params![filing_type, period, filed_date, actor],
    )
    .map_err(sql_err)?;
    record(
        db,
        RecordArgs {
            actor,
            action: "compliance.mark",
            command: Some("compliance mark"),
            args: Some(json!({ "type": filing_type, "period": period, "filed_at": filed_date })),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({ "type": filing_type, "period": period, "filed_at": filed_date }))
}

/// Compute deadline for a filing type using the profile's deadline rules.
fn compute_deadline(
    fiscal_year_end: &str,
    deadline_rule: &str,
    year: i32,
    period: &str,
) -> Result<String> {
    match deadline_rule {
        "nl-quarterly" => Ok(quarter_deadline(period)?.1),
        "nl-13-months" => Ok(jaarrekening_deadline(fiscal_year_end, year)),
        "lu-quarterly" => {
            // LU TVA: 15th of month after quarter end
            let parts: Vec<&str> = period.split("-Q").collect();
            let y: i32 = parts[0].parse().unwrap();
            let qn: i32 = parts[1].parse().unwrap();
            let m = qn * 3 + 1;
            let yy = if m > 12 { y + 1 } else { y };
            let mm = ((m - 1).rem_euclid(12) + 1) as u32;
            Ok(format!("{yy}-{mm:02}-15"))
        }
        "lu-monthly" => {
            let parts: Vec<&str> = period.split('-').collect();
            let y: i32 = parts[0].parse().unwrap();
            let m: u32 = parts[1].parse().unwrap();
            let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
            Ok(format!("{ny}-{nm:02}-15"))
        }
        "lu-annual" => Ok(format!("{}-03-01", year + 1)),
        "lu-7-months" => Ok(months_after_fy_end(fiscal_year_end, year, 7)),
        "gb-9-months" => Ok(months_after_fy_end(fiscal_year_end, year, 9)),
        "gb-ct600" => Ok(months_after_fy_end(fiscal_year_end, year, 12)),
        "be-vat-monthly" => day_of_next_month(period, 20),
        "be-quarterly" => quarter_deadline_on_offset(period, 1, 25),
        "be-7-months" => Ok(months_after_fy_end(fiscal_year_end, year, 7)),
        "de-ustva-quarterly" => quarter_deadline_on_offset(period, 1, 10),
        "de-annual-vat" => Ok(format!("{}-07-31", year + 1)),
        "de-12-months" => Ok(months_after_fy_end(fiscal_year_end, year, 12)),
        "dk-quarterly" => quarter_deadline_on_offset(period, 3, 1),
        "dk-5-months" => Ok(months_after_fy_end(fiscal_year_end, year, 5)),
        "fi-quarterly" => quarter_deadline_on_offset(period, 2, 12),
        "fi-8-months" => Ok(months_after_fy_end(fiscal_year_end, year, 8)),
        "no-bimonthly" => {
            let parts: Vec<&str> = period.split("-P").collect();
            let y: i32 = parts[0].parse().unwrap();
            let p: u32 = parts[1].parse().unwrap();
            match p {
                6 => Ok(format!("{}-02-10", y + 1)),
                _ => {
                    let sched = ["", "04-10", "06-10", "08-31", "10-10", "12-10"];
                    Ok(format!("{y}-{}", sched[p as usize]))
                }
            }
        }
        "no-7-months" => Ok(months_after_fy_end(fiscal_year_end, year, 7)),
        "se-quarterly" => {
            let parts: Vec<&str> = period.split("-Q").collect();
            let y: i32 = parts[0].parse().unwrap();
            let qn: i32 = parts[1].parse().unwrap();
            let m = qn * 3 + 2;
            let yy = if m > 12 { y + 1 } else { y };
            let mm = ((m - 1).rem_euclid(12) + 1) as u32;
            let day = if mm == 8 { 17 } else { 12 };
            Ok(format!("{yy}-{mm:02}-{day:02}"))
        }
        "se-7-months" => Ok(months_after_fy_end(fiscal_year_end, year, 7)),
        "at-uva-quarterly" => quarter_deadline_on_offset(period, 2, 15),
        "at-annual-vat" => Ok(format!("{}-06-30", year + 1)),
        "ie-bimonthly" => {
            let parts: Vec<&str> = period.split("-P").collect();
            let y: i32 = parts[0].parse().unwrap();
            let p: u32 = parts[1].parse().unwrap();
            match p {
                6 => Ok(format!("{}-01-23", y + 1)),
                _ => {
                    let sched = ["", "03-23", "05-23", "07-23", "09-23", "11-23"];
                    Ok(format!("{y}-{}", sched[p as usize]))
                }
            }
        }
        "ie-9-months" => Ok(months_after_fy_end(fiscal_year_end, year, 9)),
        "it-liquidazione-quarterly" => quarter_deadline_on_offset(period, 2, 16),
        "it-dichiarazione-iva" => Ok(format!("{}-04-30", year + 1)),
        "it-bilancio" => Ok(months_after_fy_end(fiscal_year_end, year, 5)),
        "es-303-quarterly" => {
            let parts: Vec<&str> = period.split("-Q").collect();
            let y: i32 = parts[0].parse().unwrap();
            let qn: i32 = parts[1].parse().unwrap();
            if qn == 4 {
                return Ok(format!("{}-01-30", y + 1));
            }
            Ok(format!("{}-{:02}-20", y, qn * 3 + 1))
        }
        "es-390" => Ok(format!("{}-01-30", year + 1)),
        "es-200" => {
            let mm2 = 7u32;
            let yy2 = year + (mm2 as i32 - 1).div_euclid(12);
            Ok(format!(
                "{yy2}-{:02}-25",
                ((mm2 as i32 - 1).rem_euclid(12) + 1) as u32
            ))
        }
        "es-7-months" => Ok(months_after_fy_end(fiscal_year_end, year, 7)),
        "pt-dp-quarterly" => quarter_deadline_on_offset(period, 2, 20),
        "pt-irc" => Ok(format!("{}-05-31", year + 1)),
        "pt-ies" => Ok(format!("{}-07-15", year + 1)),
        // Phase E
        "bg-vat-monthly" => day_of_next_month(period, 14),
        "bg-annual-accounts" | "bg-cit" => Ok(months_after_fy_end(fiscal_year_end, year, 6)),
        "hr-vat-monthly" => day_of_next_month(period, 20),
        "hr-annual-accounts" | "hr-cit" => Ok(months_after_fy_end(fiscal_year_end, year, 4)),
        "si-vat-monthly" => day_of_next_month(period, 20),
        "si-annual-accounts" => Ok(months_after_fy_end(fiscal_year_end, year, 8)),
        "si-ddpo" => Ok(format!("{}-03-31", year + 1)),
        "ee-vat-monthly" => day_of_next_month(period, 20),
        "ee-annual-accounts" => Ok(months_after_fy_end(fiscal_year_end, year, 6)),
        "lv-vat-monthly" => day_of_next_month(period, 20),
        "lv-annual-accounts" => Ok(months_after_fy_end(fiscal_year_end, year, 7)),
        "lt-vat-monthly" => day_of_next_month(period, 25),
        "lt-annual-accounts" => Ok(months_after_fy_end(fiscal_year_end, year, 4)),
        "lt-cit" => Ok(format!("{}-10-01", year + 1)),
        "mt-vat-quarterly" => quarter_deadline_on_offset(period, 2, 15),
        "mt-annual-accounts" => Ok(months_after_fy_end(fiscal_year_end, year, 10)),
        "mt-cit" => Ok(months_after_fy_end(fiscal_year_end, year, 9)),
        "cy-vat-quarterly" => quarter_deadline_on_offset(period, 2, 10),
        "cy-annual-accounts" => Ok(months_after_fy_end(fiscal_year_end, year, 10)),
        "cy-td4" => Ok(format!("{}-01-31", year + 2)),
        // Phase F
        "cz-vat-monthly" => day_of_next_month(period, 25),
        "cz-annual-accounts" => Ok(months_after_fy_end(fiscal_year_end, year, 6)),
        "cz-cit" => Ok(months_after_fy_end(fiscal_year_end, year, 3)),
        "sk-vat-monthly" => day_of_next_month(period, 25),
        "sk-annual-accounts" => Ok(months_after_fy_end(fiscal_year_end, year, 6)),
        "sk-cit" => Ok(format!("{}-03-31", year + 1)),
        "gr-vat-monthly" => day_of_next_month(period, 26),
        "gr-annual-accounts" => Ok(months_after_fy_end(fiscal_year_end, year, 10)),
        "gr-cit" => Ok(format!("{}-06-30", year + 1)),
        "pl-vat-monthly" => day_of_next_month(period, 25),
        "pl-annual-accounts" => Ok(months_after_fy_end(fiscal_year_end, year, 6)),
        "pl-cit" => Ok(format!("{}-03-31", year + 1)),
        "hu-vat-monthly" => day_of_next_month(period, 20),
        "hu-annual-accounts" | "hu-cit" => Ok(months_after_fy_end(fiscal_year_end, year, 5)),
        "ro-vat-monthly" => day_of_next_month(period, 25),
        "ro-annual-accounts" => Ok(months_after_fy_end(fiscal_year_end, year, 5)),
        "ro-cit" => Ok(format!("{}-06-25", year + 1)),
        // Phase G — Kosovo
        "xk-vat-monthly" => day_of_next_month(period, 20),
        "xk-annual-accounts" | "xk-cit" => Ok(format!("{}-03-31", year + 1)),
        // US
        "us-941" => Ok(quarter_deadline(period)?.1),
        _ => Err(compliance_error(
            "DEADLINE_RULE_NOT_FOUND",
            format!("rule '{deadline_rule}' is not implemented"),
        )),
    }
}

/// Full compliance status for a year.
pub fn compliance_status(db: &Connection, year: i32) -> Result<Value> {
    let company_row = db
        .query_row(
            "SELECT name, fiscal_year_end FROM company WHERE id = 1",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
        )
        .map_err(|_| compliance_error("NOT_INITIALISED", "company database not initialised"))?;
    let company_name = company_row.0;
    let fy_end = company_row.1.unwrap_or_else(|| "12-31".into());

    let profile = resolve_profile(db)?;
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut obligations = Vec::new();

    let filing_types = profile["compliance"]["filingTypes"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    for ft in &filing_types {
        let ftype = ft["type"].as_str().unwrap_or("");
        let rule_name = ft["deadlineRule"].as_str().unwrap_or("");
        let shape = ft["periodShape"].as_str().unwrap_or("");

        match shape {
            "YYYY-Qn" => {
                for qn in 1..=4i32 {
                    let period = format!("{year}-Q{qn}");
                    if let Ok(deadline) = compute_deadline(&fy_end, rule_name, year, &period) {
                        let filed = is_filed(db, ftype, &period)?;
                        let status = if filed {
                            "filed"
                        } else if deadline < today {
                            "overdue"
                        } else {
                            "open"
                        };
                        obligations.push(json!({ "type": ftype, "period": period, "deadline": deadline, "status": status }));
                    }
                }
                // prev Q4
                let prev_period = format!("{}-Q4", year - 1);
                if let Ok(deadline) = compute_deadline(&fy_end, rule_name, year - 1, &prev_period) {
                    if deadline >= format!("{year}-01-01") {
                        let filed = is_filed(db, ftype, &prev_period)?;
                        let status = if filed {
                            "filed"
                        } else if deadline < today {
                            "overdue"
                        } else {
                            "open"
                        };
                        obligations.push(json!({ "type": ftype, "period": prev_period, "deadline": deadline, "status": status }));
                    }
                }
            }
            "YYYY-MM" => {
                for mm in 1..=11u32 {
                    let period = format!("{year}-{mm:02}");
                    if let Ok(deadline) = compute_deadline(&fy_end, rule_name, year, &period) {
                        if deadline >= format!("{year}-01-01") {
                            let filed = is_filed(db, ftype, &period)?;
                            let status = if filed {
                                "filed"
                            } else if deadline < today {
                                "overdue"
                            } else {
                                "open"
                            };
                            obligations.push(json!({ "type": ftype, "period": period, "deadline": deadline, "status": status }));
                        }
                    }
                }
                let prev_dec = format!("{}-12", year - 1);
                if let Ok(deadline) = compute_deadline(&fy_end, rule_name, year - 1, &prev_dec) {
                    if deadline >= format!("{year}-01-01") {
                        let filed = is_filed(db, ftype, &prev_dec)?;
                        let status = if filed {
                            "filed"
                        } else if deadline < today {
                            "overdue"
                        } else {
                            "open"
                        };
                        obligations.push(json!({ "type": ftype, "period": prev_dec, "deadline": deadline, "status": status }));
                    }
                }
            }
            "YYYY-Pn" => {
                for pn in 1..=6i32 {
                    let period = format!("{year}-P{pn}");
                    if let Ok(deadline) = compute_deadline(&fy_end, rule_name, year, &period) {
                        if deadline >= format!("{year}-01-01") {
                            let filed = is_filed(db, ftype, &period)?;
                            let status = if filed {
                                "filed"
                            } else if deadline < today {
                                "overdue"
                            } else {
                                "open"
                            };
                            obligations.push(json!({ "type": ftype, "period": period, "deadline": deadline, "status": status }));
                        }
                    }
                }
                let prev_p6 = format!("{}-P6", year - 1);
                if let Ok(deadline) = compute_deadline(&fy_end, rule_name, year - 1, &prev_p6) {
                    if deadline >= format!("{year}-01-01") {
                        let filed = is_filed(db, ftype, &prev_p6)?;
                        let status = if filed {
                            "filed"
                        } else if deadline < today {
                            "overdue"
                        } else {
                            "open"
                        };
                        obligations.push(json!({ "type": ftype, "period": prev_p6, "deadline": deadline, "status": status }));
                    }
                }
            }
            "YYYY" => {
                let period_str = year.to_string();
                if let Ok(deadline) = compute_deadline(&fy_end, rule_name, year, &period_str) {
                    let filed = is_filed(db, ftype, &period_str)?;
                    let status = if filed {
                        "filed"
                    } else if deadline < today {
                        "overdue"
                    } else {
                        "open"
                    };
                    let closed = is_year_closed(db, &year_str(year))?;
                    obligations.push(json!({ "type": ftype, "period": period_str, "deadline": deadline, "status": status, "books_closed": closed }));
                }
            }
            _ => {}
        }
    }

    let filed_count = obligations
        .iter()
        .filter(|o| o["status"] == "filed")
        .count();
    let overdue_count = obligations
        .iter()
        .filter(|o| o["status"] == "overdue")
        .count();
    let open_count = obligations.iter().filter(|o| o["status"] == "open").count();

    Ok(json!({
        "year": year,
        "company": company_name,
        "as_of": today,
        "obligations": obligations,
        "summary": { "filed": filed_count, "overdue": overdue_count, "open": open_count },
    }))
}

fn year_str(y: i32) -> String {
    y.to_string()
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_deadline_basic() {
        let (_, d) = quarter_deadline("2026-Q1").unwrap();
        assert_eq!(d, "2026-04-30");
        let (_, d) = quarter_deadline("2026-Q4").unwrap();
        assert_eq!(d, "2027-01-31");
    }

    #[test]
    fn day_of_next_month_basic() {
        assert_eq!(day_of_next_month("2026-03", 20).unwrap(), "2026-04-20");
        assert_eq!(day_of_next_month("2026-12", 15).unwrap(), "2027-01-15");
    }

    #[test]
    fn compute_deadline_nl_quarterly() {
        assert_eq!(
            compute_deadline("12-31", "nl-quarterly", 2026, "2026-Q1").unwrap(),
            "2026-04-30"
        );
    }

    #[test]
    fn compute_deadline_be_monthly() {
        assert_eq!(
            compute_deadline("12-31", "be-vat-monthly", 2026, "2026-03").unwrap(),
            "2026-04-20"
        );
    }

    #[test]
    fn compute_deadline_de_quarterly() {
        assert_eq!(
            compute_deadline("12-31", "de-ustva-quarterly", 2026, "2026-Q1").unwrap(),
            "2026-04-10"
        );
    }

    #[test]
    fn no_bimonthly_p6() {
        assert_eq!(
            compute_deadline("12-31", "no-bimonthly", 2026, "2026-P6").unwrap(),
            "2027-02-10"
        );
    }

    #[test]
    fn se_quarterly_august_exception() {
        assert_eq!(
            compute_deadline("12-31", "se-quarterly", 2026, "2026-Q2").unwrap(),
            "2026-08-17"
        );
        assert_eq!(
            compute_deadline("12-31", "se-quarterly", 2026, "2026-Q1").unwrap(),
            "2026-05-12"
        );
    }
}
