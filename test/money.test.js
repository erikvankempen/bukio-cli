/**
 * bukio-cli — agent-first double-entry bookkeeping for SMEs across thirty-one jurisdictions.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseAmount, formatAmount } from '../src/core/money.js';

test('parseAmount: valid inputs', () => {
  assert.equal(parseAmount('0'), 0);
  assert.equal(parseAmount('0.5'), 50);
  assert.equal(parseAmount('1234'), 123400);
  assert.equal(parseAmount('1234.56'), 123456);
  assert.equal(parseAmount('-12.34'), -1234);
  assert.equal(parseAmount(' 42.10 '), 4210);
  assert.equal(parseAmount('1000000.01'), 100000001);
});

test('parseAmount: rejects invalid inputs', () => {
  for (const bad of ['', 'abc', '1.234,56', '1.234', '12.345', '1.2.3', '.5', '5.', '1,5', '--5', '1e3', 'NaN', 'Infinity', null, undefined, 1234]) {
    assert.throws(() => parseAmount(bad), { code: 'INVALID_AMOUNT' }, `should reject ${JSON.stringify(bad)}`);
  }
});

test('parseAmount: rejects more than 2 decimals', () => {
  assert.throws(() => parseAmount('1.234'), { code: 'INVALID_AMOUNT' });
  assert.throws(() => parseAmount('0.001'), { code: 'INVALID_AMOUNT' });
});

test('formatAmount: round-trips with parseAmount', () => {
  for (const cents of [0, 1, -1, 50, -50, 123456, -123456, 100000001]) {
    assert.equal(parseAmount(formatAmount(cents)), cents);
  }
});

test('formatAmount: formatting', () => {
  assert.equal(formatAmount(0), '0.00');
  assert.equal(formatAmount(123456), '1234.56');
  assert.equal(formatAmount(-1234), '-12.34');
  assert.equal(formatAmount(5), '0.05');
});
