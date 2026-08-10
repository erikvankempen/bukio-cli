/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { generateKeyPair, keyidOf, sign, verify, isEncrypted } from '../src/core/sign.js';

test('sign/verify: roundtrip with a plain key', () => {
  const { publicKey, privateKey } = generateKeyPair();
  const data = 'bukio entry add --date 2026-08-10';
  const sig = sign(data, privateKey);
  assert.equal(verify(data, sig, publicKey), true);
});

test('sign/verify: works with Buffer data too', () => {
  const { publicKey, privateKey } = generateKeyPair();
  const sig = sign(Buffer.from('hello world'), privateKey);
  assert.equal(verify(Buffer.from('hello world'), sig, publicKey), true);
});

test('sign/verify: wrong key fails', () => {
  const a = generateKeyPair();
  const b = generateKeyPair();
  const sig = sign('data', a.privateKey);
  assert.equal(verify('data', sig, b.publicKey), false);
});

test('sign/verify: tampered message fails', () => {
  const { publicKey, privateKey } = generateKeyPair();
  const sig = sign('the original command', privateKey);
  assert.equal(verify('the original commanD', sig, publicKey), false);
});

test('sign/verify: malformed signature or key does not throw, returns false', () => {
  const { publicKey } = generateKeyPair();
  assert.equal(verify('data', 'not-a-signature', publicKey), false);
  assert.equal(verify('data', '', 'not-a-public-key'), false);
});

test('keyid: stable 32-hex fingerprint of the public key', () => {
  const a = generateKeyPair();
  const b = generateKeyPair();
  assert.equal(keyidOf(a.publicKey), keyidOf(a.publicKey));
  assert.match(keyidOf(a.publicKey), /^[0-9a-f]{32}$/);
  assert.notEqual(keyidOf(a.publicKey), keyidOf(b.publicKey));
});

test('keygen: writes SPKI public and PKCS8 private PEM', () => {
  const { publicKey, privateKey } = generateKeyPair();
  assert.ok(publicKey.includes('-----BEGIN PUBLIC KEY-----'));
  assert.ok(privateKey.includes('-----BEGIN PRIVATE KEY-----'));
  assert.ok(!isEncrypted(privateKey));
});

test('keygen: passphrase-encrypted key refuses to sign without the passphrase', () => {
  const { publicKey, privateKey } = generateKeyPair({ passphrase: 'correct horse' });
  assert.ok(isEncrypted(privateKey));
  assert.ok(privateKey.includes('-----BEGIN ENCRYPTED PRIVATE KEY-----'));
  assert.throws(() => sign('data', privateKey));
});

test('keygen: passphrase-encrypted key signs with the right passphrase and verifies', () => {
  const { publicKey, privateKey } = generateKeyPair({ passphrase: 'correct horse' });
  const sig = sign('data', privateKey, { passphrase: 'correct horse' });
  assert.equal(verify('data', sig, publicKey), true);
  // wrong passphrase still fails to sign
  assert.throws(() => sign('data', privateKey, { passphrase: 'wrong' }));
});

test('keyid: fingerprint is identical for plain and passphrase keys sharing a public key', () => {
  const plain = generateKeyPair();
  const encrypted = generateKeyPair({ passphrase: 'x' });
  // different keypairs -> different fingerprints; same public key -> same fingerprint
  assert.notEqual(keyidOf(plain.publicKey), keyidOf(encrypted.publicKey));
  assert.equal(keyidOf(encrypted.publicKey), keyidOf(encrypted.publicKey));
});
