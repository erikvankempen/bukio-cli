// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Backup / restore — SQLite backup API + validated restore.

use crate::audit::{record, RecordArgs};
use crate::money::{BukioError, Result};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use rusqlite::backup::Backup;
use rusqlite::Connection;
use scrypt::{scrypt, Params as ScryptParams};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// "BUKIOENC1" | salt(16) | iv(12) | tag(16) | ciphertext (JS parity)
const BACKUP_MAGIC: &str = "BUKIOENC1";
const SALT_LEN: usize = 16;
const IV_LEN: usize = 12;
const KEY_LEN: usize = 32;
const SCRYPT_LOG_N: u8 = 15; // N = 2^15, matches JS { N: 2**15, r: 8, p: 1 }
const SCRYPT_R: u32 = 8;
const SCRYPT_P: u32 = 1;

fn backup_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
        .join(".bukio")
        .join("backups")
}

fn default_backup_path(encrypted: bool) -> PathBuf {
    let ts = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    backup_dir().join(format!(
        "bukio-{ts}.db{}",
        if encrypted { ".enc" } else { "" }
    ))
}

/// True when the file starts with the BUKIOENC1 magic.
pub fn is_encrypted_backup(path: &Path) -> bool {
    let mut buf = [0u8; BACKUP_MAGIC.len()];
    match fs::File::open(path).and_then(|mut f| {
        use std::io::Read;
        f.read_exact(&mut buf)
    }) {
        Ok(()) => &buf == BACKUP_MAGIC.as_bytes(),
        Err(_) => false,
    }
}

/// Passphrase from --passphrase or BUKIO_BACKUP_PASSPHRASE; throws if absent.
fn backup_passphrase(passphrase: Option<&str>) -> Result<String> {
    let pass = passphrase
        .filter(|p| !p.is_empty())
        .map(String::from)
        .or_else(|| std::env::var("BUKIO_BACKUP_PASSPHRASE").ok().filter(|p| !p.is_empty()));
    match pass {
        Some(p) => Ok(p),
        None => Err(BukioError::new(
            "BACKUP_PASSPHRASE_REQUIRED",
            "pass --passphrase or set BUKIO_BACKUP_PASSPHRASE (needed for --encrypt / encrypted restore)",
        )),
    }
}

fn derive_key(pass: &str, salt: &[u8]) -> Result<Vec<u8>> {
    let params = ScryptParams::new(SCRYPT_LOG_N, SCRYPT_R, SCRYPT_P, KEY_LEN)
        .map_err(|e| BukioError::new("DB_ERROR", format!("scrypt params: {e}")))?;
    let mut key = vec![0u8; KEY_LEN];
    scrypt(pass.as_bytes(), salt, &params, &mut key)
        .map_err(|e| BukioError::new("DB_ERROR", format!("scrypt failed: {e}")))?;
    Ok(key)
}

/// AES-256-GCM encrypt `src` into `dest`; returns the dest size.
fn encrypt_file(src: &Path, dest: &Path, pass: &str) -> Result<u64> {
    let plain = fs::read(src)
        .map_err(|e| BukioError::new("IO_ERROR", format!("cannot read {}: {e}", src.display())))?;
    encrypt_bytes(&plain, dest, pass)
}

fn encrypt_bytes(plain: &[u8], dest: &Path, pass: &str) -> Result<u64> {
    let mut salt = vec![0u8; SALT_LEN];
    let mut iv = vec![0u8; IV_LEN];
    use ring::rand::{SecureRandom, SystemRandom};
    let rng = SystemRandom::new();
    rng.fill(&mut salt)
        .map_err(|_| BukioError::new("DB_ERROR", "rng failed"))?;
    rng.fill(&mut iv)
        .map_err(|_| BukioError::new("DB_ERROR", "rng failed"))?;
    let key = derive_key(pass, &salt)?;
    let unbound = UnboundKey::new(&AES_256_GCM, &key)
        .map_err(|e| BukioError::new("DB_ERROR", format!("key: {e}")))?;
    let key = LessSafeKey::new(unbound);
    let nonce = Nonce::assume_unique_for_key(iv.clone().try_into().unwrap());
    // ring appends the 16-byte tag after the ciphertext (matches JS layout)
    let mut in_out = plain.to_vec();
    key.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
        .map_err(|e| BukioError::new("DB_ERROR", format!("seal: {e}")))?;
    let mut out = Vec::with_capacity(BACKUP_MAGIC.len() + SALT_LEN + IV_LEN + in_out.len());
    out.extend_from_slice(BACKUP_MAGIC.as_bytes());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&iv);
    out.extend_from_slice(&in_out);
    fs::write(dest, &out)
        .map_err(|e| BukioError::new("IO_ERROR", format!("cannot write {}: {e}", dest.display())))?;
    Ok(out.len() as u64)
}

/// Decrypt an encrypted backup into a fresh Vec; wrong pass/tamper →
/// BACKUP_PASSPHRASE_WRONG.
fn decrypt_bytes(raw: &[u8], pass: &str) -> Result<Vec<u8>> {
    if raw.len() < BACKUP_MAGIC.len() || &raw[..BACKUP_MAGIC.len()] != BACKUP_MAGIC.as_bytes() {
        return Err(BukioError::new(
            "INVALID_BACKUP",
            "file is not an encrypted bukio backup",
        ));
    }
    let mut off = BACKUP_MAGIC.len();
    let salt = &raw[off..off + SALT_LEN];
    off += SALT_LEN;
    let iv: [u8; IV_LEN] = raw[off..off + IV_LEN].try_into().unwrap();
    off += IV_LEN;
    let ct = &raw[off..];
    let key = derive_key(pass, salt)?;
    let unbound = UnboundKey::new(&AES_256_GCM, &key)
        .map_err(|e| BukioError::new("DB_ERROR", format!("key: {e}")))?;
    let key = LessSafeKey::new(unbound);
    let nonce = Nonce::assume_unique_for_key(iv);
    let mut in_out = ct.to_vec();
    key.open_in_place(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| {
            BukioError::new(
                "BACKUP_PASSPHRASE_WRONG",
                "wrong passphrase or corrupted backup — decryption failed",
            )
        })?;
    Ok(in_out)
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
/// Only bukio-*.db / bukio-*.db.enc count (JS parity).
pub fn prune_backups(keep: usize, dry_run: bool) -> Result<Vec<String>> {
    let dir = backup_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }
    let re = regex::Regex::new(r"^bukio-.*\.db(\.enc)?$").unwrap();
    let mut entries: Vec<(PathBuf, std::time::SystemTime, String)> = fs::read_dir(&dir)
        .map_err(|e| BukioError::new("IO_ERROR", format!("cannot read {}: {e}", dir.display())))?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if !re.is_match(&name) {
                return None;
            }
            let mtime = e.metadata().ok()?.modified().ok()?;
            Some((e.path(), mtime, name))
        })
        .collect();
    // newest first; filename tiebreaker keeps the sort deterministic (JS parity)
    entries.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));

    let pruned: Vec<String> = entries[keep..]
        .iter()
        .map(|(_, _, name)| name.clone())
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
    encrypt: bool,
    passphrase: Option<&str>,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    if keep.is_some() && out.is_some() {
        return Err(BukioError::new(
            "INVALID_KEEP",
            "--keep rotation applies to the default backup folder only — drop --out or drop --keep",
        ));
    }
    let pass = if encrypt {
        Some(backup_passphrase(passphrase)?)
    } else {
        None
    };
    let mut dest = match out {
        Some(p) => PathBuf::from(p),
        None => default_backup_path(encrypt),
    };
    if encrypt && !dest.to_string_lossy().ends_with(".enc") {
        dest = PathBuf::from(format!("{}.enc", dest.display()));
    }
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
            "encrypted": encrypt, "pruned": pruned, "dryRun": true,
        }));
    }

    let src = Connection::open(db_path)
        .map_err(|e| BukioError::new("DB_ERROR", format!("cannot open source: {e}")))?;
    let (final_bytes, _plain_tmp): (u64, Option<u8>) = if encrypt {
        let plain = format!("{}.plain", dest.display());
        {
            let mut dest_conn = Connection::open(&plain)
                .map_err(|e| BukioError::new("DB_ERROR", format!("cannot create: {e}")))?;
            let backup = Backup::new(&src, &mut dest_conn)
                .map_err(|e| BukioError::new("DB_ERROR", format!("backup init failed: {e}")))?;
            backup
                .run_to_completion(500, std::time::Duration::ZERO, None)
                .map_err(|e| BukioError::new("DB_ERROR", format!("backup failed: {e}")))?;
        }
        let size = encrypt_file(Path::new(&plain), &dest, pass.as_deref().unwrap())?;
        fs::remove_file(&plain).ok();
        (size, None)
    } else {
        {
            let mut dest_conn = Connection::open(&dest)
                .map_err(|e| BukioError::new("DB_ERROR", format!("cannot create: {e}")))?;
            let backup = Backup::new(&src, &mut dest_conn)
                .map_err(|e| BukioError::new("DB_ERROR", format!("backup init failed: {e}")))?;
            backup
                .run_to_completion(500, std::time::Duration::ZERO, None)
                .map_err(|e| BukioError::new("DB_ERROR", format!("backup failed: {e}")))?;
        }
        let size = fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
        (size, None)
    };
    drop(src);
    // record the backup INTO the source db (the trail names who acted)
    let src_audit = Connection::open(db_path)
        .map_err(|e| BukioError::new("DB_ERROR", format!("cannot open source: {e}")))?;
    record(
        &src_audit,
        RecordArgs {
            actor,
            action: "backup",
            command: Some("backup"),
            args: Some(json!({"to": dest.display().to_string(), "bytes": final_bytes, "encrypted": encrypt})),
            outcome: "ok",
            entry_ids: vec![],
        },
    )?;
    drop(src_audit);
    // prune AFTER the new backup exists — pruning first always left N+1 files
    let pruned = if let Some(k) = keep {
        prune_backups(k, false)?
    } else {
        vec![]
    };
    Ok(
        json!({"path": dest.display().to_string(), "bytes": final_bytes, "source": db_path, "encrypted": encrypt, "pruned": pruned}),
    )
}

/// Restore from a backup file (encrypted backups auto-detected).
pub fn cmd_restore(
    from: &str,
    to: &str,
    force: bool,
    passphrase: Option<&str>,
    actor: &str,
    dry_run: bool,
) -> Result<Value> {
    let src_path = Path::new(from);
    if !src_path.exists() {
        return Err(BukioError::new(
            "FILE_NOT_FOUND",
            format!("backup file '{from}' does not exist"),
        ));
    }

    let encrypted = is_encrypted_backup(src_path);
    let pass = if encrypted {
        Some(backup_passphrase(passphrase)?)
    } else {
        None
    };
    // decrypt to a temp file so validate_backup_file sees plain SQLite
    let mut temp_path: Option<PathBuf> = None;
    let validate_src: PathBuf = if encrypted {
        let raw = fs::read(src_path)
            .map_err(|e| BukioError::new("IO_ERROR", format!("cannot read {from}: {e}")))?;
        let plain = decrypt_bytes(&raw, pass.as_deref().unwrap())?;
        let tmp = std::env::temp_dir().join(format!(
            "bukio-restore-{}-{}.db",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        fs::write(&tmp, &plain).map_err(|e| {
            BukioError::new("IO_ERROR", format!("cannot write {}: {e}", tmp.display()))
        })?;
        temp_path = Some(tmp.clone());
        tmp
    } else {
        src_path.to_path_buf()
    };

    let result = (|| {
        validate_backup_file(&validate_src)?;
        if from == to {
            return Err(BukioError::new(
                "SAME_FILE",
                format!("source and target are the same file: {to}"),
            ));
        }
        if Path::new(to).exists() && !force {
            let has_company = Connection::open(to)
                .ok()
                .and_then(|db| {
                    db.query_row(
                        "SELECT 1 FROM sqlite_master WHERE type='table' AND name='company'",
                        [],
                        |r| r.get::<_, i32>(0),
                    )
                    .ok()
                })
                .is_some();
            if has_company {
                return Err(BukioError::new(
                    "RESTORE_EXISTS",
                    format!("target '{to}' already has a company — pass --force to overwrite"),
                ));
            }
        }
        if dry_run {
            return Ok(json!({
                "action": "restore", "from": from, "to": to, "encrypted": encrypted, "dryRun": true,
            }));
        }
        fs::copy(&validate_src, to)
            .map_err(|e| BukioError::new("IO_ERROR", format!("copy failed: {e}")))?;
        let restored = Connection::open(to)
            .map_err(|e| BukioError::new("DB_ERROR", format!("open restored db: {e}")))?;
        record(
            &restored,
            RecordArgs {
                actor,
                action: "restore",
                command: Some("restore"),
                args: Some(json!({"from": from, "to": to, "encrypted": encrypted})),
                outcome: "ok",
                entry_ids: vec![],
            },
        )?;
        Ok(json!({"to": to, "from": from, "encrypted": encrypted, "restored": true}))
    })();
    if let Some(tmp) = temp_path {
        fs::remove_file(tmp).ok();
    }
    result
}
