//! Ported from test/update.test.js — the update command is git-driven CLI
//! surface, so these spawn the built binary against throwaway git fixtures
//! (bare origin + clones). The real repository is never touched.
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);

fn tmpdir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "bukio-upd-{}-{}-{}",
        tag,
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn sh(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// bare origin at v1 + work1 (a clone sitting at v1) while origin/main is at v2
fn fixture(v2_files: &[(&str, &str)]) -> (PathBuf, PathBuf) {
    let dir = tmpdir("fx");
    sh(&dir, &["init", "--bare", "origin.git"]);
    sh(
        &dir,
        &[
            "--git-dir=origin.git",
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ],
    );

    let origin = dir.join("origin.git");
    let work1 = dir.join("work1");
    sh(&dir, &["clone", origin.to_str().unwrap(), "work1"]);
    sh(&work1, &["config", "user.email", "test@example.com"]);
    sh(&work1, &["config", "user.name", "Test"]);
    std::fs::write(
        work1.join("package.json"),
        "{\"name\":\"bukio-cli\",\"version\":\"1.0.0\"}\n",
    )
    .unwrap();
    std::fs::write(work1.join("README.md"), "v1 content\n").unwrap();
    sh(&work1, &["add", "."]);
    sh(&work1, &["commit", "-m", "v1"]);
    sh(&work1, &["push", "-u", "origin", "main"]);

    let work2 = dir.join("work2");
    sh(&dir, &["clone", origin.to_str().unwrap(), "work2"]);
    sh(&work2, &["config", "user.email", "test@example.com"]);
    sh(&work2, &["config", "user.name", "Test"]);
    let files: Vec<(&str, &str)> = if v2_files.is_empty() {
        vec![("README.md", "v2 content\n")]
    } else {
        v2_files.to_vec()
    };
    for (f, c) in files {
        std::fs::write(work2.join(f), c).unwrap();
    }
    sh(&work2, &["add", "."]);
    sh(&work2, &["commit", "-m", "v2"]);
    sh(&work2, &["push", "origin", "main"]);
    (dir, work1)
}

/// run the built binary; returns (data-or-error document, exit ok)
fn bukio(args: &[&str]) -> (Value, bool) {
    let exe = env!("CARGO_BIN_EXE_bukio");
    let out = Command::new(exe)
        .args(args)
        // A plain temp fixture must never resolve to an enclosing repository:
        // git walks up out of the fixture directory, so a TMPDIR that sits
        // inside a checkout (CI's RUNNER_TEMP, a report run under the project)
        // would make the "not a clone" directories below look like clones of
        // this very repo — and a_non_clone_directory_is_refused would fail.
        .env("GIT_CEILING_DIRECTORIES", std::env::temp_dir())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    (
        serde_json::from_str(&stdout).unwrap_or_else(|_| json!({ "raw": stdout.to_string() })),
        out.status.success(),
    )
}

fn data(args: &[&str]) -> Value {
    let (v, ok) = bukio(args);
    assert!(ok, "expected success, got {v}");
    v["data"].clone()
}

fn err_code(args: &[&str]) -> String {
    let (v, ok) = bukio(args);
    assert!(!ok, "expected failure, got {v}");
    v["error"]["code"].as_str().unwrap_or_default().to_string()
}

const ACTOR: &[&str] = &["--actor", "human:erik"];

#[test]
fn a_non_clone_directory_is_refused() {
    let dir = tmpdir("plain");
    assert_eq!(
        err_code(&[
            "update",
            "--repo",
            dir.to_str().unwrap(),
            "--dry-run",
            "--json"
        ]),
        "UPDATE_NOT_A_CLONE"
    );
}

#[test]
fn a_non_official_remote_is_refused() {
    let (_dir, work1) = fixture(&[]);
    sh(
        &work1,
        &[
            "remote",
            "set-url",
            "origin",
            "https://github.com/someone-else/bukio-cli.git",
        ],
    );
    assert_eq!(
        err_code(&[
            "update",
            "--repo",
            work1.to_str().unwrap(),
            "--dry-run",
            "--json"
        ]),
        "UPDATE_WRONG_REMOTE"
    );
}

#[test]
fn a_url_embedding_the_official_path_as_substring_is_refused() {
    let (_dir, work1) = fixture(&[]);
    sh(
        &work1,
        &[
            "remote",
            "set-url",
            "origin",
            "https://evil.com/github.com:erikvankempen/bukio-cli.git",
        ],
    );
    assert_eq!(
        err_code(&[
            "update",
            "--repo",
            work1.to_str().unwrap(),
            "--dry-run",
            "--json"
        ]),
        "UPDATE_WRONG_REMOTE"
    );
}

#[test]
fn the_plan_shows_the_incoming_commit_and_current_version() {
    let (_dir, work1) = fixture(&[]);
    let p = data(&[
        "update",
        "--repo",
        work1.to_str().unwrap(),
        "--dry-run",
        "--trust-remote",
        "--json",
    ]);
    assert_eq!(p["current_version"].as_str(), Some("1.0.0"));
    assert_eq!(p["incoming_count"].as_i64(), Some(1));
    assert!(
        p["incoming"][0].as_str().unwrap_or_default().contains("v2"),
        "{p}"
    );
    assert_eq!(p["local_commits"].as_array().unwrap().len(), 0);
    assert_eq!(p["modified_files"].as_array().unwrap().len(), 0);
    assert!(p["warning"].is_null());
    assert_eq!(p["up_to_date"], json!(false));
}

#[test]
fn local_modifications_are_reported_as_overwrite_warnings() {
    let (_dir, work1) = fixture(&[]);
    std::fs::write(work1.join("README.md"), "my customization\n").unwrap();
    let p = data(&[
        "update",
        "--repo",
        work1.to_str().unwrap(),
        "--dry-run",
        "--trust-remote",
        "--json",
    ]);
    assert_eq!(p["modified_files"], json!(["README.md"]));
    let w = p["warning"].as_str().unwrap_or_default();
    assert!(w.contains("OVERWRITES LOCAL CUSTOMIZATIONS"), "{w}");
    assert!(w.contains("1 modified file"), "{w}");
}

#[test]
fn update_refuses_to_run_without_yes() {
    let (_dir, work1) = fixture(&[]);
    assert_eq!(
        err_code(&[
            "update",
            "--repo",
            work1.to_str().unwrap(),
            "--trust-remote",
            "--json",
        ]),
        "UPDATE_CONFIRM_REQUIRED"
    );
}

#[test]
fn yes_resets_the_working_tree_to_origin_main() {
    let (_dir, work1) = fixture(&[]);
    let r = data(&[
        "update",
        "--repo",
        work1.to_str().unwrap(),
        "--yes",
        "--trust-remote",
        "--json",
    ]);
    assert_eq!(r["updated"], json!(true));
    assert_eq!(r["commits_applied"].as_i64(), Some(1));
    assert_eq!(r["version_after"].as_str(), Some("1.0.0")); // package.json unchanged in v2
    assert_eq!(
        sh(&work1, &["rev-parse", "HEAD"]),
        sh(&work1, &["rev-parse", "origin/main"])
    );
    assert_eq!(
        std::fs::read_to_string(work1.join("README.md")).unwrap(),
        "v2 content\n"
    );

    let p = data(&[
        "update",
        "--repo",
        work1.to_str().unwrap(),
        "--dry-run",
        "--trust-remote",
        "--json",
    ]);
    assert_eq!(p["up_to_date"], json!(true));
}

#[test]
fn yes_overwrites_a_local_customization() {
    let (_dir, work1) = fixture(&[]);
    std::fs::write(work1.join("README.md"), "my customization\n").unwrap();
    let p = data(&[
        "update",
        "--repo",
        work1.to_str().unwrap(),
        "--dry-run",
        "--trust-remote",
        "--json",
    ]);
    assert_eq!(p["modified_files"].as_array().unwrap().len(), 1);
    let r = data(&[
        "update",
        "--repo",
        work1.to_str().unwrap(),
        "--yes",
        "--trust-remote",
        "--json",
    ]);
    assert_eq!(r["updated"], json!(true));
    assert_eq!(
        std::fs::read_to_string(work1.join("README.md")).unwrap(),
        "v2 content\n"
    );
}

#[test]
fn yes_drops_local_commits() {
    let (_dir, work1) = fixture(&[]);
    std::fs::write(work1.join("README.md"), "local commit content\n").unwrap();
    sh(&work1, &["add", "."]);
    sh(&work1, &["commit", "-m", "local change"]);
    let p = data(&[
        "update",
        "--repo",
        work1.to_str().unwrap(),
        "--dry-run",
        "--trust-remote",
        "--json",
    ]);
    assert_eq!(p["local_commits"].as_array().unwrap().len(), 1);
    assert_eq!(p["incoming_count"].as_i64(), Some(1));
    let r = data(&[
        "update",
        "--repo",
        work1.to_str().unwrap(),
        "--yes",
        "--trust-remote",
        "--json",
    ]);
    assert_eq!(r["updated"], json!(true));
    assert_eq!(
        std::fs::read_to_string(work1.join("README.md")).unwrap(),
        "v2 content\n"
    );
    assert_eq!(
        sh(&work1, &["rev-parse", "HEAD"]),
        sh(&work1, &["rev-parse", "origin/main"])
    );
}

#[test]
fn reinstalls_dependencies_when_package_json_changed() {
    let (_dir, work1) = fixture(&[(
        "package.json",
        "{\"name\":\"bukio-cli\",\"version\":\"1.0.1\"}\n",
    )]);
    let p = data(&[
        "update",
        "--repo",
        work1.to_str().unwrap(),
        "--dry-run",
        "--trust-remote",
        "--json",
    ]);
    assert_eq!(p["package_json_changed"], json!(true));
    let r = data(&[
        "update",
        "--repo",
        work1.to_str().unwrap(),
        "--yes",
        "--trust-remote",
        "--json",
    ]);
    assert_eq!(r["version_after"].as_str(), Some("1.0.1"));
    assert_eq!(r["deps_installed"], json!(true));
    assert!(r["deps_error"].is_null(), "{r}");
}

#[test]
fn records_an_audit_row_when_a_company_db_exists() {
    let (_dir, work1) = fixture(&[]);
    let dbdir = tmpdir("db");
    let db = dbdir.join("c.db");
    let _ = bukio(&[
        "init",
        "--name",
        "Demo BV",
        "--db",
        db.to_str().unwrap(),
        "--json",
        "--actor",
        "human:erik",
    ]);
    let args: Vec<&str> = vec![
        "update",
        "--repo",
        work1.to_str().unwrap(),
        "--yes",
        "--trust-remote",
        "--json",
        "--db",
        db.to_str().unwrap(),
    ];
    let mut a = args.clone();
    a.extend_from_slice(ACTOR);
    let r = data(&a);

    let conn = rusqlite::Connection::open(&db).unwrap();
    let (action, actor, args_json): (String, String, String) = conn
        .query_row(
            "SELECT action, actor, args_json FROM audit_log WHERE action = 'update'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(action, "update");
    assert_eq!(actor, "human:erik");
    let parsed: Value = serde_json::from_str(&args_json).unwrap();
    assert_eq!(parsed["commits"].as_i64(), r["commits_applied"].as_i64());
}
