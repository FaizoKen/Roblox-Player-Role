//! AES-GCM encryption for at-rest secrets (Roblox refresh tokens, Open Cloud API keys).
//! Key derived from `TOKEN_ENCRYPTION_KEY` env var (32-byte hex).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rand::RngCore;

use crate::error::AppError;

const NONCE_LEN: usize = 12;

pub fn parse_key(hex_str: &str) -> Result<[u8; 32], AppError> {
    let bytes = hex::decode(hex_str.trim())
        .map_err(|e| AppError::Internal(format!("TOKEN_ENCRYPTION_KEY not hex: {e}")))?;
    if bytes.len() != 32 {
        return Err(AppError::Internal(format!(
            "TOKEN_ENCRYPTION_KEY must be 32 bytes (64 hex chars); got {}",
            bytes.len()
        )));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

/// Encrypt plaintext with AES-256-GCM. Returns base64(nonce || ciphertext || tag).
pub fn encrypt(key: &[u8; 32], plaintext: &str) -> Result<String, AppError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| AppError::Internal(format!("encrypt failed: {e}")))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(B64.encode(out))
}

pub fn decrypt(key: &[u8; 32], encoded: &str) -> Result<String, AppError> {
    let raw = B64
        .decode(encoded)
        .map_err(|e| AppError::Internal(format!("decrypt: bad base64: {e}")))?;
    if raw.len() < NONCE_LEN {
        return Err(AppError::Internal("decrypt: ciphertext too short".into()));
    }
    let (nonce_bytes, ciphertext) = raw.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|e| AppError::Internal(format!("decrypt failed: {e}")))?;
    String::from_utf8(plaintext)
        .map_err(|e| AppError::Internal(format!("decrypt: invalid utf8: {e}")))
}
