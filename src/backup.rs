// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Backup / restore — SQLite backup API + validated restore.

use crate::audit::{record, RecordArgs};
use crate::money::{BukioError, Result};
use rusqlite::backup::Backup;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

fn backup_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
        .join(".bukio")
        .join("backups")
}

fn default_backup_path() -> PathBuf {
    let ts = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    backup_dir().join(format!("bukio-{ts}.db"))
}

fn validate_backup_file(path: &Path) -> Result<()> {
    let db = Connection::open(path).map_err(|e| {
        BukioError::new(
            "INVALID_BACKUP",
            format!("'{}' is not a valid SQLite database: {e}", path.display()),
        )
    })?;
    let has_company: bool = db
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='company'",
            [],
            |r| r.get::<_, i32>(0),
        )
        .map(|_| true)
        .unwrap_or(false);
    if !has_company {
        return Err(BukioError::new(
            "INVALID_BACKUP",
            format!("'{}' is not a bukio database", path.display()),
        ));
    }
    Ok(())
}

/// Prune old backups in the default backup directory, keeping the newest N.
pub fn prune_backups(keep: usize, dry_run: bool) -> Result<Vec<String>> {
    let dir = backup_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut entries: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(&dir)
        .map_err(|e| BukioError::new("IO_ERROR", format!("cannot read {}: {e}", dir.display())))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with("bukio-") && name.ends_with(".db")
        })
        .filter_map(|e| {
            let mtime = e.metadata().ok()?.modified().ok()?;
            Some((e.path(), mtime))
        })
        .collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let pruned: Vec<String> = entries[keep..]
        .iter()
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    if !dry_run {
        for name in &pruned {
            fs::remove_file(dir.join(name)).ok();
        }
    }
    Ok(pruned)
}

/// Backup the database.
pub fn cmd_backup(
    db_path: &str,
    out: Option<&str>,
    keep: Option<usize>,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let dest = match out {
        Some(p) => PathBuf::from(p),
        None => default_backup_path(),
    };
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).ok();
    }

    if dry_run {
        let pruned = if let Some(k) = keep {
            prune_backups(k, true)?
        } else {
            vec![]
        };
        return Ok(json!({
            "action": "backup", "from": db_path, "to": dest.display().to_string(),
            "pruned": pruned, "dryRun": true,
        }));
    }

    let src = Connection::open(db_path)
        .map_err(|e| BukioError::new("DB_ERROR", format!("cannot open source: {e}")))?;
    let mut dest_conn = Connection::open(&dest)
        .map_err(|e| BukioError::new("DB_ERROR", format!("cannot create: {e}")))?;

    let backup = Backup::new(&src, &mut dest_conn)
        .map_err(|e| BukioError::new("DB_ERROR", format!("backup init failed: {e}")))?;
    backup
        .run_to_completion(500, std::time::Duration::ZERO, None)
        .map_err(|e| BukioError::new("DB_ERROR", format!("backup failed: {e}")))?;

    let bytes = fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    let pruned = if let Some(k) = keep {
        prune_backups(k, false)?
    } else {
        vec![]
    };
    Ok(
        json!({"path": dest.display().to_string(), "bytes": bytes, "source": db_path, "pruned": pruned}),
    )
}

/// Restore from a backup file.
pub fn cmd_restore(from: &str, to: &str, force: bool, actor: &str, dry_run: bool) -> Result<Value> {
    let src_path = Path::new(from);
    if !src_path.exists() {
        return Err(BukioError::new(
            "FILE_NOT_FOUND",
            format!("backup file '{from}' does not exist"),
        ));
    }

    validate_backup_file(src_path)?;

    if !force && Path::new(to).exists() {
        return Err(BukioError::new(
            "RESTORE_EXISTS",
            format!("target '{to}' already exists — pass --force to overwrite"),
        ));
    }

    if dry_run {
        return Ok(json!({
            "action": "restore", "from": from, "to": to, "dryRun": true,
        }));
    }

    fs::copy(src_path, to).map_err(|e| BukioError::new("IO_ERROR", format!("copy failed: {e}")))?;

    let restored = Connection::open(to)
        .map_err(|e| BukioError::new("DB_ERROR", format!("open restored db: {e}")))?;
    record(
        &restored,
        RecordArgs {
            actor,
            action: "restore",
            command: Some("restore"),
            args: Some(json!({"from": from, "to": to})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;

    Ok(json!({"to": to, "from": from, "restored": true}))
}
