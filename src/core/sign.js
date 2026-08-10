/**
 * bukio-cli — agent-first double-entry bookkeeping for Dutch SMEs.
 * Copyright (c) 2026 Erik van Kempen.
 * SPDX-License-Identifier: Apache-2.0
 */

// Ed25519 key material for signed actor commands (Tier 0 of the actor
// authentication design): keypair generation (plain or passphrase-encrypted
// PKCS8), signing, verification and keyid fingerprinting. node:crypto only —
// no new dependencies (RFC 8032).
import crypto from 'node:crypto';

// Keyid = first 16 bytes (32 hex chars) of sha256(SPKI DER).
const KEYID_HEX_LENGTH = 32;

/**
 * Generate an Ed25519 keypair.
 *
 * @param {object} [opts]
 * @param {string} [opts.passphrase] - if given, the private key is written
 *   as passphrase-encrypted PKCS8 (aes-256-cbc) and cannot be used without it.
 * @returns {{publicKey: string, privateKey: string, keyid: string}} PEM
 *   strings (SPKI public, PKCS8 private) plus the keyid fingerprint.
 */
export function generateKeyPair({ passphrase } = {}) {
  const privateKeyEncoding = passphrase
    ? { type: 'pkcs8', format: 'pem', cipher: 'aes-256-cbc', passphrase }
    : { type: 'pkcs8', format: 'pem' };
  const { publicKey, privateKey } = crypto.generateKeyPairSync('ed25519', {
    publicKeyEncoding: { type: 'spki', format: 'pem' },
    privateKeyEncoding,
  });
  return { publicKey, privateKey, keyid: keyidOf(publicKey) };
}

/**
 * Stable fingerprint of a public key: sha256 of the SPKI DER bytes, hex,
 * truncated to the first 16 bytes (32 hex chars).
 *
 * @param {string} publicKeyPem - SPKI PEM.
 * @returns {string} 32-hex-char keyid.
 */
export function keyidOf(publicKeyPem) {
  const der = crypto.createPublicKey(publicKeyPem).export({ type: 'spki', format: 'der' });
  return crypto.createHash('sha256').update(der).digest('hex').slice(0, KEYID_HEX_LENGTH);
}

/**
 * Sign data (string or Buffer) with an Ed25519 private key.
 *
 * @param {string|Buffer|Uint8Array} data - the bytes to sign.
 * @param {string} privateKeyPem - PKCS8 PEM. May be passphrase-encrypted.
 * @param {object} [opts]
 * @param {string} [opts.passphrase] - required to use an encrypted key;
 *   a wrong passphrase throws.
 * @returns {string} base64 signature.
 */
export function sign(data, privateKeyPem, { passphrase } = {}) {
  const key = passphrase ? { key: privateKeyPem, passphrase } : privateKeyPem;
  const signature = crypto.sign(null, toBuffer(data), key);
  return signature.toString('base64');
}

/**
 * Verify a base64 signature over data against a public key. Any malformed
 * input (bad signature, bad key) verifies as false — never throws.
 *
 * @param {string|Buffer|Uint8Array} data - the bytes that were signed.
 * @param {string} signatureBase64 - base64 signature.
 * @param {string} publicKeyPem - SPKI PEM.
 * @returns {boolean} true only if the signature verifies against the key.
 */
export function verify(data, signatureBase64, publicKeyPem) {
  try {
    return crypto.verify(null, toBuffer(data), publicKeyPem, Buffer.from(signatureBase64, 'base64'));
  } catch {
    return false;
  }
}

/**
 * @param {string} privateKeyPem - PKCS8 PEM.
 * @returns {boolean} true if the key is passphrase-encrypted.
 */
export function isEncrypted(privateKeyPem) {
  return privateKeyPem.includes('BEGIN ENCRYPTED PRIVATE KEY');
}

function toBuffer(data) {
  if (Buffer.isBuffer(data)) return data;
  if (data instanceof Uint8Array) return Buffer.from(data);
  return Buffer.from(String(data), 'utf8');
}
