// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// In-database document attachments (mirrors src/core/attachments.js).

use crate::audit::{record, RecordArgs};
use crate::money::{BukioError, Result};
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

fn attachment_error(code: &'static str, msg: impl Into<String>) -> BukioError {
    BukioError::new(code, msg.into())
}

fn sql_err(e: rusqlite::Error) -> BukioError {
    BukioError::new("DB_ERROR", e.to_string())
}

fn mime_from_ext(ext: &str) -> &str {
    match ext.to_lowercase().as_str() {
        ".pdf" => "application/pdf",
        ".jpg" | ".jpeg" => "image/jpeg",
        ".png" => "image/png",
        ".gif" => "image/gif",
        ".svg" => "image/svg+xml",
        ".xml" => "application/xml",
        ".eml" => "message/rfc822",
        ".txt" => "text/plain",
        ".csv" => "text/csv",
        ".html" => "text/html",
        ".doc" => "application/msword",
        ".docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ".xls" => "application/vnd.ms-excel",
        ".xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ".zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn ref_exists(db: &Connection, kind: &str, ref_id: i64) -> Result<bool> {
    let table = if kind == "invoice" {
        "invoices"
    } else {
        "journal_entries"
    };
    let sql = format!("SELECT id FROM {table} WHERE id = ?1");
    let r = db.query_row(&sql, [ref_id], |_| Ok(true));
    Ok(r.unwrap_or(false))
}

pub fn add_attachment(
    db: &Connection,
    kind: &str,
    ref_id: i64,
    file_path: &str,
    note: Option<&str>,
    store: &str,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    if !["invoice", "entry"].contains(&kind) {
        return Err(attachment_error(
            "INVALID_KIND",
            format!("attachment kind must be 'invoice' or 'entry', got '{kind}'"),
        ));
    }
    if ref_id <= 0 {
        return Err(attachment_error(
            "REF_REQUIRED",
            "pass exactly one of --invoice <id> or --entry <id>",
        ));
    }
    if !["db", "file"].contains(&store) {
        return Err(attachment_error(
            "INVALID_STORE",
            format!("attachment store must be 'db' or 'file', got '{store}'"),
        ));
    }
    if !ref_exists(db, kind, ref_id)? {
        return Err(attachment_error(
            "NOT_FOUND",
            format!("{kind} {ref_id} does not exist"),
        ));
    }
    let meta = std::fs::metadata(file_path).map_err(|_| {
        attachment_error(
            "ATTACHMENT_FILE_NOT_FOUND",
            format!("file '{file_path}' does not exist"),
        )
    })?;
    if !meta.is_file() {
        return Err(attachment_error(
            "ATTACHMENT_FILE_NOT_FOUND",
            format!("'{file_path}' is not a file"),
        ));
    }
    let size = meta.len() as usize;
    if size > MAX_ATTACHMENT_BYTES {
        return Err(attachment_error(
            "ATTACHMENT_TOO_LARGE",
            format!("file is {size} bytes — the cap is {MAX_ATTACHMENT_BYTES} (25 MB)"),
        ));
    }
    if size == 0 {
        return Err(attachment_error(
            "ATTACHMENT_EMPTY",
            "file is empty — nothing to attach",
        ));
    }
    let bytes = std::fs::read(file_path).map_err(|e| BukioError::new("IO_ERROR", e.to_string()))?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let dup = db.query_row(
        "SELECT id FROM attachments WHERE kind = ?1 AND ref_id = ?2 AND sha256 = ?3",
        rusqlite::params![kind, ref_id, sha256],
        |r| r.get::<_, i64>(0),
    );
    if let Ok(id) = dup {
        return Err(attachment_error(
            "ATTACHMENT_DUPLICATE",
            format!("the same file is already attached (attachment {id})"),
        ));
    }
    let file_name = std::path::Path::new(file_path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let ext = std::path::Path::new(&file_name)
        .extension()
        .unwrap_or_default()
        .to_string_lossy();
    let ext_dot = format!(".{ext}");
    let mime = mime_from_ext(&ext_dot);
    if dry_run {
        let dest = if store == "file" {
            Some(attachments_dir(db).join(&sha256))
        } else {
            None
        };
        return Ok(
            json!({"action": "attachments.add", "kind": kind, "ref_id": ref_id, "file_name": file_name, "mime": mime, "size": size, "sha256": sha256, "mode": store, "path": dest.map(|d| d.to_string_lossy().to_string()), "dryRun": true}),
        );
    }
    // file mode: copy into <db-dir>/<db>-attachments/<sha256> (JS parity)
    let (data_opt, path_opt) = if store == "db" {
        (Some(bytes), None)
    } else {
        let dir = attachments_dir(db);
        std::fs::create_dir_all(&dir)
            .map_err(|e| attachment_error("IO_ERROR", format!("cannot create {dir:?}: {e}")))?;
        let dest = dir.join(&sha256);
        std::fs::copy(file_path, &dest).map_err(|e| {
            attachment_error(
                "IO_ERROR",
                format!("cannot copy {file_path} to {dest:?}: {e}"),
            )
        })?;
        (None, Some(dest.to_string_lossy().to_string()))
    };
    db.execute(
        "INSERT INTO attachments (kind, ref_id, file_name, mime, size, sha256, mode, data, path, note, created_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![kind, ref_id, file_name, mime, size as i64, sha256, store, data_opt, path_opt, note, actor],
    ).map_err(sql_err)?;
    let id = db.last_insert_rowid();
    record(
        db,
        RecordArgs {
            actor,
            action: "attachments.add",
            command: Some("attach add"),
            args: Some(
                json!({"attachment_id": id, "kind": kind, "ref_id": ref_id, "file_name": file_name, "size": size, "mode": store, "sha256": sha256}),
            ),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(
        json!({"id": id, "kind": kind, "ref_id": ref_id, "file_name": file_name, "mime": mime, "size": size, "sha256": sha256, "mode": store, "note": note, "created_by": actor}),
    )
}

/// Metadata only — never selects data column.
pub fn list_attachments(db: &Connection, kind: &str, ref_id: i64) -> Result<Vec<Value>> {
    let mut stmt = db.prepare(
        "SELECT id, kind, ref_id, file_name, mime, size, sha256, mode, path, note, created_by, created_at FROM attachments WHERE kind = ?1 AND ref_id = ?2 ORDER BY id"
    ).map_err(sql_err)?;
    let rows = stmt.query_map(rusqlite::params![kind, ref_id], |r| {
        Ok(json!({
            "id": r.get::<_, i64>(0)?, "kind": r.get::<_, String>(1)?, "ref_id": r.get::<_, i64>(2)?,
            "file_name": r.get::<_, String>(3)?, "mime": r.get::<_, String>(4)?, "size": r.get::<_, i64>(5)?,
            "sha256": r.get::<_, String>(6)?, "mode": r.get::<_, String>(7)?,
            "path": r.get::<_, Option<String>>(8)?, "note": r.get::<_, Option<String>>(9)?,
            "created_by": r.get::<_, String>(10)?, "created_at": r.get::<_, String>(11)?,
        }))
    }).map_err(sql_err)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// <db-dir>/<db-name-without-.db>-attachments (JS parity: same helper)
fn attachments_dir(db: &Connection) -> std::path::PathBuf {
    let db_path = db
        .path()
        .map(std::path::Path::new)
        .unwrap_or(std::path::Path::new("bukio.db"));
    let dir = db_path.parent().unwrap_or(std::path::Path::new("."));
    let stem = db_path
        .file_name()
        .map(|f| f.to_string_lossy().replace(".db", ""))
        .unwrap_or_else(|| "bukio".into());
    dir.join(format!("{stem}-attachments"))
}

/// Write the attachment's bytes to `out`. `db` mode reads the blob; `file`
/// mode copies the stored path. Refuses to overwrite unless `force`.
pub fn extract_attachment(db: &Connection, id: i64, out: &str, force: bool) -> Result<Value> {
    let row = db.query_row(
        "SELECT mode, data, path, file_name FROM attachments WHERE id = ?1",
        [id],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<Vec<u8>>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
            ))
        },
    );
    let (mode, data, path, file_name) = match row {
        Ok(v) => v,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return Err(attachment_error(
                "ATTACHMENT_NOT_FOUND",
                format!("attachment {id} does not exist"),
            ));
        }
        Err(e) => return Err(sql_err(e)),
    };
    let bytes = match mode.as_str() {
        "db" => data.ok_or_else(|| attachment_error("NO_DATA", "attachment has no stored blob"))?,
        "file" => {
            let p = path.ok_or_else(|| attachment_error("NO_PATH", "attachment has no path"))?;
            std::fs::read(&p)
                .map_err(|e| attachment_error("IO_ERROR", format!("cannot read {p}: {e}")))?
        }
        other => {
            return Err(attachment_error(
                "INVALID_STORE",
                format!("unknown store mode '{other}'"),
            ));
        }
    };
    if std::path::Path::new(out).exists() && !force {
        return Err(attachment_error(
            "FILE_EXISTS",
            format!("{out} already exists — pass --force to overwrite"),
        ));
    }
    if let Some(parent) = std::path::Path::new(out).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| attachment_error("IO_ERROR", format!("cannot create {parent:?}: {e}")))?;
    }
    std::fs::write(out, &bytes)
        .map_err(|e| attachment_error("IO_ERROR", format!("cannot write {out}: {e}")))?;
    Ok(json!({
        "id": id, "file_name": file_name, "mode": mode, "size": bytes.len(),
        "sha256": format!("{:x}", Sha256::digest(&bytes)), "out": out,
    }))
}

pub fn get_attachment(db: &Connection, id: i64) -> Result<Value> {
    let row = db.query_row(
        "SELECT id, kind, ref_id, file_name, mime, size, sha256, mode, path, note, created_by, created_at FROM attachments WHERE id = ?1",
        [id], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "kind": r.get::<_, String>(1)?, "ref_id": r.get::<_, i64>(2)?,
                "file_name": r.get::<_, String>(3)?, "mime": r.get::<_, String>(4)?, "size": r.get::<_, i64>(5)?,
                "sha256": r.get::<_, String>(6)?, "mode": r.get::<_, String>(7)?,
                "path": r.get::<_, Option<String>>(8)?, "note": r.get::<_, Option<String>>(9)?,
                "created_by": r.get::<_, String>(10)?, "created_at": r.get::<_, String>(11)?,
            }))
        },
    );
    match row {
        Ok(v) => Ok(v),
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(attachment_error(
            "ATTACHMENT_NOT_FOUND",
            format!("attachment {id} does not exist"),
        )),
        Err(e) => Err(sql_err(e)),
    }
}

pub fn remove_attachment(db: &Connection, id: i64, actor: &str, dry_run: bool) -> Result<Value> {
    let row = db
        .query_row(
            "SELECT id, kind, ref_id, file_name, mode, path FROM attachments WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .map_err(|_| {
            attachment_error(
                "ATTACHMENT_NOT_FOUND",
                format!("attachment {id} does not exist"),
            )
        })?;
    if dry_run {
        return Ok(
            json!({"action": "attachments.remove", "id": id, "kind": row.1, "ref_id": row.2, "file_name": row.3, "mode": row.4, "dryRun": true}),
        );
    }
    db.execute("DELETE FROM attachments WHERE id = ?1", [id])
        .map_err(sql_err)?;
    record(
        db,
        RecordArgs {
            actor,
            action: "attachments.remove",
            command: Some("attach remove"),
            args: Some(
                json!({"attachment_id": id, "kind": row.1, "ref_id": row.2, "file_name": row.3, "mode": row.4}),
            ),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    Ok(json!({"id": id, "kind": row.1, "ref_id": row.2, "file_name": row.3, "mode": row.4}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_db;
    use std::io::Write;

    fn test_db() -> Connection {
        let d = open_db(":memory:").unwrap();
        d.execute("INSERT INTO company (name) VALUES ('AttachCo')", [])
            .unwrap();
        crate::accounts::seed_default_chart(&d).unwrap();
        // create a minimal entry for attachment
        d.execute("INSERT INTO journal_entries (date, description, source, state, created_by) VALUES ('2026-01-01', 'test', 'manual', 'draft', 'human:erik')", []).unwrap();
        d
    }

    fn temp_file(content: &[u8]) -> String {
        let path = format!("/tmp/test_attach_{}.txt", std::process::id());
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content).unwrap();
        path
    }

    #[test]
    fn add_and_list_attachment() {
        let d = test_db();
        let path = temp_file(b"hello world");
        let result =
            add_attachment(&d, "entry", 1, &path, None, "db", "human:erik", false).unwrap();
        assert_eq!(result["kind"], "entry");
        assert_eq!(result["size"], 11);
        let list = list_attachments(&d, "entry", 1).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(
            list[0]["file_name"]
                .as_str()
                .unwrap()
                .contains("test_attach"),
            true
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn duplicate_rejected() {
        let d = test_db();
        let path = temp_file(b"dup test");
        add_attachment(&d, "entry", 1, &path, None, "db", "human:erik", false).unwrap();
        let err =
            add_attachment(&d, "entry", 1, &path, None, "db", "human:erik", false).unwrap_err();
        assert_eq!(err.code, "ATTACHMENT_DUPLICATE");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn remove_attachment_test() {
        let d = test_db();
        let path = temp_file(b"to remove");
        let result =
            add_attachment(&d, "entry", 1, &path, None, "db", "human:erik", false).unwrap();
        let id = result["id"].as_i64().unwrap();
        let removed = remove_attachment(&d, id, "human:erik", false).unwrap();
        assert_eq!(removed["id"], id);
        assert!(list_attachments(&d, "entry", 1).unwrap().is_empty());
        std::fs::remove_file(&path).ok();
    }
}
