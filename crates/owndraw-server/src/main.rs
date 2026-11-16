//! `owndraw-server`: Webserver. Unterbefehle: `admin create <email>` (Passwort per stdin),
//! `admin reset-2fa <email>` (Notfall-Reset der Zwei-Faktor-Anmeldung), `--version`.

use owndraw_server::{build_app, config::Config, ensure_admin_from_env, open_state, update};
use std::io::BufRead;

const USAGE: &str = "Verwendung:
  owndraw-server                       Server starten (Konfiguration über OWNDRAW_*-Umgebungsvariablen)
  owndraw-server admin create <email>  Admin anlegen oder befördern (Passwort wird von stdin gelesen)
  owndraw-server admin reset-2fa <email>
                                       Zwei-Faktor-Anmeldung eines Admins zurücksetzen (Notfall);
                                       beim nächsten Login wird sie neu eingerichtet
  owndraw-server --version";

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["--version" | "-V" | "version"] => {
            println!("{}", update::CURRENT);
            0
        }
        [] | ["serve"] => run_server().await,
        ["admin", "create", email] => admin_create(email).await,
        ["admin", "reset-2fa", email] => admin_reset_2fa(email).await,
        _ => {
            eprintln!("{USAGE}");
            2
        }
    };
    std::process::exit(code);
}

async fn admin_create(email: &str) -> i32 {
    eprintln!("Passwort für {email} (eine Zeile von stdin, mindestens 8 Zeichen):");
    let mut pw = String::new();
    if std::io::stdin().lock().read_line(&mut pw).is_err() {
        eprintln!("stdin nicht lesbar");
        return 1;
    }
    let pw = pw.trim_end_matches(['\r', '\n']);
    let state = match open_state(Config::from_env()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    match owndraw_server::auth::create_admin(&state.db, email, pw).await {
        Ok(()) => {
            eprintln!("Admin {email} ist angelegt.");
            0
        }
        Err(e) => {
            eprintln!("Fehler: {}", e.message);
            1
        }
    }
}

async fn admin_reset_2fa(email: &str) -> i32 {
    let state = match open_state(Config::from_env()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    match owndraw_server::totp::reset(&state.db, email).await {
        Ok(true) => {
            eprintln!("Zwei-Faktor-Anmeldung für {email} zurückgesetzt. Alle Sitzungen des Kontos sind beendet.");
            eprintln!("Beim nächsten Login unter /admin wird sie neu eingerichtet.");
            0
        }
        Ok(false) => {
            eprintln!("Kein Admin-Konto mit der Adresse {email} gefunden.");
            1
        }
        Err(e) => {
            eprintln!("Fehler: {}", e.message);
            1
        }
    }
}

async fn run_server() -> i32 {
    let cfg = Config::from_env();
    let bind = cfg.bind.clone();
    let web = cfg.web.clone();
    let state = match open_state(cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    if let Err(e) = ensure_admin_from_env(&state).await {
        eprintln!("Admin konnte nicht angelegt werden: {e}");
        return 1;
    }
    if !web.join("index.html").is_file() {
        eprintln!("Hinweis: {} enthält keine index.html (OWNDRAW_WEB prüfen); die Web-App wird nicht ausgeliefert.", web.display());
    }
    update::spawn_auto(state.clone());
    owndraw_server::downloads::spawn_refresh(state.clone());
    let st2 = state.clone();
    let app = build_app(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Kann {bind} nicht öffnen: {e}");
            return 1;
        }
    };
    eprintln!("OwnDraw-Server {} lauscht auf {bind}", update::CURRENT);
    let r = axum::serve(listener, app).with_graceful_shutdown(shutdown()).await;
    // offene Live-Sitzungen noch speichern
    st2.hub.flush_all(&st2).await;
    if let Err(e) = r {
        eprintln!("Serverfehler: {e}");
        return 1;
    }
    0
}

async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = term.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
