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
  const page = await browser.newPage({ viewport: { width: 1000, height: 1500 }, deviceScaleFactor: 2 });
  await page.goto(`file://${template}`);
  await page.waitForTimeout(300);
  const box = await page.locator('.window').boundingBox();
  await page.screenshot({
    path: out,
    clip: { x: box.x, y: box.y, width: box.width, height: box.height },
  });
  console.log(`screenshot.png regenerated (${Math.round(box.width * 2)}x${Math.round(box.height * 2)}px)`);
} finally {
  await browser.close();
}
