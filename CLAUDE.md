# Hinweise für Claude

Lies `CONTRIBUTING.md`. Wichtigste Regel: **Jedes neue Editor-Feature bekommt im selben PR einen Eintrag in
`crates/owndraw/tutorial.json`** (Anleitung & Tipps); `cargo test -p owndraw --lib tutorial` prüft das.
Version anheben + `cargo update -w --offline`, kein `cargo fmt`.
