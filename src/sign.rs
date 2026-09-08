// bukio-cli — agent-first double-entry bookkeeping for SMEs.
// Copyright (c) 2026 Erik van Kempen.
// SPDX-License-Identifier: Apache-2.0
//
// Ed25519 key material (mirrors src/core/sign.js): keypair generation,
// signing, verification, keyid fingerprinting, passphrase-encrypted PKCS8.

use base64::Engine;
use ed25519_dalek::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};

/// Generate an Ed25519 keypair → (spki_public_pem, pkcs8_private_pem, keyid).
/// Passphrase encryption comes with the actor module port (phase 2 of the
/// port; the JS bridge keeps human-key flows until then).
pub fn generate_key_pair() -> (String, String, String) {
    let signing = SigningKey::generate(&mut OsRng);
    let public_pem = public_pem_of(&signing);
    let private_der = signing.to_pkcs8_der().expect("pkcs8");
    let private_pem = pem_encode("PRIVATE KEY", private_der.as_bytes());
    let keyid = keyid_of(&public_pem).expect("generated key is valid");
    (public_pem, private_pem, keyid)
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

/// Extract public PEM from a private PEM (unencrypted only).
pub fn public_key_from_private(
    private_pem: &str,
    _passphrase: Option<&str>,
) -> std::result::Result<String, String> {
    if is_encrypted(private_pem) {
        return Err("encrypted keys require session — run 'bukio actor unlock' first".into());
    }
    let der = pem_decode(private_pem).ok_or("not a valid PEM")?;
    let signing = SigningKey::from_pkcs8_der(&der).map_err(|e| e.to_string())?;
    Ok(public_pem_of(&signing))
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
        // fixture from src/core/sign.js: node generated the pair + signature
        let f = include_str!("tests/fixtures/sign_js.json");
        let f: serde_json::Value = serde_json::from_str(f).unwrap();
        let public_pem = f["publicKey"].as_str().unwrap();
        let private_pem = f["privateKey"].as_str().unwrap();
        let data = f["data"].as_str().unwrap();
        let sig = f["signature"].as_str().unwrap();
        let keyid = f["keyid"].as_str().unwrap();
        assert_eq!(keyid_of(public_pem).unwrap(), keyid);
        assert!(verify(data.as_bytes(), sig, public_pem));
        // Rust can sign with the JS-generated private key, verified by the
        // JS-generated public key
        let rust_sig = sign(data.as_bytes(), private_pem).unwrap();
        assert!(verify(data.as_bytes(), &rust_sig, public_pem));
    }
}
