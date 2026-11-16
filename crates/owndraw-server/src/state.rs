//! Gemeinsamer Anwendungszustand.

use crate::config::Config;
use crate::db::Db;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub cfg: Arc<Config>,
    pub limiter: Arc<RateLimiter>,
    /// Verhindert parallele Installationen eines Updates.
    pub installing: Arc<AtomicBool>,
    /// Offene Live-Sitzungen (`collab.rs`).
    pub hub: Arc<crate::collab::Hub>,
    /// Wartungsmodus, zwischengespeichert (`None` = noch nicht aus der DB geladen), siehe `notices.rs`.
    pub maintenance: Arc<RwLock<Option<crate::notices::Maintenance>>>,
    /// Zuletzt aktive Nutzer für die Admin-Statistik (`stats.rs`).
    pub seen: Arc<crate::stats::Seen>,
}

impl AppState {
    pub fn new(db: Db, cfg: Config) -> Self {
        Self { db, cfg: Arc::new(cfg), limiter: Arc::new(RateLimiter::default()), installing: Arc::new(AtomicBool::new(false)), hub: Arc::default(), maintenance: Arc::default(), seen: Arc::default() }
    }
}

/// Einfacher Rate-Limiter im Speicher (Schlüssel → Zeitpunkte der letzten Versuche).
#[derive(Default)]
pub struct RateLimiter {
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    /// Zählt einen Versuch; `false`, wenn das Limit im Fenster überschritten ist.
    pub fn hit(&self, key: &str, limit: u32, window: Duration) -> bool {
        let mut map = self.hits.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        if map.len() > 10_000 {
            map.retain(|_, q| q.back().is_some_and(|t| now.duration_since(*t) < window));
        }
        let q = map.entry(key.to_string()).or_default();
        while q.front().is_some_and(|t| now.duration_since(*t) >= window) {
            q.pop_front();
        }
        if q.len() as u32 >= limit {
            return false;
        }
        q.push_back(now);
        true
    }

    /// Anzahl der Versuche im Fenster, ohne einen neuen zu zählen.
    pub fn count(&self, key: &str, window: Duration) -> u32 {
        let map = self.hits.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        map.get(key).map_or(0, |q| q.iter().filter(|t| now.duration_since(**t) < window).count() as u32)
    }

    pub fn clear(&self, key: &str) {
        self.hits.lock().unwrap_or_else(|p| p.into_inner()).remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_sperrt_nach_limit() {
        let l = RateLimiter::default();
        let w = Duration::from_secs(60);
        assert!(l.hit("a", 2, w));
        assert!(l.hit("a", 2, w));
        assert!(!l.hit("a", 2, w));
        assert!(l.hit("b", 2, w));
    }
}
