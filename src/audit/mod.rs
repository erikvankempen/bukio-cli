//! Audit log — append-only record of every mutation (human or agent).

use crate::error::Result;
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::Value;

pub fn record(
    conn: &Connection,
    actor: &str,
    action: &str,
    command: Option<&str>,
    args: Option<&Value>,
    outcome: &str,
    entry_ids: &[i64],
) -> Result<()> {
    let args_json = args.map(|v| v.to_string());
    let entry_ids_json = if entry_ids.is_empty() {
        None
    } else {
        Some(serde_json::to_string(entry_ids).unwrap_or_default())
    };
    conn.execute(
        "INSERT INTO audit_log (actor, action, command, args_json, outcome, entry_ids) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![actor, action, command, args_json, outcome, entry_ids_json],
    )?;
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AuditRow {
    pub id: i64,
    pub ts: String,
    pub actor: String,
    pub action: String,
    pub command: Option<String>,
    pub args: Option<Value>,
    pub outcome: String,
    pub entry_ids: Vec<i64>,
}

pub fn list(conn: &Connection, since: Option<&str>, actor: Option<&str>, limit: i64) -> Result<Vec<AuditRow>> {
    let mut sql = String::from("SELECT * FROM audit_log");
    let mut clauses: Vec<String> = Vec::new();
    let mut params_vec: Vec<String> = Vec::new();
    if let Some(s) = since {
        clauses.push("ts >= ?".to_string());
        params_vec.push(s.to_string());
    }
    if let Some(a) = actor {
        clauses.push("actor = ?".to_string());
        params_vec.push(a.to_string());
    }
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY id DESC LIMIT ?");

    let mut stmt = conn.prepare(&sql)?;
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = params_vec.into_iter().map(|p| Box::new(p) as Box<dyn rusqlite::ToSql>).collect();
    args.push(Box::new(limit));
    let param_refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let mut query = stmt.query(rusqlite::params_from_iter(param_refs))?;
    let mut rows: Vec<AuditRow> = Vec::new();
    while let Some(row) = query.next()? {
        rows.push(AuditRow {
            id: row.get(0)?,
            ts: row.get(1)?,
            actor: row.get(2)?,
            action: row.get(3)?,
            command: row.get(4)?,
            args: row
                .get::<_, Option<String>>(5)?
                .and_then(|s| serde_json::from_str(&s).ok()),
            outcome: row.get(6)?,
            entry_ids: row
                .get::<_, Option<String>>(7)?
                .map(|s| serde_json::from_str::<Vec<i64>>(&s).unwrap_or_default())
                .unwrap_or_default(),
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db::open_in_memory;

    #[test]
    fn record_and_list() {
        let conn = open_in_memory().unwrap();
        record(&conn, "agent:test", "entry.create", Some("entry add"), Some(&serde_json::json!({"a": 1})), "ok", &[1, 2]).unwrap();
        record(&conn, "human", "company.init", Some("init"), None, "ok", &[]).unwrap();
        let rows = list(&conn, None, None, 50).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].action, "company.init");
        assert_eq!(rows[1].args, Some(serde_json::json!({"a": 1})));
        assert_eq!(rows[1].entry_ids, vec![1, 2]);
        let by_agent = list(&conn, None, Some("agent:test"), 50).unwrap();
        assert_eq!(by_agent.len(), 1);
    }

    #[test]
    fn audit_is_append_only() {
        let conn = open_in_memory().unwrap();
        record(&conn, "human", "x", None, None, "ok", &[]).unwrap();
        let err = conn.execute("UPDATE audit_log SET outcome='hacked'", []).unwrap_err();
        assert!(err.to_string().contains("append-only"));
        let err = conn.execute("DELETE FROM audit_log", []).unwrap_err();
        assert!(err.to_string().contains("append-only"));
    }
}
