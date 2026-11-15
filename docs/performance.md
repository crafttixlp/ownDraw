# Leistung (ab 4.2)

Messung: `cargo run --release -p owndraw-core --example perf` (ohne Oberfläche; Bild je Ereignis, Anteil der Fläche,
die zur GPU muss).

## Aufbau

- **Strich-Rasterer** (`Canvas::fill_capsules`): Freihand-Striche sind Ketten von Kapseln (Segment + Radius aus dem
  Druck). Deckung je Pixel über ein Abstandsfeld, Vereinigung per Maximum (keine doppelte Deckung an Gelenken, auch
  beim halbtransparenten Marker). Ersetzt eine tiny-skia-Linie je Segment; Formen, Notizen, Bilder bleiben tiny-skia.
  Bei Drehung/Scherung im Canvas gilt der alte Weg.
- **Zwischenspeicher mit Teil-Updates:** Der Kern merkt sich, mit welchen Objekten (`Rc`-Zeiger) der Zwischenspeicher
  gezeichnet wurde. Änderungen erzeugen per Copy-on-Write neue Objekte; nur deren Bereiche werden neu gezeichnet
  (Radieren, Rückgängig, Löschen, Verschieben der Auswahl …). Ebenen, Theme, Muster, Seiten und Zoom stecken in einer
  Signatur; ändert sie sich, wird alles neu gezeichnet.
- **Dirty-Rects:** `Frame::damage` nennt die geänderten Bereiche; die Oberfläche lädt nur diese hoch
  (`TextureHandle::set_partial`). Beim Zeichnen sind das ~5 % der Fläche statt 100 %.
- **Ring-Speicher beim Verschieben:** Zwischenspeicher und Textur sind ein Ring (Ursprung `Frame::org`). Verschieben
  ändert nur den Ursprung; gezeichnet und hochgeladen werden nur die frei gewordenen Streifen. Die Oberfläche zeichnet
  die Textur in bis zu vier Stücken (`torus_parts`). In Ruhe rastet die Ansicht auf das Pixelraster ein (< 0,5 px).
- **Zoom auf der GPU:** Während Mausrad-/Pinch-Zoom zeigt die Oberfläche die vorhandene Textur skaliert
  (`Frame::scaled`), die CPU zeichnet nichts. Erst danach wird einmal scharf gezeichnet.
- **Parallel:** Große Flächen werden aufgezeichnet (`Canvas::start_recording`) und in waagrechten Streifen auf allen
  Kernen abgespielt (`Recording::replay_parallel`). Objekte selbst bleiben einthreadig (`Rc`).
- **Autosave:** Im UI-Thread nur Board-JSON und Vorschaubild; PNG, Serialisieren und Packen im Schreib-Thread.
- **Seitenliste:** Vorschaubilder nur für Seiten, deren Inhalt sich geändert hat (`Board::content_key`).
- **Release-Profil:** `lto = "fat"`, `codegen-units = 1`.

Test `incremental_render_matches_full` prüft, dass das schrittweise aufgebaute Bild (Teil-Updates, Ring-Speicher,
Dirty-Rects) dem komplett neu gezeichneten entspricht.
