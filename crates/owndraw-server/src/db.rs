//! SQLite-Zugriff (eine Verbindung hinter einem Mutex, Aufrufe über `spawn_blocking`)
//! sowie gemeinsame Abfragen für Nutzer, Lizenzen und Einstellungen.

use crate::error::{ApiError, ApiResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    pw_hash TEXT NOT NULL,
    admin INTEGER NOT NULL DEFAULT 0,
    disabled INTEGER NOT NULL DEFAULT 0,
    created INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
    token_hash TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created INTEGER NOT NULL,
    expires INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS licenses (
    code TEXT PRIMARY KEY,
    created INTEGER NOT NULL,
    days INTEGER,
    expires_at INTEGER,
    quota_mb INTEGER NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    revoked INTEGER NOT NULL DEFAULT 0,
    redeemed_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    redeemed_at INTEGER
);
CREATE TABLE IF NOT EXISTS boards (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    version INTEGER NOT NULL,
    size INTEGER NOT NULL,
    modified INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS members (
    board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    PRIMARY KEY (board_id, user_id)
);
CREATE TABLE IF NOT EXISTS invites (
    id TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    created INTEGER NOT NULL,
    expires_at INTEGER,
    uses INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS notices (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'info',
    active INTEGER NOT NULL DEFAULT 1,
    starts_at INTEGER,
    ends_at INTEGER,
    created INTEGER NOT NULL,
    updated INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS bug_reports (
    id TEXT PRIMARY KEY,
    created INTEGER NOT NULL,
    user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    sender TEXT NOT NULL,
    guest INTEGER NOT NULL DEFAULT 0,
    text TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '',
    user_agent TEXT NOT NULL DEFAULT '',
    page TEXT NOT NULL DEFAULT '',
    screen TEXT NOT NULL DEFAULT '',
    images TEXT NOT NULL DEFAULT '[]',
    size INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'open'
);
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_boards_owner ON boards(owner_id);
CREATE INDEX IF NOT EXISTS idx_members_user ON members(user_id);
CREATE INDEX IF NOT EXISTS idx_licenses_user ON licenses(redeemed_by);
CREATE INDEX IF NOT EXISTS idx_invites_board ON invites(board_id);
";

/// Spalten, die nach 4.6 dazugekommen sind (ältere Datenbanken bekommen sie beim Start).
fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let has_guest = conn.prepare("SELECT 1 FROM pragma_table_info('users') WHERE name='guest'")?.exists([])?;
    if !has_guest {
        conn.execute_batch("ALTER TABLE users ADD COLUMN guest INTEGER NOT NULL DEFAULT 0;")?;
    }
    Ok(())
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    /// Führt `f` auf einem Blocking-Thread mit der DB-Verbindung aus.
    pub async fn call<T, F>(&self, f: F) -> ApiResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> ApiResult<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().unwrap_or_else(|p| p.into_inner());
            f(&mut guard)
        })
        .await
        .map_err(ApiError::internal)?
    }
}

// ---------- Einstellungen ----------

pub fn get_setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0)).optional()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, value])?;
    Ok(())
}

// ---------- Nutzer ----------

#[derive(Clone, Debug)]
pub struct UserRow {
    pub id: String,
    pub email: String,
    pub name: String,
    pub pw_hash: String,
    pub admin: bool,
    pub disabled: bool,
    pub created: i64,
    /// Gast ohne Konto (über einen Einladungslink beigetreten): kein Passwort, keine Lizenz, eigene Boards nicht möglich.
    pub guest: bool,
}

fn user_from_row(r: &rusqlite::Row) -> rusqlite::Result<UserRow> {
    Ok(UserRow {
        id: r.get(0)?,
        email: r.get(1)?,
        name: r.get(2)?,
        pw_hash: r.get(3)?,
        admin: r.get::<_, i64>(4)? != 0,
        disabled: r.get::<_, i64>(5)? != 0,
        created: r.get(6)?,
        guest: r.get::<_, i64>(7)? != 0,
    })
}

const USER_COLS: &str = "id,email,name,pw_hash,admin,disabled,created,guest";

pub fn user_by_email(conn: &Connection, email: &str) -> rusqlite::Result<Option<UserRow>> {
    conn.query_row(&format!("SELECT {USER_COLS} FROM users WHERE email=?1"), [email], user_from_row).optional()
}

pub fn user_by_id(conn: &Connection, id: &str) -> rusqlite::Result<Option<UserRow>> {
    conn.query_row(&format!("SELECT {USER_COLS} FROM users WHERE id=?1"), [id], user_from_row).optional()
}

pub fn insert_user(conn: &Connection, email: &str, name: &str, pw_hash: &str, admin: bool) -> ApiResult<UserRow> {
    if user_by_email(conn, email)?.is_some() {
        return Err(ApiError::conflict("Diese E-Mail-Adresse ist bereits registriert."));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let created = now();
    conn.execute(
        "INSERT INTO users(id,email,name,pw_hash,admin,disabled,created) VALUES(?1,?2,?3,?4,?5,0,?6)",
        params![id, email, name, pw_hash, admin as i64, created],
    )?;
    Ok(UserRow { id, email: email.into(), name: name.into(), pw_hash: pw_hash.into(), admin, disabled: false, created, guest: false })
}

pub fn used_bytes(conn: &Connection, user_id: &str) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COALESCE(SUM(size),0) FROM boards WHERE owner_id=?1", [user_id], |r| r.get(0))
}

// ---------- Lizenzen ----------

#[derive(Clone, Debug)]
pub struct LicenseRow {
    pub code: String,
    pub created: i64,
    pub days: Option<i64>,
    pub expires_at: Option<i64>,
    pub quota_mb: i64,
    pub note: String,
    pub revoked: bool,
    pub redeemed_by: Option<(String, String)>,
    pub redeemed_at: Option<i64>,
}

impl LicenseRow {
    /// Gültig = eingelöst, nicht widerrufen, `expires_at` leer oder in der Zukunft.
    pub fn active(&self, now: i64) -> bool {
        self.redeemed_by.is_some() && !self.revoked && self.expires_at.is_none_or(|e| e > now)
    }

    pub fn to_json(&self, now: i64) -> Value {
        json!({
            "code": self.code,
            "created": self.created,
            "days": self.days,
            "expires_at": self.expires_at,
            "quota_mb": self.quota_mb,
            "note": self.note,
            "revoked": self.revoked,
            "redeemed_by": self.redeemed_by.as_ref().map(|(id, email)| json!({"id": id, "email": email})),
            "redeemed_at": self.redeemed_at,
            "active": self.active(now),
        })
    }
}

const LICENSE_SELECT: &str = "SELECT l.code,l.created,l.days,l.expires_at,l.quota_mb,l.note,l.revoked,l.redeemed_by,u.email,l.redeemed_at
     FROM licenses l LEFT JOIN users u ON u.id=l.redeemed_by";

fn license_from_row(r: &rusqlite::Row) -> rusqlite::Result<LicenseRow> {
    let by: Option<String> = r.get(7)?;
    let email: Option<String> = r.get(8)?;
    Ok(LicenseRow {
        code: r.get(0)?,
        created: r.get(1)?,
        days: r.get(2)?,
        expires_at: r.get(3)?,
        quota_mb: r.get(4)?,
        note: r.get(5)?,
        revoked: r.get::<_, i64>(6)? != 0,
        redeemed_by: by.map(|id| (id, email.unwrap_or_default())),
        redeemed_at: r.get(9)?,
    })
}

pub fn license_by_code(conn: &Connection, code: &str) -> rusqlite::Result<Option<LicenseRow>> {
    conn.query_row(&format!("{LICENSE_SELECT} WHERE l.code=?1"), [code], license_from_row).optional()
}

pub fn all_licenses(conn: &Connection) -> rusqlite::Result<Vec<LicenseRow>> {
    let mut st = conn.prepare(&format!("{LICENSE_SELECT} ORDER BY l.created DESC, l.code"))?;
    let rows = st.query_map([], license_from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Die Lizenz eines Nutzers: die zuletzt eingelöste, nicht deaktivierte vor deaktivierten.
pub fn license_of(conn: &Connection, user_id: &str) -> rusqlite::Result<Option<LicenseRow>> {
    conn.query_row(&format!("{LICENSE_SELECT} WHERE l.redeemed_by=?1 ORDER BY l.revoked, l.redeemed_at DESC, l.rowid DESC LIMIT 1"), [user_id], license_from_row)
        .optional()
}

pub fn active_license_of(conn: &Connection, user_id: &str, now: i64) -> rusqlite::Result<Option<LicenseRow>> {
    Ok(license_of(conn, user_id)?.filter(|l| l.active(now)))
}

pub fn mb(bytes: i64) -> f64 {
    ((bytes as f64) / 1_048_576.0 * 100.0).round() / 100.0
}

/// `Me`-Objekt des Vertrags.
pub fn me_json(conn: &Connection, u: &UserRow) -> ApiResult<Value> {
    let t = now();
    let license = license_of(conn, &u.id)?.map(|l| {
        json!({
            "active": l.active(t),
            "expires_at": l.expires_at,
            "quota_mb": l.quota_mb,
            "used_mb": 0.0,
        })
    });
    let license = match license {
        Some(mut v) => {
            v["used_mb"] = json!(mb(used_bytes(conn, &u.id)?));
            v
        }
        None => Value::Null,
    };
    let email = if u.guest { "" } else { u.email.as_str() };
    Ok(json!({"id": u.id, "email": email, "name": u.name, "admin": u.admin, "guest": u.guest, "license": license}))
}

/// Ein kryptografisch zufälliges Zeichen aus `ABCDEFGHJKLMNPQRSTUVWXYZ23456789` (ohne 0/O/1/I).
fn code_char() -> char {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut buf = [0u8; 1];
    loop {
        getrandom::fill(&mut buf).expect("Zufallsquelle nicht verfügbar");
        // 224 = 7 * 32: Werte darüber verwerfen, damit keine Verzerrung entsteht.
        if buf[0] < 224 {
            return ALPHABET[(buf[0] % 32) as usize] as char;
        }
    }
}

/// Erzeugt einen Lizenzcode `OWND-XXXX-XXXX-XXXX` (ohne 0/O/1/I), kryptografisch zufällig.
pub fn new_license_code() -> String {
    let mut out = String::from("OWND");
    for i in 0..12 {
        if i % 4 == 0 {
            out.push('-');
        }
        out.push(code_char());
    }
    out
}

// ---------- Gäste ----------

/// Länge des selbst gewählten Namensteils einer Gast-ID.
pub const GUEST_NAME_MAX: usize = 24;

/// Prüft und kürzt den gewünschten Gastnamen (ohne `#`, keine Steuerzeichen, 1–24 Zeichen).
pub fn clean_guest_name(raw: &str) -> ApiResult<String> {
    let name: String = raw.trim().chars().filter(|c| !c.is_control() && *c != '#').collect();
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err(ApiError::invalid("Bitte einen Namen angeben."));
    }
    if name.chars().count() > GUEST_NAME_MAX {
        return Err(ApiError::invalid(format!("Der Name darf höchstens {GUEST_NAME_MAX} Zeichen lang sein.")));
    }
    Ok(name)
}

/// Legt einen Gast an. Seine ID für die Anzeige ist `<Name>#<4 Zeichen>` (eindeutig unter allen Nutzern).
pub fn insert_guest(conn: &Connection, name: &str) -> ApiResult<UserRow> {
    for _ in 0..50 {
        let tag: String = (0..4).map(|_| code_char()).collect();
        let display = format!("{name}#{tag}");
        let taken: bool = conn.prepare("SELECT 1 FROM users WHERE name=?1")?.exists([&display])?;
        if taken {
            continue;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let created = now();
        // Platzhalter-Adresse (nicht anmeldbar, wird nie angezeigt); `!` ist kein gültiger Passwort-Hash.
        let email = format!("gast:{id}");
        conn.execute(
            "INSERT INTO users(id,email,name,pw_hash,admin,disabled,created,guest) VALUES(?1,?2,?3,'!',0,0,?4,1)",
            params![id, email, display, created],
        )?;
        return Ok(UserRow { id, email, name: display, pw_hash: "!".into(), admin: false, disabled: false, created, guest: true });
    }
    Err(ApiError::internal("Keine freie Gast-ID gefunden"))
}

/// Löscht Gäste ohne gültige Sitzung (abgelaufen oder abgemeldet) samt ihren Mitgliedschaften.
pub fn purge_guests(conn: &Connection, now: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM users WHERE guest=1 AND NOT EXISTS (SELECT 1 FROM sessions s WHERE s.user_id=users.id AND s.expires>?1)",
        [now],
    )
}

pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("Zufallsquelle nicht verfügbar");
    hex::encode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gast_ids_eindeutig_und_name_geprueft() {
        let db = Db::memory().unwrap();
        let c = db.conn.lock().unwrap();
        let a = insert_guest(&c, "Mila").unwrap();
        let b = insert_guest(&c, "Mila").unwrap();
        assert!(a.guest && a.name.starts_with("Mila#") && a.name.len() == 9);
        assert_ne!(a.name, b.name);
        assert_ne!(a.id, b.id);
        assert_eq!(clean_guest_name("  Ben  #  Lu ").unwrap(), "Ben Lu");
        assert!(clean_guest_name("   ").is_err());
        assert!(clean_guest_name(&"x".repeat(GUEST_NAME_MAX + 1)).is_err());
        // ohne Sitzung: wird aufgeräumt
        assert_eq!(purge_guests(&c, now()).unwrap(), 2);
    }

    #[test]
    fn lizenzcode_format() {
        for _ in 0..200 {
            let c = new_license_code();
            assert_eq!(c.len(), 19);
            assert!(c.starts_with("OWND-"));
            for (i, part) in c.split('-').enumerate().skip(1) {
                assert_eq!(part.len(), 4, "Teil {i} in {c}");
                assert!(part.chars().all(|ch| ch.is_ascii_uppercase() || ('2'..='9').contains(&ch)));
                assert!(!part.contains(['O', 'I', '0', '1']));
            }
        }
    }
}
