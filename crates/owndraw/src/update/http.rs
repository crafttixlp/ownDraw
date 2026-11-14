//! Minimaler HTTP-Zugriff. Desktop: `ureq` (rustls); Android: Java-Seite (`HttpURLConnection`), weil die
//! TLS-Bibliothek dort eine C-Toolchain bräuchte. Beide erzwingen HTTPS (Tests dürfen `allow_http` setzen).

use super::Error;
use std::path::Path;
use std::sync::atomic::AtomicBool;

pub trait Http: Send + Sync {
    /// Lädt eine Textantwort (GitHub-API).
    fn get_text(&self, url: &str) -> Result<String, Error>;
    /// Lädt nach `dest` (wird überschrieben). `progress(bytes, gesamt)`; bei gesetztem `cancel` wird mit
    /// `Error::Cancelled` abgebrochen und `dest` gelöscht.
    fn download(&self, url: &str, dest: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), Error>;
}

/// Standard-Implementierung der Plattform. `allow_http` nur für Tests mit lokalem Server.
pub fn default_http(allow_http: bool) -> Box<dyn Http> {
    #[cfg(not(target_os = "android"))]
    {
        Box::new(desktop::Ureq { allow_http })
    }
    #[cfg(target_os = "android")]
    {
        Box::new(android::JavaHttp { allow_http })
    }
}

#[cfg(not(target_os = "android"))]
mod desktop {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    pub struct Ureq {
        pub allow_http: bool,
    }

    impl Ureq {
        fn agent(&self) -> ureq::Agent {
            ureq::Agent::config_builder()
                .https_only(!self.allow_http)
                .timeout_connect(Some(Duration::from_secs(20)))
                .timeout_recv_response(Some(Duration::from_secs(30)))
                .timeout_recv_body(Some(Duration::from_secs(30)))
                .user_agent("OwnDraw-Updater")
                .build()
                .into()
        }
    }

    fn net(e: ureq::Error) -> Error {
        Error::Other(format!("Netzwerkfehler: {e}"))
    }

    impl Http for Ureq {
        fn get_text(&self, url: &str) -> Result<String, Error> {
            let mut r = self.agent().get(url).header("Accept", "application/vnd.github+json").call().map_err(net)?;
            r.body_mut().with_config().limit(8 * 1024 * 1024).read_to_string().map_err(net)
        }

        fn download(&self, url: &str, dest: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), Error> {
            let mut r = self.agent().get(url).call().map_err(net)?;
            let total = r.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|s| s.parse::<u64>().ok());
            let mut body = r.body_mut().as_reader();
            let res = (|| -> Result<(), Error> {
                let mut f = std::fs::File::create(dest)?;
                let mut buf = vec![0u8; 64 * 1024];
                let mut got = 0u64;
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(Error::Cancelled);
                    }
                    let n = body.read(&mut buf).map_err(|e| Error::Other(format!("Download unterbrochen: {e}")))?;
                    if n == 0 {
                        break;
                    }
                    f.write_all(&buf[..n])?;
                    got += n as u64;
                    progress(got, total);
                }
                f.sync_all()?;
                if let Some(t) = total {
                    if t != got {
                        return Err(Error::Other(format!("Download unvollständig ({got} von {t} Bytes)")));
                    }
                }
                Ok(())
            })();
            if res.is_err() {
                let _ = std::fs::remove_file(dest);
            }
            res
        }
    }
}

#[cfg(target_os = "android")]
mod android {
    use super::*;
    use std::sync::atomic::Ordering;

    pub struct JavaHttp {
        pub allow_http: bool,
    }

    impl Http for JavaHttp {
        fn get_text(&self, url: &str) -> Result<String, Error> {
            if !self.allow_http && !url.starts_with("https://") {
                return Err(Error::Other("Nur HTTPS erlaubt".into()));
            }
            crate::platform::android::http_get(url).map_err(|e| Error::Other(format!("Netzwerkfehler: {e}")))
        }

        fn download(&self, url: &str, dest: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), Error> {
            if !self.allow_http && !url.starts_with("https://") {
                return Err(Error::Other("Nur HTTPS erlaubt".into()));
            }
            let r = crate::platform::android::http_download(url, dest, &|| cancel.load(Ordering::Relaxed), progress);
            if r.is_err() {
                let _ = std::fs::remove_file(dest);
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            r.map_err(|e| Error::Other(format!("Download fehlgeschlagen: {e}")))
        }
    }
}
