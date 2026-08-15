/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across eleven jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Regenerate the README screenshot from scripts/screenshot-template.html.
// The template contains a verbatim capture of a real bukio demo session
// (init → opening balances → journal import → month-end → activastaat →
// SEPA payment batch → audit) rendered as a terminal window. Run: node scripts/screenshot.js
// Requires the pinned playwright-core + cached Chromium (see src/invoice/pdf.js).
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { chromium } from 'playwright-core';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const template = path.join(root, 'scripts', 'screenshot-template.html');
const out = path.join(root, 'screenshot.png');

const browser = await chromium.launch();
try {
  const page = await browser.newPage({ viewport: { width: 1120, height: 1500 }, deviceScaleFactor: 2 });
  await page.goto(`file://${template}`);
  await page.waitForTimeout(300);
  // The window can be taller than the default viewport — grow the viewport
  // to fit the whole element so the clip never cuts the bottom off.
  let box = await page.locator('.window').boundingBox();
  const needH = Math.ceil(box.height + 200);
  const needW = Math.ceil(box.width + 160);
  if (needH > 1500 || needW > 1120) {
    await page.setViewportSize({ width: needW, height: needH });
    await page.waitForTimeout(100);
    box = await page.locator('.window').boundingBox();
  }
  await page.screenshot({
    path: out,
    clip: { x: box.x, y: box.y, width: box.width, height: box.height },
  });
  console.log(`screenshot.png regenerated (${Math.round(box.width * 2)}x${Math.round(box.height * 2)}px)`);
} finally {
  await browser.close();
}
