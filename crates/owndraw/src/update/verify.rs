//! minisign-Signaturprüfung (Ed25519). Ohne gültige Signatur wird nichts installiert.

use super::Error;
use minisign_verify::{PublicKey, Signature};
use std::path::Path;

/// Öffentlicher Schlüssel der Releases (minisign, Schlüssel-ID FE8B7DCAA4C21501).
pub const PUBKEY: &str = include_str!("minisign.pub");

/// Ob der Text der Platzhalter ist (dann sind Updates „nicht eingerichtet“).
pub fn is_placeholder(pubkey: &str) -> bool {
    let up = pubkey.to_uppercase();
    up.contains("PLATZHALTER") || up.contains("PLACEHOLDER") || PublicKey::decode(pubkey).is_err()
}

/// Prüft `data` gegen den Inhalt einer `.minisig`-Datei.
pub fn verify_bytes(pubkey: &str, data: &[u8], sig_text: &str) -> Result<(), Error> {
    if is_placeholder(pubkey) {
        return Err(Error::NotConfigured);
    }
    let pk = PublicKey::decode(pubkey).map_err(|e| Error::Other(format!("Öffentlicher Schlüssel unlesbar: {e}")))?;
    let sig = Signature::decode(sig_text).map_err(|e| Error::BadSignature(format!("Signatur unlesbar: {e}")))?;
    pk.verify(data, &sig, true).map_err(|e| Error::BadSignature(e.to_string()))
}

/// Prüft eine Datei gegen ihre Signatur (die Datei wird dafür vollständig gelesen).
pub fn verify_file(pubkey: &str, file: &Path, sig_text: &str) -> Result<(), Error> {
    if is_placeholder(pubkey) {
        return Err(Error::NotConfigured);
    }
    let data = std::fs::read(file)?;
    verify_bytes(pubkey, &data, sig_text)
}

#[cfg(test)]
mod tests {
    use super::*;
    const PLACEHOLDER: &str = include_str!("placeholder.pub");

    #[test]
    fn platzhalter_wird_erkannt() {
        assert!(is_placeholder(PLACEHOLDER));
        assert!(matches!(verify_bytes(PLACEHOLDER, b"x", "y"), Err(Error::NotConfigured)));
        assert!(!is_placeholder(PUBKEY), "eingebauter Release-Schlüssel muss gültig sein");
    }
}
