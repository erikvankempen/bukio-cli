#!/usr/bin/env node
/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { runCli } from '../src/cli/index.js';

runCli(process.argv).catch((err) => {
  console.error(`fatal: ${err.message}`);
  process.exit(1);
});
