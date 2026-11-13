# Architektur (Rust, ab 3.0)

## Überblick

```
owndraw (Desktop, egui/eframe)  ──►  owndraw-core (keine UI-Abhängigkeit)
  main.rs   Start, Datenordner, Fenster      board.rs       Board: Objekte, Ebenen, Werkzeuge, Eingabe, Undo, Rendering
  app.rs    Übersicht, Dialoge, Dateidialog  item.rs        Stroke / Shape / Note / Img, Geometrie, JSON
  editor.rs Editor, Leisten, Tasten, Eingabe recognizer.rs  Formerkennung
  layers.rs Ebenen-Panel                     layer.rs       Ebenen + Animationen
  style.rs  Farben, Knöpfe, Icon-Texturen    canvas.rs      Canvas-Schicht auf tiny-skia (save/restore, Transformation)
  prefs.rs  Einstellungen (prefs.txt)        text.rs        Schrift (Ubuntu Light + Noto Emoji), Umbruch
                                             icons.rs       Vektor-Icons
                                             store.rs       Boards auf Platte, .owndraw-Import/Export
                                             export.rs      PDF, Ebenen-PNG, Website-ZIP
```

## Kern

- **Koordinaten:** Board in logischen Pixeln; `set_size(w, h, px_scale)` übernimmt die Pixeldichte der Oberfläche.
  Ansicht über `sc`, `tx`, `ty` (Zoom/Verschiebung).
- **Objekte:** `Vec<Rc<Item>>`; Änderungen per Copy-on-Write (`Rc::make_mut`). Undo-Schnappschüsse teilen
  unveränderte Objekte. Auswahl über Laufzeit-ids (`u64`, nicht gespeichert).
- **Rendering:** `render(&mut Pixmap)` zeichnet in ein vormultipliziertes tiny-skia-Pixmap, das die Oberfläche als
  Textur hochlädt. Hintergrund, Muster und ruhende Objekte liegen in einem Zwischenspeicher; gerade bewegte
  Objekte (`lift`) und der aktuelle Strich werden obenauf gezeichnet.
- **Eingabe:** plattformneutral über `Motion` (Finger, Stylus, Radierer, Maus; Mehrfinger, Historie). Zeitgesteuerte
  Gesten (Halten für Formerkennung 0,45 s, Ende von Rad-/Pan-Gesten) über `tick(now)` und `next_deadline()`.
- **Rückmeldungen** an die Oberfläche über `board.events`: `Changed`, `AskText`, `Notice`, `Haptic`.
- **Formerkennung:** zwei Abtastungen des Strichs (geglättet für die Topologie, ungeglättet für die Passung),
  Modelle Linie/Pfeil/Doppelpfeil/Ellipse/Polygon/Stern/Pentagramm/Herz, Auswahl über normiertes Residuum.
  Interne Fehler führen zu „keine Form“ (`catch_unwind`).
- **Speichern:** Board-JSON wie Java 2.x (Felder `t/c/w/h/v/sh/p/k/q/an/x/y/nw/mh/s/bg/tx/iw/ih/f/ly`, Farben als
  vorzeichenbehaftetes int). Schreiben atomar (`write_atomic`). Bilder unter `img/`, verkleinert auf ≤ 2048 px.

## Desktop

- egui 0.37 / eframe (glow, X11 + Wayland). Alle Leisten sind `Area`s über der Board-Textur.
- Maus: linke Taste wird als Stylus (Druck 0,5) an den Kern gegeben, rechte/mittlere Taste und Leertaste
  verschieben direkt; Touch wird auf Finger-Ereignisse abgebildet.
- Autosave alle 30 s im Hintergrund-Thread, Speichern beim Schließen.
- Dateidialoge: egui-file-dialog; Vorschau der Website über `open`.

## AppImage

`build-appimage.sh` baut Release, füllt `packaging/appimage/appdir/` und packt mit eigener Runtime
(`packaging/appimage/runtime.c`, squashfs + statische zlib). Ergebnis: eine Datei ohne Java, ca. 7 MB.
