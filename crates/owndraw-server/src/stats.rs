//! Statistik für das Admin-Panel (`GET /api/admin/stats`), jeweils getrennt nach Konten und Gästen.
//!
//! Definitionen (auch in `docs/server.md`):
//! - Online: Nutzer mit einer angemeldeten API-Anfrage in den letzten [`ONLINE_MINUTES`] Minuten oder einer offenen
//!   Live-Verbindung. Wird nur im Speicher geführt (nach einem Neustart beginnt die Zählung neu).
//! - Erstellte Boards: Konten = Boards am Server (Besitzer ist immer ein Konto), Gäste = Boards mit mindestens einem
//!   Gast als Mitglied (Gäste können selbst keine Server-Boards anlegen). `total_ever` zählt jedes je angelegte Board,
//!   auch gelöschte (Zähler in `settings`, beim ersten Abruf mit der aktuellen Anzahl begonnen).
//! - Gerade bearbeitet: Boards mit offener Live-Sitzung, Konten/Gäste = Boards mit mindestens einem solchen Teilnehmer.
//! - Gespeichert: Anzahl Boards und Summe ihrer Dateigrößen.

use crate::auth::AdminAuth;
use crate::db::{self, now};
use crate::error::ApiResult;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const ONLINE_MINUTES: u64 = 5;
const CREATED_KEY: &str = "stats_boards_created";

/// Zuletzt gesehene angemeldete Nutzer (Nutzer-ID → Zeitpunkt, Gast).
#[derive(Default)]
pub struct Seen {
    map: Mutex<HashMap<String, (Instant, bool)>>,
}

impl Seen {
    pub fn touch(&self, user_id: &str, guest: bool) {
        let mut m = self.map.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        if m.len() > 50_000 {
            m.retain(|_, (t, _)| now.duration_since(*t) < Duration::from_secs(ONLINE_MINUTES * 60));
        }
        m.insert(user_id.to_string(), (now, guest));
    }

    /// Nutzer, die innerhalb von `window` aktiv waren (Nutzer-ID → Gast).
    pub fn recent(&self, window: Duration) -> HashMap<String, bool> {
        let mut m = self.map.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        m.retain(|_, (t, _)| now.duration_since(*t) < window);
        m.iter().map(|(k, (_, g))| (k.clone(), *g)).collect()
    }
}

/// Zählt ein neu angelegtes Board (für `total_ever`).
pub fn count_board_created(c: &Connection) -> rusqlite::Result<()> {
    // Ohne Zähler: mit der aktuellen Anzahl beginnen (das neue Board ist schon eingetragen).
    let n = match db::get_setting(c, CREATED_KEY)?.and_then(|v| v.parse::<i64>().ok()) {
        Some(n) => n + 1,
        None => c.query_row("SELECT COUNT(*) FROM boards", [], |r| r.get(0))?,
    };
    db::set_setting(c, CREATED_KEY, &n.to_string())
}

fn created_total(c: &Connection) -> rusqlite::Result<i64> {
    match db::get_setting(c, CREATED_KEY)?.and_then(|v| v.parse().ok()) {
        Some(n) => Ok(n),
        None => c.query_row("SELECT COUNT(*) FROM boards", [], |r| r.get(0)),
    }
}

fn split(m: impl IntoIterator<Item = bool>) -> Value {
    let (mut a, mut g) = (0, 0);
    for guest in m {
        if guest {
            g += 1
        } else {
            a += 1
        }
    }
    json!({"account": a, "guest": g})
}

pub async fn get(State(s): State<AppState>, _a: AdminAuth) -> ApiResult<Json<Value>> {
    let rooms = s.hub.snapshot();
    let mut online = s.seen.recent(Duration::from_secs(ONLINE_MINUTES * 60));
    for r in &rooms {
        for (uid, guest) in &r.peers {
            online.insert(uid.clone(), *guest);
        }
    }
    let v =
        s.db.call(move |c| {
            let q = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0));
            let mut active = Vec::new();
            let (mut act_acc, mut act_guest) = (0, 0);
            for r in &rooms {
                let row: Option<(String, String)> = c
                    .query_row(
                        "SELECT b.name, COALESCE(u.name,'') FROM boards b LEFT JOIN users u ON u.id=b.owner_id WHERE b.id=?1",
                        [&r.id],
                        |x| Ok((x.get(0)?, x.get(1)?)),
                    )
                    .optional()?;
                let Some((name, owner)) = row else { continue };
                let ids: HashSet<(&String, bool)> = r.peers.iter().map(|(u, g)| (u, *g)).collect();
                let guests = ids.iter().filter(|(_, g)| *g).count();
                let accounts = ids.len() - guests;
                if accounts > 0 {
                    act_acc += 1;
                }
                if guests > 0 {
                    act_guest += 1;
                }
                active.push(json!({"id": r.id, "name": name, "owner": owner, "accounts": accounts, "guests": guests}));
            }
            Ok(json!({
                "time": now(),
                "online_minutes": ONLINE_MINUTES,
                "online": split(online.values().copied()),
                "users": {
                    "account": q("SELECT COUNT(*) FROM users WHERE guest=0")?,
                    "guest": q("SELECT COUNT(*) FROM users WHERE guest=1")?,
                },
                "boards_created": {
                    "account": q("SELECT COUNT(*) FROM boards")?,
                    "guest": q("SELECT COUNT(DISTINCT m.board_id) FROM members m JOIN users u ON u.id=m.user_id WHERE u.guest=1")?,
                    "total_ever": created_total(c)?,
                },
                "boards_active": {"account": act_acc, "guest": act_guest, "list": active},
                "stored": {
                    "boards": q("SELECT COUNT(*) FROM boards")?,
                    "bytes": q("SELECT COALESCE(SUM(size),0) FROM boards")?,
                },
            }))
        })
        .await?;
    Ok(Json(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seen_vergisst_alte_eintraege() {
        let s = Seen::default();
        s.touch("a", false);
        s.touch("g", true);
        s.touch("a", false);
        let r = s.recent(Duration::from_secs(60));
        assert_eq!(r.len(), 2);
        assert_eq!(split(r.values().copied()), json!({"account": 1, "guest": 1}));
        assert!(s.recent(Duration::ZERO).is_empty());
    }
}
