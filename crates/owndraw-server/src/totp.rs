//! Zwei-Faktor-Anmeldung für Admins: TOTP (RFC 6238, SHA-1, 6 Stellen, 30 s), einmalige
//! Wiederherstellungscodes und das verschlüsselte Ablegen des Geheimnisses.
//!
//! Admin-Rechte gibt es nur in einer Sitzung, die mit Passwort **und** Code bestätigt ist
//! (`sessions.mfa = 1`, geprüft in [`crate::auth::AdminAuth`]). Routen: `/api/auth/2fa/*`
//! (Einrichten, Code bestätigen) und `/api/admin/2fa/recovery-codes` (neue Wiederherstellungscodes).

use crate::auth::{hash_token, Auth, ClientIp, J};
use crate::db::{self, now};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hmac::{Hmac, Mac};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

pub const STEP: i64 = 30;
pub const DIGITS: u32 = 6;
/// Erlaubte Abweichung in Zeitschritten (±1 = ±30 s).
pub const SKEW: i64 = 1;
pub const RECOVERY_COUNT: usize = 10;
/// Falsche Codes je Konto, bevor es für [`LOCK_SECS`] gesperrt wird.
pub const MAX_FAILS: u32 = 5;
pub const LOCK_SECS: u64 = 15 * 60;
const ISSUER: &str = "OwnDraw";
const KEY_FILE: &str = "secret.key";

// ---------- TOTP ----------

/// Code zum Zeitschritt `step` (RFC 4226/6238).
pub fn code_at_step(secret: &[u8], step: i64) -> u32 {
    let mut mac = <Hmac<sha1::Sha1> as Mac>::new_from_slice(secret).expect("HMAC nimmt jede Schlüssellänge");
    mac.update(&(step as u64).to_be_bytes());
    let h = mac.finalize().into_bytes();
    let off = (h[19] & 0x0f) as usize;
    let bin = u32::from_be_bytes([h[off] & 0x7f, h[off + 1], h[off + 2], h[off + 3]]);
    bin % 10u32.pow(DIGITS)
}

/// Aktueller Code als Text (für Tests und Werkzeuge), Geheimnis in Base32.
pub fn code_now(secret_b32: &str) -> String {
    let secret = decode_b32(secret_b32).unwrap_or_default();
    format!("{:06}", code_at_step(&secret, now() / STEP))
}

/// Prüft `code` gegen die Schritte um `t`; liefert den passenden Schritt, wenn er neuer als `last` ist.
pub fn check(secret: &[u8], code: &str, t: i64, last: i64) -> Option<i64> {
    let n: u32 = code.parse().ok()?;
    let cur = t / STEP;
    (cur - SKEW..=cur + SKEW).find(|s| *s > last && code_at_step(secret, *s) == n)
}

pub fn encode_b32(b: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(b)
}

pub fn decode_b32(s: &str) -> Option<Vec<u8>> {
    let s: String = s.chars().filter(|c| !c.is_whitespace() && *c != '=').collect::<String>().to_uppercase();
    data_encoding::BASE32_NOPAD.decode(s.as_bytes()).ok()
}

fn otpauth_uri(email: &str, secret_b32: &str) -> String {
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
    let label = utf8_percent_encode(&format!("{ISSUER}:{email}"), NON_ALPHANUMERIC).to_string();
    format!("otpauth://totp/{label}?secret={secret_b32}&issuer={ISSUER}&algorithm=SHA1&digits={DIGITS}&period={STEP}")
}

fn qr_svg(text: &str) -> ApiResult<String> {
    let qr = qrcode::QrCode::new(text.as_bytes()).map_err(ApiError::internal)?;
    Ok(qr.render::<qrcode::render::svg::Color>().min_dimensions(200, 200).quiet_zone(true).build())
}

// ---------- Verschlüsselung des Geheimnisses ----------

/// Schlüssel aus `<data>/secret.key` (32 Byte, Rechte 0600); wird beim ersten Mal angelegt.
fn data_key(data: &Path) -> ApiResult<[u8; 32]> {
    let path = data.join(KEY_FILE);
    match std::fs::read(&path) {
        Ok(b) if b.len() == 32 => return Ok(b.try_into().expect("32 Byte")),
        Ok(_) => return Err(ApiError::internal(format!("{} ist beschädigt", path.display()))),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(ApiError::internal(e)),
        Err(_) => {}
    }
    std::fs::create_dir_all(data).map_err(ApiError::internal)?;
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(ApiError::internal)?;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    match opts.open(&path) {
        Ok(mut f) => {
            f.write_all(&key).and_then(|_| f.sync_all()).map_err(ApiError::internal)?;
            Ok(key)
        }
        // Gleichzeitig angelegt: die vorhandene Datei gilt.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => data_key(data),
        Err(e) => Err(ApiError::internal(e)),
    }
}

fn seal(data: &Path, user_id: &str, secret: &[u8]) -> ApiResult<String> {
    let key = data_key(data)?;
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(ApiError::internal)?;
    let ct = ChaCha20Poly1305::new(Key::from_slice(&key))
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: secret, aad: user_id.as_bytes() })
        .map_err(|_| ApiError::internal("Verschlüsseln fehlgeschlagen"))?;
    Ok(format!("v1:{}{}", hex::encode(nonce), hex::encode(ct)))
}

fn open(data: &Path, user_id: &str, sealed: &str) -> ApiResult<Vec<u8>> {
    let raw = sealed.strip_prefix("v1:").and_then(|h| hex::decode(h).ok()).filter(|b| b.len() > 12).ok_or_else(|| ApiError::internal("2FA-Geheimnis unlesbar"))?;
    let key = data_key(data)?;
    ChaCha20Poly1305::new(Key::from_slice(&key))
        .decrypt(Nonce::from_slice(&raw[..12]), Payload { msg: &raw[12..], aad: user_id.as_bytes() })
        .map_err(|_| ApiError::internal("2FA-Geheimnis passt nicht zu data/secret.key"))
}

// ---------- Datenbank ----------

/// 2FA-Zustand eines Kontos.
pub struct TotpRow {
    pub secret: Option<String>,
    pub pending: Option<String>,
    pub last_step: i64,
}

impl TotpRow {
    pub fn enabled(&self) -> bool {
        self.secret.is_some()
    }
}

pub fn row(c: &Connection, user_id: &str) -> rusqlite::Result<TotpRow> {
    c.query_row("SELECT totp_secret, totp_pending, totp_last_step FROM users WHERE id=?1", [user_id], |r| {
        Ok(TotpRow { secret: r.get(0)?, pending: r.get(1)?, last_step: r.get(2)? })
    })
    .optional()
    .map(|o| o.unwrap_or(TotpRow { secret: None, pending: None, last_step: 0 }))
}

/// Ist die Sitzung mit beiden Faktoren bestätigt?
pub fn session_mfa(c: &Connection, token_hash: &str) -> rusqlite::Result<bool> {
    Ok(c.query_row("SELECT mfa FROM sessions WHERE token_hash=?1", [token_hash], |r| r.get::<_, i64>(0)).optional()?.unwrap_or(0) != 0)
}

fn set_session_mfa(c: &Connection, token_hash: &str) -> rusqlite::Result<()> {
    c.execute("UPDATE sessions SET mfa=1 WHERE token_hash=?1", [token_hash])?;
    Ok(())
}

/// Normalisiert einen Wiederherstellungscode (Großbuchstaben, ohne Leer- und Bindestriche).
fn norm_recovery(code: &str) -> String {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_uppercase()
}

/// Erzeugt neue Wiederherstellungscodes (alte werden ungültig); gespeichert wird nur der SHA-256-Hash.
fn new_recovery_codes(c: &Connection, user_id: &str) -> rusqlite::Result<Vec<String>> {
    c.execute("DELETE FROM recovery_codes WHERE user_id=?1", [user_id])?;
    let mut out = Vec::with_capacity(RECOVERY_COUNT);
    for _ in 0..RECOVERY_COUNT {
        let raw: String = (0..10).map(|_| db::random_code_char()).collect();
        c.execute("INSERT INTO recovery_codes(user_id,code_hash,used_at) VALUES(?1,?2,NULL)", params![user_id, hash_token(&raw)])?;
        out.push(format!("{}-{}", &raw[..5], &raw[5..]));
    }
    Ok(out)
}

fn recovery_left(c: &Connection, user_id: &str) -> rusqlite::Result<i64> {
    c.query_row("SELECT COUNT(*) FROM recovery_codes WHERE user_id=?1 AND used_at IS NULL", [user_id], |r| r.get(0))
}

/// Löst einen Wiederherstellungscode ein (nur einmal gültig).
fn use_recovery(c: &Connection, user_id: &str, code: &str) -> rusqlite::Result<bool> {
    let n = c.execute(
        "UPDATE recovery_codes SET used_at=?3 WHERE user_id=?1 AND code_hash=?2 AND used_at IS NULL",
        params![user_id, hash_token(&norm_recovery(code)), now()],
    )?;
    Ok(n == 1)
}

/// Notfall-Reset (Kommandozeile): 2FA, Wiederherstellungscodes und alle Sitzungen des Kontos löschen.
/// Liefert `false`, wenn es kein Admin-Konto mit dieser Adresse gibt.
pub async fn reset(db: &db::Db, email: &str) -> ApiResult<bool> {
    let email = email.trim().to_lowercase();
    db.call(move |c| {
        let Some(u) = db::user_by_email(c, &email)?.filter(|u| u.admin) else { return Ok(false) };
        c.execute("UPDATE users SET totp_secret=NULL, totp_pending=NULL, totp_last_step=0 WHERE id=?1", [&u.id])?;
        c.execute("DELETE FROM recovery_codes WHERE user_id=?1", [&u.id])?;
        c.execute("DELETE FROM sessions WHERE user_id=?1", [&u.id])?;
        Ok(true)
    })
    .await
}

/// `mfa`-Feld für `/api/me` und `/api/auth/login` (nur bei Admins).
pub fn me_mfa(c: &Connection, user_id: &str, token_hash: Option<&str>) -> rusqlite::Result<Value> {
    let r = row(c, user_id)?;
    let verified = match token_hash {
        Some(th) => session_mfa(c, th)?,
        None => false,
    };
    Ok(json!({"enabled": r.enabled(), "verified": verified && r.enabled(), "recovery_left": recovery_left(c, user_id)?}))
}

// ---------- Sperre bei falschen Codes ----------

fn fail_key(user_id: &str) -> String {
    format!("2fa-fail:{user_id}")
}

fn ensure_not_locked(s: &AppState, user_id: &str) -> ApiResult<()> {
    if s.limiter.count(&fail_key(user_id), Duration::from_secs(LOCK_SECS)) >= MAX_FAILS {
        return Err(ApiError::rate_limited_with("Zu viele falsche Codes. Die Anmeldung ist für 15 Minuten gesperrt."));
    }
    Ok(())
}

fn note_fail(s: &AppState, user_id: &str) -> ApiError {
    s.limiter.hit(&fail_key(user_id), u32::MAX, Duration::from_secs(LOCK_SECS));
    ApiError::unauthorized_with("Der Code ist falsch oder abgelaufen.")
}

// ---------- Routen ----------

/// Angemeldeter Admin, bei dem der zweite Faktor noch fehlen darf (nur für Einrichten/Bestätigen).
fn require_admin(a: &Auth) -> ApiResult<()> {
    if !a.user.admin {
        return Err(ApiError::forbidden("Nur für Administratoren."));
    }
    Ok(())
}

/// `POST /api/auth/2fa/setup`: neues Geheimnis (noch nicht aktiv) mit QR-Code.
pub async fn setup(State(s): State<AppState>, ClientIp(ip): ClientIp, a: Auth) -> ApiResult<Json<Value>> {
    require_admin(&a)?;
    crate::auth::rate_check(&s, "2fa", &ip)?;
    let mut secret = [0u8; 20];
    getrandom::fill(&mut secret).map_err(ApiError::internal)?;
    let sealed = seal(&s.cfg.data, &a.user.id, &secret)?;
    let uid = a.user.id.clone();
    s.db.call(move |c| {
        if row(c, &uid)?.enabled() {
            return Err(ApiError::conflict("Die Zwei-Faktor-Anmeldung ist bereits eingerichtet."));
        }
        c.execute("UPDATE users SET totp_pending=?2 WHERE id=?1", params![uid, sealed])?;
        Ok(())
    })
    .await?;
    let b32 = encode_b32(&secret);
    let uri = otpauth_uri(&a.user.email, &b32);
    let svg = qr_svg(&uri)?;
    Ok(Json(json!({"secret": b32, "uri": uri, "qr_svg": svg})))
}

#[derive(Deserialize)]
pub struct CodeBody {
    code: String,
}

/// `POST /api/auth/2fa/enable`: Einrichtung mit einem Code bestätigen. Aktiviert 2FA, bestätigt die
/// Sitzung und liefert die Wiederherstellungscodes (einmalig).
pub async fn enable(State(s): State<AppState>, ClientIp(ip): ClientIp, a: Auth, J(b): J<CodeBody>) -> ApiResult<Json<Value>> {
    require_admin(&a)?;
    crate::auth::rate_check(&s, "2fa", &ip)?;
    ensure_not_locked(&s, &a.user.id)?;
    let (uid, th) = (a.user.id.clone(), a.token_hash.clone());
    let r = s.db.call(move |c| Ok(row(c, &uid)?)).await?;
    if r.enabled() {
        return Err(ApiError::conflict("Die Zwei-Faktor-Anmeldung ist bereits eingerichtet."));
    }
    let pending = r.pending.ok_or_else(|| ApiError::invalid("Starte zuerst die Einrichtung."))?;
    let secret = open(&s.cfg.data, &a.user.id, &pending)?;
    let Some(step) = check(&secret, b.code.trim(), now(), 0) else {
        return Err(note_fail(&s, &a.user.id));
    };
    let uid = a.user.id.clone();
    let codes = s
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            tx.execute("UPDATE users SET totp_secret=totp_pending, totp_pending=NULL, totp_last_step=?2 WHERE id=?1", params![uid, step])?;
            let codes = new_recovery_codes(&tx, &uid)?;
            // Andere Sitzungen dieses Kontos sind ab jetzt keine Admin-Sitzungen mehr.
            tx.execute("UPDATE sessions SET mfa=0 WHERE user_id=?1", [&uid])?;
            set_session_mfa(&tx, &th)?;
            tx.commit()?;
            Ok(codes)
        })
        .await?;
    Ok(Json(json!({"recovery_codes": codes})))
}

/// `POST /api/auth/2fa/verify`: zweiter Faktor beim Anmelden (Code aus der App oder Wiederherstellungscode).
pub async fn verify(State(s): State<AppState>, ClientIp(ip): ClientIp, a: Auth, J(b): J<CodeBody>) -> ApiResult<Json<Value>> {
    require_admin(&a)?;
    crate::auth::rate_check(&s, "2fa", &ip)?;
    ensure_not_locked(&s, &a.user.id)?;
    let uid = a.user.id.clone();
    let r = s.db.call(move |c| Ok(row(c, &uid)?)).await?;
    let sealed = r.secret.ok_or_else(|| ApiError::invalid("Die Zwei-Faktor-Anmeldung ist noch nicht eingerichtet."))?;
    let code = b.code.trim().to_string();
    let digits = code.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let (uid, th) = (a.user.id.clone(), a.token_hash.clone());
    let ok = if digits.len() == DIGITS as usize && digits.chars().all(|c| c.is_ascii_digit()) {
        let secret = open(&s.cfg.data, &a.user.id, &sealed)?;
        match check(&secret, &digits, now(), r.last_step) {
            Some(step) => {
                // Bedingtes Update: ein Code gilt auch bei gleichzeitigen Anfragen nur einmal.
                s.db.call(move |c| Ok(c.execute("UPDATE users SET totp_last_step=?2 WHERE id=?1 AND totp_last_step<?2", params![uid, step])? == 1)).await?
            }
            None => false,
        }
    } else if norm_recovery(&code).len() == 10 {
        s.db.call(move |c| Ok(use_recovery(c, &uid, &code)?)).await?
    } else {
        false
    };
    if !ok {
        return Err(note_fail(&s, &a.user.id));
    }
    s.limiter.clear(&fail_key(&a.user.id));
    let user = a.user.clone();
    let me = s
        .db
        .call(move |c| {
            set_session_mfa(c, &th)?;
            let mut me = db::me_json(c, &user)?;
            me["mfa"] = me_mfa(c, &user.id, Some(&th))?;
            Ok(me)
        })
        .await?;
    Ok(Json(me))
}

/// `POST /api/admin/2fa/recovery-codes`: neue Wiederherstellungscodes (bestätigt mit einem aktuellen Code).
pub async fn regenerate(State(s): State<AppState>, ClientIp(ip): ClientIp, a: crate::auth::AdminAuth, J(b): J<CodeBody>) -> ApiResult<Json<Value>> {
    let a = a.0;
    crate::auth::rate_check(&s, "2fa", &ip)?;
    ensure_not_locked(&s, &a.user.id)?;
    let uid = a.user.id.clone();
    let r = s.db.call(move |c| Ok(row(c, &uid)?)).await?;
    let sealed = r.secret.ok_or_else(|| ApiError::invalid("Die Zwei-Faktor-Anmeldung ist noch nicht eingerichtet."))?;
    let secret = open(&s.cfg.data, &a.user.id, &sealed)?;
    let Some(step) = check(&secret, b.code.trim(), now(), r.last_step) else {
        return Err(note_fail(&s, &a.user.id));
    };
    let uid = a.user.id.clone();
    let codes = s
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            if tx.execute("UPDATE users SET totp_last_step=?2 WHERE id=?1 AND totp_last_step<?2", params![uid, step])? != 1 {
                return Ok(None);
            }
            let codes = new_recovery_codes(&tx, &uid)?;
            tx.commit()?;
            Ok(Some(codes))
        })
        .await?;
    let codes = codes.ok_or_else(|| note_fail(&s, &a.user.id))?;
    Ok(Json(json!({"recovery_codes": codes})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_testvektoren() {
        // RFC 6238 Anhang B (SHA-1, Geheimnis "12345678901234567890"), letzte 6 Stellen.
        let k = b"12345678901234567890";
        for (t, want) in [(59, 287082), (1111111109, 81804), (1111111111, 50471), (1234567890, 5924), (2000000000, 279037)] {
            assert_eq!(code_at_step(k, t / STEP), want, "t={t}");
        }
    }

    #[test]
    fn fenster_und_wiederverwendung() {
        let k = b"12345678901234567890";
        let t = 1111111109;
        let code = format!("{:06}", code_at_step(k, t / STEP - 1));
        assert_eq!(check(k, &code, t, 0), Some(t / STEP - 1));
        assert_eq!(check(k, &code, t, t / STEP - 1), None, "derselbe Schritt gilt nur einmal");
        let alt = format!("{:06}", code_at_step(k, t / STEP - 2));
        assert_eq!(check(k, &alt, t, 0), None, "zu alt");
        assert_eq!(check(k, "abc", t, 0), None);
    }

    #[test]
    fn base32_und_verschluesselung() {
        let s = b"\x01\x02\x03\x04\x05abcdefghijklmno";
        assert_eq!(decode_b32(&encode_b32(s).to_lowercase()).unwrap(), s);
        let dir = tempfile::tempdir().unwrap();
        let sealed = seal(dir.path(), "u1", s).unwrap();
        assert_eq!(open(dir.path(), "u1", &sealed).unwrap(), s);
        assert!(open(dir.path(), "u2", &sealed).is_err(), "an das Konto gebunden");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(KEY_FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
