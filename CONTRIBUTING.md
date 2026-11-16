# Mitarbeit an OwnDraw

## Regel: Neues Editor-Feature ⇒ Eintrag in der Anleitung

Die App hat eine Seite **„Anleitung & Tipps“** (Startseite → „Anleitung“, Editor → „⋯“ → „Anleitung & Tipps“,
Taste F1, Web: `/tutorial`). Sie erklärt alle Editor-Funktionen und sammelt Tipps & Tricks.

**Jede neue oder geänderte Editor-Funktion muss im selben PR dort eingetragen werden.**

- Inhalt: eine Datei, [`crates/owndraw/tutorial.json`](crates/owndraw/tutorial.json) (Kategorien → Einträge mit
  `title`, `text`, optional `how`, `keys`, `only`, `covers`). Die Seite selbst: `crates/owndraw/src/ui/tutorial.rs`.
- `covers` verknüpft einen Eintrag mit dem Code:
  - `tool:<Name>` – Werkzeug der Leiste (`Tool::label`)
  - `key:<egui-Key>` – jede `Key::…` im Editor (`ui/editor/*.rs`)
  - `menu:<Beschriftung>` – jede Zeile im „⋯“-Menü (`ui/editor/menu.rs`)
  - `setting:<Beschriftung>` – jede Zeile in den Einstellungen „Werkzeugleisten“ und „Einrasten“
  - `extra:<frei>` – Funktionen ohne prüfbares Gegenstück (Gesten, Collab, …)
- **CI-Check:** `cargo test` führt `ui::tutorial::tests::tutorial_deckt_editor_ab` aus. Der Test schlägt fehl, wenn ein
  Werkzeug, Tastenkürzel, Menüeintrag oder eine Editor-Einstellung keinen Eintrag hat, und nennt die fehlenden
  Kennungen. `tutorial_ohne_veraltete_eintraege` meldet umgekehrt Kennungen, die es im Code nicht mehr gibt.
- Funktionen, die der Test nicht sieht (neue Geste, neue Option im Popover, Collab …), trotzdem eintragen
  (`extra:`), ggf. auch einen Tipp in „Tipps & Tricks“.

## Weitere Regeln

- Version in `Cargo.toml` anheben **und** `Cargo.lock` aktualisieren: `cargo update -w --offline`
  (sonst scheitern CI/Release mit `--locked`). Version anheben + Merge = Release.
- Kein `cargo fmt` (das Repo ist nicht rustfmt-formatiert).
- Oberflächenänderungen im Browser testen (`./build-web.sh`), Screenshots beilegen.
