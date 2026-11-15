//! Platzhalter für die Server-Anbindung der Web-App (Phase 2, Vertrag: `docs/web-api.md`).
//!
//! Hier docken später an (alles unter derselben Origin, Cookie `owndraw_session`, schreibende Anfragen mit
//! Header `X-OwnDraw: 1`):
//! - Konto: `GET /api/me` beim Start (Gast, wenn 401), Anmelden/Registrieren/Abmelden, Lizenz einlösen.
//! - Boards am Server (nur mit gültiger Lizenz): `GET/POST /api/boards`, `GET/PUT /api/boards/{id}` mit
//!   `If-Match`-Version. Geplant als weiterer Verweis `FileRef::Uri("srv:<id>")` in [`super::WebPlatform`]:
//!   `read` lädt, `write_atomic` speichert per PUT (Autosave), `exists`/Zuletzt-Liste aus `GET /api/boards`.
//! - Teilen/Mitglieder, später Kollaboration per WebSocket (`/api/collab/{board_id}`).
//!
//! Im Gastmodus (Phase 1) ist hier nichts aktiv.

/// Angemeldeter Nutzer (Antwort `Me` aus `docs/web-api.md`); im Gastmodus `None`.
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct Session {
    pub id: String,
    pub email: String,
    pub name: String,
    pub admin: bool,
    /// Gültige Lizenz: Boards am Server speichern und teilen.
    pub licensed: bool,
}

/// Zugang zum Server (noch leer).
#[allow(dead_code)]
#[derive(Default)]
pub struct ServerApi {
    pub session: Option<Session>,
}
