use anyhow::{Context, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::store::Conversation;

const PREFIX: &str = "enc:v1:";
const BYTE_MAGIC: &[u8] = b"ENC1";
const SALT: &[u8] = b"conversation-handoff.v1";

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Crypto {
    key: [u8; 32],
}

impl Crypto {
    pub fn new(secret: &str) -> Result<Self> {
        let secret = secret.trim();
        if secret.is_empty() {
            anyhow::bail!("encryption_key is empty");
        }
        Ok(Self {
            key: derive_key(secret)?,
        })
    }

    pub fn encrypt_text(&self, plain: &str) -> Result<String> {
        if plain.is_empty() || plain.starts_with(PREFIX) {
            return Ok(plain.to_string());
        }
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, plain.as_bytes())
            .map_err(|e| anyhow::anyhow!("encrypt failed: {e}"))?;
        let mut packed = Vec::with_capacity(nonce.len() + ciphertext.len());
        packed.extend_from_slice(&nonce);
        packed.extend_from_slice(&ciphertext);
        Ok(format!("{PREFIX}{}", STANDARD.encode(packed)))
    }

    pub fn decrypt_text(&self, value: &str) -> Result<String> {
        let Some(rest) = value.strip_prefix(PREFIX) else {
            return Ok(value.to_string());
        };
        let packed = STANDARD.decode(rest).context("decode encrypted value")?;
        if packed.len() < 12 {
            anyhow::bail!("encrypted value is truncated");
        }
        let nonce = Nonce::from_slice(&packed[..12]);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let plain = cipher
            .decrypt(nonce, &packed[12..])
            .map_err(|_| anyhow::anyhow!("failed to decrypt; check store.encryption_key"))?;
        String::from_utf8(plain).context("decrypted text is not utf-8")
    }

    pub fn encrypt_bytes(&self, plain: &[u8]) -> Result<Vec<u8>> {
        if plain.is_empty() || plain.starts_with(BYTE_MAGIC) {
            return Ok(plain.to_vec());
        }
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, plain)
            .map_err(|e| anyhow::anyhow!("encrypt bytes failed: {e}"))?;
        let mut packed = Vec::with_capacity(BYTE_MAGIC.len() + nonce.len() + ciphertext.len());
        packed.extend_from_slice(BYTE_MAGIC);
        packed.extend_from_slice(&nonce);
        packed.extend_from_slice(&ciphertext);
        Ok(packed)
    }

    pub fn decrypt_bytes(&self, value: &[u8]) -> Result<Vec<u8>> {
        let Some(rest) = value.strip_prefix(BYTE_MAGIC) else {
            return Ok(value.to_vec());
        };
        if rest.len() < 12 {
            anyhow::bail!("encrypted image is truncated");
        }
        let nonce = Nonce::from_slice(&rest[..12]);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        cipher
            .decrypt(nonce, &rest[12..])
            .map_err(|_| anyhow::anyhow!("failed to decrypt image; check store.encryption_key"))
    }

    pub fn encrypt_conv(&self, conv: &Conversation) -> Result<Conversation> {
        Ok(Conversation {
            id: conv.id.clone(),
            parent_id: conv.parent_id.clone(),
            title: map_opt(&conv.title, |s| self.encrypt_text(s))?,
            created_at: conv.created_at,
            latest_message: map_opt(&conv.latest_message, |s| self.encrypt_text(s))?,
            brief: map_opt(&conv.brief, |s| self.encrypt_text(s))?,
            chunks: conv
                .chunks
                .iter()
                .map(|c| self.encrypt_text(c))
                .collect::<Result<Vec<_>>>()?,
            summary: map_opt(&conv.summary, |s| self.encrypt_text(s))?,
            status: conv.status,
            updated_at: conv.updated_at,
            last_saved_at: conv.last_saved_at,
            pruned_at: conv.pruned_at,
            chunk_count: conv.chunk_count,
            images: Vec::new(),
        })
    }

    pub fn decrypt_conv(&self, conv: Conversation) -> Result<Conversation> {
        Ok(Conversation {
            title: map_opt(&conv.title, |s| self.decrypt_text(s))?,
            latest_message: map_opt(&conv.latest_message, |s| self.decrypt_text(s))?,
            brief: map_opt(&conv.brief, |s| self.decrypt_text(s))?,
            chunks: conv
                .chunks
                .iter()
                .map(|c| self.decrypt_text(c))
                .collect::<Result<Vec<_>>>()?,
            summary: map_opt(&conv.summary, |s| self.decrypt_text(s))?,
            images: Vec::new(),
            ..conv
        })
    }
}

fn map_opt(value: &Option<String>, f: impl Fn(&str) -> Result<String>) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(s) => Ok(Some(f(s)?)),
    }
}

fn derive_key(secret: &str) -> Result<[u8; 32]> {
    if let Some(raw) = parse_hex32(secret) {
        return Ok(raw);
    }
    let params =
        Params::new(32_768, 2, 1, Some(32)).map_err(|e| anyhow::anyhow!("argon2 params: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon
        .hash_password_into(secret.as_bytes(), SALT, &mut key)
        .map_err(|e| anyhow::anyhow!("derive encryption key: {e}"))?;
    Ok(key)
}

fn parse_hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let crypto = Crypto::new("test-secret-please-change").unwrap();
        let enc = crypto.encrypt_text("hello jwt secret").unwrap();
        assert!(enc.starts_with(PREFIX));
        assert_ne!(enc, "hello jwt secret");
        assert_eq!(crypto.decrypt_text(&enc).unwrap(), "hello jwt secret");
    }

    #[test]
    fn wrong_key_fails() {
        let a = Crypto::new("key-a").unwrap();
        let b = Crypto::new("key-b").unwrap();
        let enc = a.encrypt_text("secret notes").unwrap();
        let err = b.decrypt_text(&enc).unwrap_err().to_string();
        assert!(err.contains("encryption_key"));
    }

    #[test]
    fn plaintext_passthrough() {
        let crypto = Crypto::new("key").unwrap();
        assert_eq!(crypto.decrypt_text("plain").unwrap(), "plain");
    }

    #[test]
    fn bytes_round_trip() {
        let crypto = Crypto::new("test-secret-please-change").unwrap();
        let plain = b"\x89PNG secret bytes";
        let enc = crypto.encrypt_bytes(plain).unwrap();
        assert!(enc.starts_with(BYTE_MAGIC));
        assert_ne!(enc, plain);
        assert_eq!(crypto.decrypt_bytes(&enc).unwrap(), plain);
    }
}
