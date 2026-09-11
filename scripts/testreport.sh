#!/usr/bin/env bash
# bukio-cli — Rust test report.
# Copyright (c) 2026 Erik van Kempen.
# SPDX-License-Identifier: Apache-2.0
#
# Runs the Rust test suite (all targets) and writes test/report.md plus the
# README tests badge. Rust only by design: no node, no npm, nothing to install
# beyond a toolchain — this script must keep working after the JS tree is gone.
#
# Per-test descriptions are DERIVED, not invented: the suites carry no per-test
# doc comments, so each line is the test's own name de-slugged, listed under the
# suite that ran it. A name that reads badly is a naming problem to fix in the
# test itself, not here. (Section banners in the sources were tried and dropped:
# a lib test's name can collide across modules, which filed tests under another
# module's banner.)
#
# Usage: scripts/testreport.sh   (exit 1 if anything failed, 2 if nothing ran)

set -uo pipefail
cd "$(dirname "$0")/.."

report=test/report.md
# Scratch, and the suite's own temp fixtures, must live OUTSIDE the repository:
# the update tests assert that a plain directory is not a git clone, and git
# walks up out of target/tmp into the real repo, which makes such a directory
# look like a clone. /var/tmp is the safe place.
export TMPDIR="${BUKIO_TEST_TMPDIR:-/var/tmp}"
mkdir -p "$TMPDIR"

log=$(mktemp); raw=$(mktemp); suites=$(mktemp); tests=$(mktemp); index=$(mktemp)
body=$(mktemp); files=$(mktemp)
trap 'rm -f "$log" "$raw" "$suites" "$tests" "$index" "$body" "$files"' EXIT

echo "running cargo test --release — a few minutes..." >&2
# --no-fail-fast: without it cargo stops at the first failing target, which
# would hide every suite after the one that failed.
cargo test --release --no-fail-fast >"$log" 2>&1
status=$?

# ── 1. suite totals + per-test results ───────────────────────────────────────
# cargo prints "Running <target>" then "test result: ok. N passed; ..."; the
# binary target and the doc-test target have no tests, so targets that ran
# nothing are dropped instead of showing as "0 passed" rows.
awk '
  /^ *Running / { t=$0; sub(/^ *Running /,"",t); sub(/ \(.*$/,"",t); sub(/^unittests /,"",t); next }
  /^ *Doc-tests / { t="doc-tests"; next }
  /^test result:/ {
    p=f=i=0
    if (match($0, /[0-9]+ passed/))  { p=substr($0,RSTART,RLENGTH); sub(/ passed/,"",p) }
    if (match($0, /[0-9]+ failed/))  { f=substr($0,RSTART,RLENGTH); sub(/ failed/,"",f) }
    if (match($0, /[0-9]+ ignored/)) { i=substr($0,RSTART,RLENGTH); sub(/ ignored/,"",i) }
    if (p+f+i > 0) print (t==""?"?":t) "\t" p "\t" f "\t" i
    t=""
    next
  }
  /^test .+ \.\.\. / {
    n=$2; r=$NF
    sub(/^.*::/, "", n)          # lib tests are module-qualified
    if (r != "ok" && r != "FAILED" && r != "ignored") r="ok"
    print "T\t" (t==""?"?":t) "\t" n "\t" r
  }' "$log" >"$raw"

grep -v '^T	' "$raw" >"$suites" || true
grep '^T	' "$raw" | cut -f2- >"$tests" || true

passed=$(awk -F'\t' '{s+=$2} END{print s+0}' "$suites")
failed=$(awk -F'\t' '{s+=$3} END{print s+0}' "$suites")
ignored=$(awk -F'\t' '{s+=$4} END{print s+0}' "$suites")

# A blocked or failed cargo run produces no "test result:" lines at all. Refuse
# to publish a bogus report (and a red badge) that looks like a real verdict.
if [ "$((passed + failed))" -eq 0 ]; then
  echo "ERROR: cargo produced no test results — nothing to report." >&2
  grep -E "^ *error|Blocking waiting for file lock|No space left" "$log" >&2 || true
  exit 2
fi

# ── 2. test name -> de-slugged description ───────────────────────────────────
find src tests -name '*.rs' 2>/dev/null | sort >"$files"
# shellcheck disable=SC2046
awk '
  /^[[:space:]]*(pub )?(async )?fn [A-Za-z_0-9]+[[:space:]]*\(/ {
    name=$0
    sub(/^[[:space:]]*(pub )?(async )?fn /, "", name)
    sub(/[[:space:]]*\(.*/, "", name)
    desc=name
    gsub(/_/, " ", desc)
    if (!(name in d)) d[name]=desc
  }
  END { for (n in d) print n "\t" d[n] }' $(cat "$files") >"$index"

awk -F'\t' '
  NR==FNR { desc[$1]=$2; next }
  {
    target=$1; name=$2; res=$3
    d = (name in desc) ? desc[name] : name
    gsub(/_/, " ", d)
    mark = (res == "FAILED") ? "❌ " : (res == "ignored" ? "⏭ " : "")
    if (target != last) { printf "%s### `%s`\n\n", (last=="" ? "" : "\n"), target; last=target }
    printf "- %s`%s` — %s\n", mark, name, d
  }' "$index" "$tests" >"$body"

# ── 3. the report + the badge ────────────────────────────────────────────────
sha=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
size=$(stat -c%s target/release/bukio 2>/dev/null || stat -f%z target/release/bukio 2>/dev/null || echo 0)

if [ "$failed" -eq 0 ] && [ "$status" -eq 0 ]; then
  verdict="✅ ${passed} passing · 0 failing (${passed} tests)"
  badge="brightgreen"
else
  verdict="❌ ${passed} passing · ${failed} FAILING"
  badge="red"
fi
[ "$ignored" -gt 0 ] && verdict="$verdict · ⚠️ ${ignored} ignored"

{
  echo "# bukio-cli — test report"
  echo
  echo "**Latest run:** $(date -u '+%Y-%m-%d %H:%M:%S UTC') — ${verdict}"
  echo "**Command:** \`cargo test --release --no-fail-fast\` — Rust suite, all targets"
  echo "**Revision:** \`${sha}\` · **Binary:** \`target/release/bukio\`, ${size} bytes"
  echo
  echo "## Suites"
  echo
  echo "| Suite | Passed | Failed | Ignored |"
  echo "|-------|--------|--------|---------|"
  awk -F'\t' '{printf "| `%s` | %s | %s | %s |\n", $1, $2, $3, $4}' "$suites"
  echo "| **total** | **${passed}** | **${failed}** | **${ignored}** |"
  echo
  echo "## Tests"
  echo
  echo "One line per test: the test's own name, then it read back as words — the"
  echo "suites carry no per-test doc comments, so nothing here is invented."
  echo "${passed} passed, ${failed} failed, ${ignored} ignored."
  echo
  cat "$body"
  echo "Generated by \`scripts/testreport.sh\` — do not edit by hand."
} >"$report"

# Keep the README badge and this report from drifting apart.
sed -i -E "s#badge/tests-[0-9]+%20passing-[a-z]+#badge/tests-${passed}%20passing-${badge}#" README.md

echo "wrote $report (${passed} passed, ${failed} failed, ${ignored} ignored)" >&2
[ "$status" -eq 0 ] && [ "$failed" -eq 0 ]
