// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Library crate: module declarations for the engine. The binary entry point
// (src/bin/bukio.rs) imports these via `use bukio::*`.

pub mod accounts;
pub mod actor;
pub mod actor_cli;
pub mod assets;
pub mod attachments;
pub mod audit;
pub mod authz;
pub mod backup;
pub mod bank;
pub mod canonical;
pub mod company;
pub mod compliance;
pub mod contacts;
pub mod dates;
pub mod db;
pub mod entries;
pub mod export;
pub mod fx;
pub mod i18n;
pub mod import_mod;
pub mod invoice;
pub mod items;
pub mod mcp;
pub mod money;
pub mod month_end;
pub mod payments;
pub mod peppol;
pub mod recurring;
pub mod report_pdf;
pub mod reports;
pub mod server;
pub mod sign;
pub mod sign_gate;
pub mod smtp;
pub mod ubl;
pub mod vat;
pub mod year_end;
