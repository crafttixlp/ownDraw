//! Tests mit lokalem HTTP-Server (kein echtes GitHub) und zur Laufzeit erzeugten minisign-Schlüsseln.

use super::*;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

struct Keys {
    pk_text: String,
    pk: minisign::PublicKey,
    sk: minisign::SecretKey,
}

fn keys() -> Keys {
    let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    Keys {
        pk_text: kp.pk.to_box().unwrap().to_string(),
        pk: kp.pk,
        sk: kp.sk,
    }
}

fn sign(k: &Keys, data: &[u8]) -> Vec<u8> {
    minisign::sign(
        Some(&k.pk),
        &k.sk,
        Cursor::new(data),
        Some("trusted"),
        Some("test"),
    )
    .unwrap()
    .to_string()
    .into_bytes()
}

/// Antwort, die ihren Inhalt langsam liefert (für den Abbruch-Test).
struct Slow {
    data: Cursor<Vec<u8>>,
}

impl Read for Slow {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::thread::sleep(Duration::from_millis(15));
        let n = buf.len().min(32 * 1024);
        self.data.read(&mut buf[..n])
    }
}

struct Server {
    base: String,
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    slow: Arc<Mutex<Vec<String>>>,
    hits: Arc<AtomicUsize>,
}

impl Server {
    fn start() -> Server {
        let srv = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = srv.server_addr().to_ip().unwrap().port();
        let files: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::default();
        let slow: Arc<Mutex<Vec<String>>> = Arc::default();
        let hits = Arc::new(AtomicUsize::new(0));
        let (f, sl, h) = (files.clone(), slow.clone(), hits.clone());
        std::thread::spawn(move || {
            for req in srv.incoming_requests() {
                h.fetch_add(1, Ordering::SeqCst);
                let url = req.url().to_string();
                let body = f.lock().unwrap().get(&url).cloned();
                let is_slow = sl.lock().unwrap().contains(&url);
                std::thread::spawn(move || match body {
                    None => drop(req.respond(
                        tiny_http::Response::from_string("nicht gefunden").with_status_code(404),
                    )),
                    Some(b) if is_slow => {
                        let len = b.len();
                        drop(req.respond(tiny_http::Response::new(
                            tiny_http::StatusCode(200),
                            vec![],
                            Slow {
                                data: Cursor::new(b),
                            },
                            Some(len),
                            None,
                        )));
                    }
                    Some(b) => drop(req.respond(tiny_http::Response::from_data(b))),
                });
            }
        });
        Server {
            base: format!("http://127.0.0.1:{port}"),
            files,
            slow,
            hits,
        }
    }

    fn put(&self, path: &str, data: Vec<u8>) {
        self.files.lock().unwrap().insert(path.to_string(), data);
    }

    /// Legt ein Asset samt Signatur (mit `signer`) ab und liefert die Asset-JSONs.
    fn asset(&self, name: &str, data: &[u8], signer: &Keys) -> Vec<serde_json::Value> {
        self.put(&format!("/dl/{name}"), data.to_vec());
        self.put(&format!("/dl/{name}.minisig"), sign(signer, data));
        [name.to_string(), format!("{name}.minisig")]
            .iter()
            .map(|n| serde_json::json!({"name": n, "browser_download_url": format!("{}/dl/{n}", self.base), "size": 1}))
            .collect()
    }

    fn release(&self, v: &str, assets: Vec<serde_json::Value>) -> serde_json::Value {
        serde_json::json!({"tag_name": format!("v{v}"), "name": format!("OwnDraw {v}"), "body": format!("Änderungen {v}"),
            "published_at": "2026-05-01T12:00:00Z", "draft": false, "prerelease": false, "assets": assets})
    }

    fn publish(&self, releases: &[serde_json::Value]) {
        self.put(
            "/repos/o/r/releases/latest",
            releases[0].to_string().into_bytes(),
        );
        self.put(
            "/repos/o/r/releases?per_page=50",
            serde_json::Value::Array(releases.to_vec())
                .to_string()
                .into_bytes(),
        );
    }
}

struct Fix {
    dir: tempfile::TempDir,
    srv: Server,
    keys: Keys,
    cfg: Config,
}

impl Fix {
    fn target(&self) -> PathBuf {
        self.dir.path().join("OwnDraw.AppImage")
    }
}

fn app_name(v: &str) -> String {
    package_asset(&Version::parse(v).unwrap()).unwrap()
}

/// Server mit Releases 4.1.0 und 4.0.0 (Paket-Assets), laufende Version `current`, "AppImage" mit Inhalt `alt`.
fn fixture(current: &str) -> Fix {
    let dir = tempfile::tempdir().unwrap();
    let srv = Server::start();
    let keys = keys();
    let a410 = srv.asset(&app_name("4.1.0"), b"APPIMAGE-4.1.0", &keys);
    let a400 = srv.asset(&app_name("4.0.0"), b"APPIMAGE-4.0.0", &keys);
    srv.publish(&[srv.release("4.1.0", a410), srv.release("4.0.0", a400)]);
    let target = dir.path().join("OwnDraw.AppImage");
    std::fs::write(&target, format!("APPIMAGE-{current}")).unwrap();
    let cfg = Config {
        api_base: srv.base.clone(),
        repo: "o/r".into(),
        current: Version::parse(current).unwrap(),
        pubkey: keys.pk_text.clone(),
        work_dir: dir.path().join("cache/update"),
        build_dir: dir.path().join("cache/build"),
        versions_dir: dir.path().join("data/versions"),
        target: Some(target),
        skip: None,
        mode: UpdateMode::Package,
        allow_http: true,
        cargo: None,
        spec: build::BuildSpec {
            package: "mini".into(),
            bin: "mini".into(),
            appimage_script: false,
        },
        recent_boards: 3,
    };
    Fix {
        dir,
        srv,
        keys,
        cfg,
    }
}

fn http() -> Box<dyn Http> {
    http::default_http(true)
}

#[test]
fn konstruktor_und_status_starten_kein_netz() {
    struct Count(Arc<AtomicUsize>);
    impl Http for Count {
        fn get_text(&self, _: &str) -> Result<String, Error> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(Error::Other("x".into()))
        }
        fn download(
            &self,
            _: &str,
            _: &Path,
            _: &AtomicBool,
            _: &mut dyn FnMut(u64, Option<u64>),
        ) -> Result<(), Error> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(Error::Other("x".into()))
        }
    }
    let n = Arc::new(AtomicUsize::new(0));
    let up = Updater::with_http(Arc::new(Count(n.clone())));
    let _ = up.status();
    let _ = Updater::new().status();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        n.load(Ordering::SeqCst),
        0,
        "ohne Klick darf nichts passieren"
    );
    assert!(!up.is_busy());
    // Erst der Aufruf sucht.
    let f = fixture("4.0.0");
    assert!(up.check(f.cfg.clone(), false));
    for _ in 0..100 {
        if !up.is_busy() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(n.load(Ordering::SeqCst), 1);
}

#[test]
fn update_wird_erkannt() {
    let mut f = fixture("4.0.0");
    let h = http();
    let r = check(&f.cfg, &*h).unwrap();
    assert!(r.newer && !r.skipped);
    assert_eq!(r.latest.as_ref().unwrap().version, Version::new(4, 1, 0));
    assert_eq!(r.latest.unwrap().body, "Änderungen 4.1.0");
    f.cfg.skip = Some("4.1.0".into());
    assert!(check(&f.cfg, &*h).unwrap().skipped);
    f.cfg.current = Version::new(4, 1, 0);
    assert!(!check(&f.cfg, &*h).unwrap().newer);
    let l = list_releases(&f.cfg, &*h).unwrap();
    assert_eq!(
        l.iter().map(|r| r.version.to_string()).collect::<Vec<_>>(),
        ["4.1.0", "4.0.0"]
    );
}

#[test]
fn updater_thread_liefert_status() {
    let f = fixture("4.0.0");
    let up = Updater::with_http(Arc::from(http()));
    assert!(up.check(f.cfg.clone(), false));
    while up.is_busy() {
        std::thread::sleep(Duration::from_millis(10));
    }
    let st = up.status();
    assert!(st.check.unwrap().newer);
    assert!(st.error.is_none());
}

#[test]
fn falsche_signatur_wird_abgelehnt() {
    let f = fixture("4.0.0");
    let other = keys();
    let a = f.srv.asset(&app_name("4.2.0"), b"APPIMAGE-4.2.0", &other); // von fremdem Schlüssel signiert
    let rel = github::parse_release(&f.srv.release("4.2.0", a)).unwrap();
    let err = install_release(&f.cfg, &*http(), &rel, &Ctl::default()).unwrap_err();
    assert!(matches!(err, Error::BadSignature(_)), "{err:?}");
    assert_eq!(std::fs::read(f.target()).unwrap(), b"APPIMAGE-4.0.0");
    assert!(!f.dir.path().join("OwnDraw.AppImage.bak").exists());
    assert!(!f
        .cfg
        .work_dir
        .join("v4.2.0")
        .join(app_name("4.2.0"))
        .exists());
}

#[test]
fn fehlende_signatur_wird_abgelehnt() {
    let f = fixture("4.0.0");
    let mut a = f.srv.asset(&app_name("4.2.0"), b"APPIMAGE-4.2.0", &f.keys);
    a.pop(); // .minisig weglassen
    let rel = github::parse_release(&f.srv.release("4.2.0", a)).unwrap();
    assert!(matches!(
        install_release(&f.cfg, &*http(), &rel, &Ctl::default()),
        Err(Error::BadSignature(_))
    ));
    assert_eq!(std::fs::read(f.target()).unwrap(), b"APPIMAGE-4.0.0");
}

#[test]
fn platzhalterschluessel_installiert_nichts() {
    let mut f = fixture("4.0.0");
    f.cfg.pubkey = include_str!("placeholder.pub").to_string();
    assert!(!f.cfg.configured());
    let rel = check(
        &{
            let mut c = f.cfg.clone();
            c.pubkey = f.keys.pk_text.clone();
            c
        },
        &*http(),
    )
    .unwrap()
    .latest
    .unwrap();
    let before = f.srv.hits.load(Ordering::SeqCst);
    assert_eq!(
        install_release(&f.cfg, &*http(), &rel, &Ctl::default()),
        Err(Error::NotConfigured)
    );
    assert_eq!(
        f.srv.hits.load(Ordering::SeqCst),
        before,
        "es darf nichts geladen werden"
    );
    assert_eq!(std::fs::read(f.target()).unwrap(), b"APPIMAGE-4.0.0");
}

#[test]
fn ohne_appimage_verstaendlicher_hinweis() {
    let mut f = fixture("4.0.0");
    f.cfg.target = None;
    let rel = check(&f.cfg, &*http()).unwrap().latest.unwrap();
    let before = f.srv.hits.load(Ordering::SeqCst);
    let e = install_release(&f.cfg, &*http(), &rel, &Ctl::default()).unwrap_err();
    assert!(matches!(e, Error::NoAppImage(_)));
    assert_eq!(f.srv.hits.load(Ordering::SeqCst), before);
}

#[cfg(unix)]
#[test]
fn ohne_schreibrecht_bleibt_alles_heil() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture("4.0.0");
    let ro = f.dir.path().join("ro");
    std::fs::create_dir(&ro).unwrap();
    let t = ro.join("OwnDraw.AppImage");
    std::fs::write(&t, b"APPIMAGE-4.0.0").unwrap();
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
    let r = install::check_writable(&t);
    // Als root gelten Rechte nicht; dann ist der Test nicht aussagekräftig.
    if std::fs::File::create(ro.join("probe")).is_err() {
        assert!(matches!(r, Err(Error::NoAppImage(_))));
    }
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(std::fs::read(&t).unwrap(), b"APPIMAGE-4.0.0");
}

#[test]
fn paket_installation_ersetzt_datei_und_legt_backups_an() {
    let f = fixture("4.0.0");
    let rel = check(&f.cfg, &*http()).unwrap().latest.unwrap();
    let r = install_release(&f.cfg, &*http(), &rel, &Ctl::default()).unwrap();
    assert_eq!(r.version, Version::new(4, 1, 0));
    assert!(r.restart);
    assert_eq!(std::fs::read(f.target()).unwrap(), b"APPIMAGE-4.1.0");
    assert_eq!(
        std::fs::read(f.dir.path().join("OwnDraw.AppImage.bak")).unwrap(),
        b"APPIMAGE-4.0.0"
    );
    let saved = versions::list_saved(&f.cfg.versions_dir);
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].version, Version::new(4, 0, 0));
    assert_eq!(std::fs::read(&saved[0].path).unwrap(), b"APPIMAGE-4.0.0");
    assert!(!f.dir.path().join("OwnDraw.AppImage.new").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(f.target()).unwrap().permissions().mode() & 0o111,
            0o111
        );
    }
}

#[test]
fn abbruch_mitten_im_download_laesst_alte_datei_intakt() {
    let f = fixture("4.0.0");
    let big = vec![7u8; 4 * 1024 * 1024];
    let a = f.srv.asset(&app_name("4.3.0"), &big, &f.keys);
    f.srv
        .slow
        .lock()
        .unwrap()
        .push(format!("/dl/{}", app_name("4.3.0")));
    let rel = github::parse_release(&f.srv.release("4.3.0", a)).unwrap();
    let ctl = Ctl::default();
    let c2 = ctl.clone();
    let watcher = std::thread::spawn(move || {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(20) {
            if c2.status().progress.map(|p| p.0 > 0).unwrap_or(false) {
                c2.cancel.store(true, Ordering::SeqCst);
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    });
    let e = install_release(&f.cfg, &*http(), &rel, &ctl).unwrap_err();
    assert!(watcher.join().unwrap(), "Download hat nie begonnen");
    assert_eq!(e, Error::Cancelled);
    assert_eq!(std::fs::read(f.target()).unwrap(), b"APPIMAGE-4.0.0");
    assert!(!f.dir.path().join("OwnDraw.AppImage.bak").exists());
    assert!(!f.dir.path().join("OwnDraw.AppImage.new").exists());
    assert!(versions::list_saved(&f.cfg.versions_dir).is_empty());
    let part = f
        .cfg
        .work_dir
        .join("v4.3.0")
        .join(format!("{}.part", app_name("4.3.0")));
    assert!(!part.exists());
}

#[test]
fn downgrade_online_und_offline_aus_versions() {
    let f = fixture("4.1.0");
    let h = http();
    let rel = list_releases(&f.cfg, &*h)
        .unwrap()
        .into_iter()
        .find(|r| r.version == Version::new(4, 0, 0))
        .unwrap();
    // Warnung: 4.0.0 liest Format 2, 3.x nicht.
    assert!(versions::format_warning(&rel.version, 2).is_none());
    assert!(versions::format_warning(&Version::new(3, 9, 0), 2).is_some());
    // Online auf 4.0.0.
    install_release(&f.cfg, &*h, &rel, &Ctl::default()).unwrap();
    assert_eq!(std::fs::read(f.target()).unwrap(), b"APPIMAGE-4.0.0");
    let newest = versions::newest_saved(&f.cfg.versions_dir).unwrap();
    assert_eq!(newest.version, Version::new(4, 1, 0));
    // Offline zurück auf 4.1.0 (ohne Server).
    let mut cfg2 = f.cfg.clone();
    cfg2.current = Version::new(4, 0, 0);
    cfg2.api_base = "http://127.0.0.1:1".into();
    install_saved(&cfg2, &newest, &Ctl::default()).unwrap();
    assert_eq!(std::fs::read(f.target()).unwrap(), b"APPIMAGE-4.1.0");
    // Jetzt liegt 4.0.0 in versions/.
    assert!(versions::list_saved(&f.cfg.versions_dir)
        .iter()
        .any(|s| s.version == Version::new(4, 0, 0)));
}

#[test]
fn nur_die_letzten_drei_vorversionen_bleiben() {
    let dir = tempfile::tempdir().unwrap();
    let t = dir.path().join("a.AppImage");
    let new = dir.path().join("new");
    std::fs::write(&new, b"n").unwrap();
    let vd = dir.path().join("v");
    for i in 0..5 {
        std::fs::write(&t, format!("alt{i}")).unwrap();
        install::install_appimage(
            &new,
            &Version::new(9, 0, 0),
            &t,
            &Version::new(4, i, 0),
            &vd,
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
    let s = versions::list_saved(&vd);
    assert_eq!(
        s.iter().map(|s| s.version.to_string()).collect::<Vec<_>>(),
        ["4.4.0", "4.3.0", "4.2.0"]
    );
}

/// Mini-Cargo-Projekt als Tarball (statt OwnDraw); `extra` = zusätzlicher Inhalt für build.rs.
fn mini_tarball(build_rs: Option<&str>) -> Vec<u8> {
    let mut files: Vec<(&str, String)> = vec![
        (
            "Cargo.toml",
            "[package]\nname = \"mini\"\nversion = \"0.1.0\"\nedition = \"2021\"\n".into(),
        ),
        (
            "Cargo.lock",
            "version = 4\n\n[[package]]\nname = \"mini\"\nversion = \"0.1.0\"\n".into(),
        ),
        (
            "src/main.rs",
            "fn main() { println!(\"mini-ok\"); }\n".into(),
        ),
    ];
    if let Some(b) = build_rs {
        files.push(("build.rs", b.to_string()));
    }
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut tar = tar::Builder::new(gz);
    for (name, data) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        tar.append_data(&mut h, format!("OwnDraw-4.1.0/{name}"), data.as_bytes())
            .unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap()
}

#[test]
fn quellbau_mit_mini_projekt() {
    let mut f = fixture("4.0.0");
    f.cfg.mode = UpdateMode::Source;
    let a = f.srv.asset(
        &source_asset(&Version::new(4, 1, 0)),
        &mini_tarball(None),
        &f.keys,
    );
    let rel = github::parse_release(&f.srv.release("4.1.0", a)).unwrap();
    let ctl = Ctl::default();
    install_release(&f.cfg, &*http(), &rel, &ctl).unwrap();
    assert!(f.cfg.build_dir.join("v4.1.0").exists());
    let out = std::process::Command::new(f.target()).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "mini-ok");
    assert_eq!(
        std::fs::read(f.dir.path().join("OwnDraw.AppImage.bak")).unwrap(),
        b"APPIMAGE-4.0.0"
    );
    assert!(ctl
        .status()
        .log
        .iter()
        .any(|l| l.contains("cargo build --release --locked")));
}

#[test]
fn quellbau_ohne_toolchain_meldet_hinweis() {
    let mut f = fixture("4.0.0");
    f.cfg.mode = UpdateMode::Source;
    f.cfg.cargo = Some(PathBuf::from("/nicht/vorhanden/cargo"));
    // `cargo` explizit gesetzt, aber nicht startbar → Bauen schlägt fehl; ohne Angabe und ohne cargo im PATH → NoToolchain.
    let a = f.srv.asset(
        &source_asset(&Version::new(4, 1, 0)),
        &mini_tarball(None),
        &f.keys,
    );
    let rel = github::parse_release(&f.srv.release("4.1.0", a)).unwrap();
    assert!(install_release(&f.cfg, &*http(), &rel, &Ctl::default()).is_err());
    assert_eq!(std::fs::read(f.target()).unwrap(), b"APPIMAGE-4.0.0");
}

#[test]
fn abbruch_beim_bauen_toetet_den_prozess() {
    let mut f = fixture("4.0.0");
    f.cfg.mode = UpdateMode::Source;
    let tar = mini_tarball(Some("fn main() { println!(\"cargo:warning=baue\"); std::thread::sleep(std::time::Duration::from_secs(60)); }\n"));
    let a = f
        .srv
        .asset(&source_asset(&Version::new(4, 1, 0)), &tar, &f.keys);
    let rel = github::parse_release(&f.srv.release("4.1.0", a)).unwrap();
    let ctl = Ctl::default();
    let c2 = ctl.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(4));
        c2.cancel.store(true, Ordering::SeqCst);
    });
    let t = Instant::now();
    let e = install_release(&f.cfg, &*http(), &rel, &ctl).unwrap_err();
    assert_eq!(e, Error::Cancelled);
    assert!(t.elapsed() < Duration::from_secs(40));
    assert_eq!(std::fs::read(f.target()).unwrap(), b"APPIMAGE-4.0.0");
}
