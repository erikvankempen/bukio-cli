// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Ed25519 key material (mirrors src/core/sign.js): keypair generation,
// signing, verification, keyid fingerprinting, passphrase-encrypted PKCS8.

use base64::Engine;
use ed25519_dalek::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use pkcs8::EncryptedPrivateKeyInfo;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// PBKDF2 iterations matching Node.js 20+ default for PKCS8 encryption.
const PBKDF2_ITERATIONS: u32 = 2048;

/// Generate an Ed25519 keypair → (spki_public_pem, pkcs8_private_pem, keyid).
pub fn generate_key_pair() -> (String, String, String) {
    let signing = SigningKey::generate(&mut OsRng);
    let public_pem = public_pem_of(&signing);
    let private_der = signing.to_pkcs8_der().expect("pkcs8");
    let private_pem = pem_encode("PRIVATE KEY", private_der.as_bytes());
    let keyid = keyid_of(&public_pem).expect("generated key is valid");
    (public_pem, private_pem, keyid)
}

/// Generate an Ed25519 keypair with a passphrase-encrypted private key (PKCS8).
/// Returns (spki_public_pem, encrypted_pkcs8_private_pem, keyid).
pub fn generate_key_pair_encrypted(
    passphrase: &str,
) -> std::result::Result<(String, String, String), String> {
    let signing = SigningKey::generate(&mut OsRng);
    let public_pem = public_pem_of(&signing);
    let private_der = signing.to_pkcs8_der().expect("pkcs8");
    let encrypted_der = encrypt_der(private_der.as_bytes(), passphrase)?;
    let private_pem = pem_encode("ENCRYPTED PRIVATE KEY", &encrypted_der);
    let keyid = keyid_of(&public_pem).expect("generated key is valid");
    Ok((public_pem, private_pem, keyid))
}

fn public_pem_of(signing: &SigningKey) -> String {
    let verifying: VerifyingKey = signing.verifying_key();
    let spki_der = verifying.to_public_key_der().expect("spki");
    pem_encode("PUBLIC KEY", spki_der.as_ref())
}

/// sha256(SPKI DER) hex, first 32 chars (mirrors keyidOf).
pub fn keyid_of(public_pem: &str) -> Result<String, String> {
    let der = pem_decode(public_pem).ok_or("not a PEM")?;
    let mut hasher = Sha256::new();
    hasher.update(&der);
    Ok(hex::encode(hasher.finalize())[..32].to_string())
}

/// Sign data (bytes) with an Ed25519 private key (PKCS8 PEM) → base64.
pub fn sign(data: &[u8], private_pem: &str) -> Result<String, String> {
    let der = pem_decode(private_pem).ok_or("not a PEM")?;
    let signing = SigningKey::from_pkcs8_der(&der).map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(signing.sign(data).to_bytes()))
}

/// Sign with passphrase support: decrypts the key in-memory if encrypted.
pub fn sign_with_passphrase(
    data: &[u8],
    private_pem: &str,
    passphrase: Option<&str>,
) -> Result<String, String> {
    if is_encrypted(private_pem) {
        let pp = passphrase.ok_or("passphrase required for encrypted key")?;
        let decrypted = decrypt_private_key_pem(private_pem, pp)?;
        sign(data, &decrypted)
    } else {
        sign(data, private_pem)
    }
}

/// Verify a base64 signature over data against an SPKI public PEM.
/// Any malformed input → false, never throws (mirrors verify()).
pub fn verify(data: &[u8], signature_b64: &str, public_pem: &str) -> bool {
    let Some(der) = pem_decode(public_pem) else {
        return false;
    };
    let Ok(verifying) = VerifyingKey::from_public_key_der(&der) else {
        return false;
    };
    let Ok(sig_bytes) = base64::engine::general_purpose::STANDARD.decode(signature_b64) else {
        return false;
    };
    let Ok(sig) = ed25519_dalek::Signature::from_slice(&sig_bytes) else {
        return false;
    };
    verifying.verify(data, &sig).is_ok()
}

pub fn is_encrypted(private_pem: &str) -> bool {
    private_pem.contains("BEGIN ENCRYPTED PRIVATE KEY")
}

/// Extract public PEM from a private PEM, with optional passphrase for encrypted keys.
pub fn public_key_from_private(
    private_pem: &str,
    passphrase: Option<&str>,
) -> std::result::Result<String, String> {
    if is_encrypted(private_pem) {
        let pp = passphrase.ok_or("encrypted keys require passphrase")?;
        let decrypted = decrypt_private_key_pem(private_pem, pp)?;
        let der = pem_decode(&decrypted).ok_or("not a valid PEM")?;
        let signing = SigningKey::from_pkcs8_der(&der).map_err(|e| e.to_string())?;
        return Ok(public_pem_of(&signing));
    }
    let der = pem_decode(private_pem).ok_or("not a valid PEM")?;
    let signing = SigningKey::from_pkcs8_der(&der).map_err(|e| e.to_string())?;
    Ok(public_pem_of(&signing))
}

/// Decrypt an encrypted PKCS8 PEM into an unencrypted PKCS8 PEM.
/// Uses PBKDF2-SHA256 + AES-256-CBC (matches Node.js crypto).
pub fn decrypt_private_key_pem(
    encrypted_pem: &str,
    passphrase: &str,
) -> std::result::Result<String, String> {
    let encrypted_der = pem_decode(encrypted_pem).ok_or("not a valid PEM")?;
    let enc_info =
        EncryptedPrivateKeyInfo::try_from(encrypted_der.as_slice()).map_err(|e| e.to_string())?;
    let secret = enc_info
        .decrypt(passphrase.as_bytes())
        .map_err(|e| format!("wrong passphrase or corrupt key: {e}"))?;
    Ok(pem_encode("PRIVATE KEY", secret.as_bytes()))
}

/// Encrypt an unencrypted PKCS8 PEM with a passphrase (PBKDF2-SHA256 + AES-256-CBC).
fn encrypt_der(plaintext_der: &[u8], passphrase: &str) -> std::result::Result<Vec<u8>, String> {
    use pkcs8::PrivateKeyInfo;

    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let mut iv = [0u8; 16];
    OsRng.fill_bytes(&mut iv);

    let params =
        pkcs8::pkcs5::pbes2::Parameters::pbkdf2_sha256_aes256cbc(PBKDF2_ITERATIONS, &salt, &iv)
            .map_err(|e| e.to_string())?;

    let pk_info = PrivateKeyInfo::try_from(plaintext_der).map_err(|e| e.to_string())?;
    let encrypted = pk_info
        .encrypt_with_params(params, passphrase)
        .map_err(|e| e.to_string())?;
    Ok(encrypted.as_bytes().to_vec())
}

// --- tiny PEM codec (std only) ---------------------------------------------

fn pem_encode(label: &str, der: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap());
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

fn pem_decode(pem: &str) -> Option<Vec<u8>> {
    let b64: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----") && !l.trim().is_empty())
        .collect();
    base64::engine::general_purpose::STANDARD.decode(b64).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_pair_signs_and_verifies() {
        let (public_pem, private_pem, keyid) = generate_key_pair();
        assert_eq!(keyid.len(), 32);
        let data = b"cross-language digest bytes";
        let sig = sign(data, &private_pem).unwrap();
        assert!(verify(data, &sig, &public_pem));
        assert!(!verify(b"tampered", &sig, &public_pem));
    }

    #[test]
    fn verifies_a_js_generated_keypair_and_signature() {
        let f = include_str!("tests/fixtures/sign_js.json");
        let f: serde_json::Value = serde_json::from_str(f).unwrap();
        let public_pem = f["publicKey"].as_str().unwrap();
        let private_pem = f["privateKey"].as_str().unwrap();
        let data = f["data"].as_str().unwrap();
        let sig = f["signature"].as_str().unwrap();
        let keyid = f["keyid"].as_str().unwrap();
        assert_eq!(keyid_of(public_pem).unwrap(), keyid);
        assert!(verify(data.as_bytes(), sig, public_pem));
        let rust_sig = sign(data.as_bytes(), private_pem).unwrap();
        assert!(verify(data.as_bytes(), &rust_sig, public_pem));
    }

    #[test]
    fn decrypt_js_encrypted_key() {
        let f = include_str!("tests/fixtures/sign_encrypted.json");
        let f: serde_json::Value = serde_json::from_str(f).unwrap();
        let public_pem = f["publicKey"].as_str().unwrap();
        let encrypted_pem = f["privateKey"].as_str().unwrap();
        let expected_keyid = f["keyid"].as_str().unwrap();

        let decrypted = decrypt_private_key_pem(encrypted_pem, "test123").unwrap();
        assert!(decrypted.contains("BEGIN PRIVATE KEY"));

        let pub_from_decrypted = public_key_from_private(&decrypted, None).unwrap();
        assert_eq!(keyid_of(&pub_from_decrypted).unwrap(), expected_keyid);
        assert_eq!(pub_from_decrypted, public_pem);

        let sig = sign(b"test data", &decrypted).unwrap();
        assert!(verify(b"test data", &sig, public_pem));
    }

    #[test]
    fn decrypt_wrong_passphrase_fails() {
        let f = include_str!("tests/fixtures/sign_encrypted.json");
        let f: serde_json::Value = serde_json::from_str(f).unwrap();
        let encrypted_pem = f["privateKey"].as_str().unwrap();
        let result = decrypt_private_key_pem(encrypted_pem, "wrong");
        assert!(result.is_err());
    }

    #[test]
    fn encrypt_and_decrypt_roundtrip() {
        let (_, private_pem, _) = generate_key_pair();
        let passphrase = "test_passphrase_123";
        let encrypted = encrypt_der(&pem_decode(&private_pem).unwrap(), passphrase).unwrap();
        let encrypted_pem = pem_encode("ENCRYPTED PRIVATE KEY", &encrypted);
        assert!(is_encrypted(&encrypted_pem));

        let decrypted = decrypt_private_key_pem(&encrypted_pem, passphrase).unwrap();
        assert_eq!(decrypted, private_pem);

        let sig1 = sign(b"hello", &private_pem).unwrap();
        let sig2 = sign(b"hello", &decrypted).unwrap();
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn sign_with_passphrase_encrypts_and_signs() {
        let (_, private_pem, _) = generate_key_pair();
        let pp = "mypass";
        let encrypted = encrypt_der(&pem_decode(&private_pem).unwrap(), pp).unwrap();
        let encrypted_pem = pem_encode("ENCRYPTED PRIVATE KEY", &encrypted);

        let sig = sign_with_passphrase(b"test", &encrypted_pem, Some(pp)).unwrap();
        let pub_pem = public_key_from_private(&private_pem, None).unwrap();
        assert!(verify(b"test", &sig, &pub_pem));
    }

    #[test]
    fn public_key_from_encrypted_private() {
        let (_, private_pem, _) = generate_key_pair();
        let pp = "test";
        let encrypted = encrypt_der(&pem_decode(&private_pem).unwrap(), pp).unwrap();
        let encrypted_pem = pem_encode("ENCRYPTED PRIVATE KEY", &encrypted);

        let pub1 = public_key_from_private(&private_pem, None).unwrap();
        let pub2 = public_key_from_private(&encrypted_pem, Some(pp)).unwrap();
        assert_eq!(pub1, pub2);
    }
}
