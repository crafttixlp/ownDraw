//! GitHub-Releases-API: Liste und neuestes Release.

use super::http::Http;
use super::versions::parse_tag;
use super::Error;
use semver::Version;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub tag: String,
    pub version: Version,
    pub title: String,
    /// Änderungstext (Markdown, wie bei GitHub).
    pub body: String,
    /// Veröffentlichungsdatum `JJJJ-MM-TT`.
    pub date: String,
    pub prerelease: bool,
    pub assets: Vec<Asset>,
}

impl Release {
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Ein Release aus dem API-JSON; Entwürfe und Tags ohne semver ergeben `None`.
pub fn parse_release(v: &Value) -> Option<Release> {
    if v.get("draft").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let tag = s(v, "tag_name");
    let version = parse_tag(&tag)?;
    let assets = v
        .get("assets")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|x| Asset { name: s(x, "name"), url: s(x, "browser_download_url"), size: x.get("size").and_then(Value::as_u64).unwrap_or(0) })
                .collect()
        })
        .unwrap_or_default();
    let date = s(v, "published_at");
    Some(Release {
        title: s(v, "name"),
        body: s(v, "body"),
        date: date.chars().take(10).collect(),
        prerelease: v.get("prerelease").and_then(Value::as_bool).unwrap_or(false),
        tag,
        version,
        assets,
    })
}

pub struct Api<'a> {
    pub http: &'a dyn Http,
    /// z. B. `https://api.github.com` (Tests: lokaler Server).
    pub base: &'a str,
    pub repo: &'a str,
}

impl Api<'_> {
    fn url(&self, tail: &str) -> Result<String, Error> {
        if self.repo.split('/').count() != 2 || self.repo.contains(char::is_whitespace) || self.repo.contains("..") {
            return Err(Error::Other(format!("Ungültiges Repository „{}“ (erwartet owner/name)", self.repo)));
        }
        Ok(format!("{}/repos/{}/releases{}", self.base.trim_end_matches('/'), self.repo, tail))
    }

    /// `/releases/latest`; `None`, wenn es (noch) kein Release gibt. Bei 404 wird geprüft, ob das Repository
    /// überhaupt existiert (sonst wäre „kein Release“ irreführend).
    pub fn latest(&self) -> Result<Option<Release>, Error> {
        let t = match self.http.get_text(&self.url("/latest")?) {
            Ok(t) => t,
            Err(Error::Status(404)) => {
                let repo_url = format!("{}/repos/{}", self.base.trim_end_matches('/'), self.repo);
                return match self.http.get_text(&repo_url) {
                    Ok(_) => Ok(None),
                    Err(Error::Status(404)) => Err(Error::Other(format!("Das Repository „{}“ wurde auf GitHub nicht gefunden (oder ist nicht öffentlich).", self.repo))),
                    Err(e) => Err(e),
                };
            }
            Err(e) => return Err(e),
        };
        let v: Value = serde_json::from_str(&t).map_err(|e| Error::Other(format!("Antwort von GitHub unlesbar: {e}")))?;
        Ok(parse_release(&v))
    }

    /// `/releases` (Liste), neueste Version zuerst.
    pub fn list(&self) -> Result<Vec<Release>, Error> {
        let t = self.http.get_text(&self.url("?per_page=50")?)?;
        let v: Value = serde_json::from_str(&t).map_err(|e| Error::Other(format!("Antwort von GitHub unlesbar: {e}")))?;
        let mut r: Vec<Release> = v.as_array().map(|a| a.iter().filter_map(parse_release).collect()).unwrap_or_default();
        r.sort_by(|a, b| b.version.cmp(&a.version));
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parst_release() {
        let v: Value = serde_json::json!({"tag_name":"v4.1.0","name":"Neu","body":"- x","published_at":"2026-03-04T10:00:00Z","draft":false,"prerelease":false,
            "assets":[{"name":"a","browser_download_url":"https://x/a","size":3}]});
        let r = parse_release(&v).unwrap();
        assert_eq!(r.version, Version::new(4, 1, 0));
        assert_eq!(r.date, "2026-03-04");
        assert_eq!(r.assets.len(), 1);
        assert!(parse_release(&serde_json::json!({"tag_name":"nightly"})).is_none());
        assert!(parse_release(&serde_json::json!({"tag_name":"v1.0.0","draft":true})).is_none());
    }

    #[test]
    fn fehler_werden_verstaendlich() {
        use super::super::http::classify;
        assert_eq!(classify("http status: 403"), Error::RateLimit);
        assert_eq!(classify("java.io.IOException: http status: 404"), Error::Status(404));
        assert_eq!(classify("java.net.UnknownHostException: Unable to resolve host \"api.github.com\""), Error::Offline);
        assert_eq!(classify("io: Connection refused (os error 111)"), Error::Offline);
        assert_eq!(classify("java.net.SocketTimeoutException: timeout"), Error::Timeout);
        assert!(matches!(classify("seltsam"), Error::Other(_)));
        assert!(Error::Offline.to_string().starts_with("Offline"));
    }
}
