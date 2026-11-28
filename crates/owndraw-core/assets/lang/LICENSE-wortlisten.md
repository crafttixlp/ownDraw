# Lizenzhinweise der eingebetteten Sprachdaten

Diese Dateien werden von `crates/owndraw-core/src/office/lang.rs` per `include_bytes!` eingebettet
(Rechtschreibprüfung, Silbentrennung). Der Quellcode von OwnDraw steht unter Apache-2.0; die Daten
unten behalten ihre eigenen Lizenzen.

## Wortlisten: `de.txt.gz`, `en.txt.gz`

- Quelle: FrequencyWords von Hermit Dave, https://github.com/hermitdave/FrequencyWords
  (`content/2018/de/de_full.txt`, `content/2018/en/en_full.txt`), abgeleitet aus OpenSubtitles 2018.
- Lizenz der Daten: Creative Commons Attribution-ShareAlike 4.0 (CC-BY-SA-4.0),
  https://creativecommons.org/licenses/by-sa/4.0/
- Bearbeitung durch OwnDraw: nur Wörter aus Buchstaben (Apostroph/Bindestrich innen), kleingeschrieben,
  Mindesthäufigkeit (de >= 15, en >= 30), Einzelbuchstaben und Wiederholungs-Müll entfernt, nach
  Häufigkeit sortiert (Zählwerte entfernt), gzip-komprimiert. Bekannte Tippfehler der Autokorrektur-Liste
  werden beim Laden ausgeblendet.
- Hinweis: Die abgeleiteten Wortlisten bleiben unter CC-BY-SA-4.0 (Namensnennung, Weitergabe unter
  gleichen Bedingungen). Der Programmcode wird dadurch nicht verändert.

## Trennmuster: `hyph-de.pat.gz`, `hyph-en.pat.gz`

Quelle: hyph-utf8 / tex-hyphen, https://github.com/hyphenation/tex-hyphen (CTAN-Paket `hyph-utf8`),
Dateien `patterns/txt/hyph-de-1996.pat.txt` und `patterns/txt/hyph-en-us.pat.txt`, nur gzip-komprimiert.

- Deutsch (`hyph-de-1996`): Copyright (c) 2013-2024 Stephan Hennig, Werner Lemberg, Günter Milde,
  Sander van Geloven, Georg Pfeiffer, Gisbert W. Selke, Tobias Wendorf, Keno Wehr
  (Deutschsprachige Trennmustermannschaft). Lizenz: MIT.

  > Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
  > associated documentation files (the "Software"), to deal in the Software without restriction, including
  > without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
  > copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the
  > following conditions: The above copyright notice and this permission notice shall be included in all
  > copies or substantial portions of the Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
  > KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
  > PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR
  > ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
  > OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

- Englisch (`hyph-en-us`): Copyright (C) 1990, 2004, 2005 Gerard D.C. Kuiken. Lizenz:

  > Copying and distribution of this file, with or without modification, are permitted in any medium
  > without royalty provided the copyright notice and this notice are preserved.
