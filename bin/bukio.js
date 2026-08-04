#!/usr/bin/env node
import { runCli } from '../src/cli/index.js';

runCli(process.argv).catch((err) => {
  console.error(`fatal: ${err.message}`);
  process.exit(1);
});
