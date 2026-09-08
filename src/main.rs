// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Root: module tree mirrors src/ layout. The binary speaks the same
// argv/JSON protocol as bin/bukio.js; the JS shim dispatches ported
// command groups here during the transition (BUKIO_RUST_EXEC protocol,
// same shape as BUKIO_REMOTE_EXEC).

mod actor;
mod canonical;
mod db;
mod money;
mod sign;

fn main() {
    eprintln!("bukio-rust: bridge not wired yet (foundation only)");
    std::process::exit(1);
}
