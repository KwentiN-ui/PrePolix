# PrePolix

Ein FEM-Präprozessor und Postprozessor für [CalculiX](http://www.calculix.de/), geschrieben in Rust und lauffähig unter Linux und Windows.
Vorbild für Bedienung und Funktionsumfang ist [PrePoMax](https://prepomax.fs.um.si/).

> Status: Meilenstein M2, CalculiX-Netze (`.inp`) und -Ergebnisse (`.frd`) lassen sich öffnen und ansehen. Plan und Roadmap stehen in [docs/PLANUNG_TECH_STACK.md](docs/PLANUNG_TECH_STACK.md).

## Bauen und starten

Voraussetzung ist ein aktuelles stabiles Rust (`rustup`).

```sh
cargo run --release
```

Ein Modell oder eine Ergebnisdatei öffnest du über *Datei → Öffnen* (Strg+O), indem du eine `.inp`- oder `.frd`-Datei ins Fenster ziehst oder sie beim Start angibst:

```sh
cargo run --release -- testdata/wuerfel_c3d10.inp
cargo run --release -- testdata/kragbalken_c3d8.frd
```

Unter Linux braucht das Programm zur Laufzeit `libxkbcommon-x11` (X11) bzw. `libxkbcommon` (Wayland) sowie einen Vulkan- oder OpenGL-Treiber. Die meisten Desktop-Installationen bringen das bereits mit.

## Bedienung der 3D-Ansicht

Wie in PrePoMax:

| Aktion | Maus |
|---|---|
| Drehen | mittlere Taste ziehen |
| Verschieben | Umschalt + mittlere Taste ziehen |
| Zoomen | Mausrad (mit Strg feiner) oder Strg + mittlere Taste ziehen |
| Auswählen, Auswahlrahmen | linke Taste (in Auswahldialogen) |
| Kontextmenü | rechte Taste |
| Einpassen | Doppelklick |

Die Standardansichten folgen PrePoMax: Y zeigt nach oben. "Vertikal" stellt die Achse senkrecht, die der Bildschirm-Hochrichtung am nächsten liegt, "Achse senkrecht" (Menü Ansicht oder Rechtsklick) stellt X, Y oder Z nach oben und schaut senkrecht auf die nächstgelegene Koordinatenebene, und "Isometrisch, Achse oben" zeigt die isometrische Ansicht mit X, Y oder Z nach oben.

## Was schon geht

- `.inp`-Netze lesen: Knoten, Elemente (C3D4/6/8/10/15/20 inkl. R/I-Varianten, S3/4/6/8, M3D, CPS/CPE/CAX, B31/B32, T3D2/T3D3), `*NSET`, `*ELSET` (auch `GENERATE`), `*SURFACE`, `*INCLUDE`. Alle anderen Keywords werden übersprungen und in der Ausgabe aufgezählt.
- Parts entstehen aus dem `ELSET=` der `*ELEMENT`-Blöcke und lassen sich im Modellbaum ein- und ausblenden.
- Darstellung der Außenhaut mit Feature-Kanten und optionalen Netzkanten, auch für quadratische Elemente.
- `.frd`-Ergebnisse lesen (ASCII und binär): Netz, Materialien als Parts, alle Steps und Inkremente inklusive Eigenformen. Komponenten heißen wie in PrePoMax (`U1`, `S11`, …); ergänzt werden Verschiebungsbetrag `ALL`, `MISES`, `TRESCA`, Hauptspannungen und Vergleichsdehnung.
- Ergebnisanzeige wie in PrePoMax: Konturplot in 9 Farbstufen (Regenbogen, einstellbar), Legende, Min/Max mit Knoten, verformte Darstellung mit automatischem, echtem oder eigenem Faktor.
- Schnittansicht (*Ansicht → Schnittansicht …*): Schnittebene als Grundebene XY/YZ/XZ an einer Koordinate oder als Punkt und Normale, beides auch per Klick ins Modell (Normale aus zwei Punkten). Im 3D-Fenster lässt sich die Ebene am Pfeil verschieben und an den Bögen kippen. Volumenelemente zeigen ihre Schnittfläche mit Netzkanten und Ergebnisfarben.

Beispielmodelle liegen in [testdata/](testdata/), erzeugt von `testdata/erzeugen.py`. Die Modelle mit Step lassen sich direkt mit `ccx` rechnen; zwei Ergebnisdateien (`*.frd`) liegen bei.

## Aufbau

| Crate | Inhalt |
|---|---|
| `plx-core` | Einheiten, Formeln, Selektions-Rezepte, gemeinsame Typen |
| `plx-mesh` | FE-Netz: Knoten, Elemente, Sets, Surfaces, Geometrie-Topologie |
| `plx-model` | FE-Modell: Materialien, Sections, Steps, Randbedingungen, Lasten, Commands |
| `plx-io` | Dateiformate: CalculiX `.inp`/`.frd`/`.dat` und weitere |
| `plx-results` | Ergebnis-Datenmodell und abgeleitete Größen |
| `plx-mesher` | Anbindung externer Vernetzer (Gmsh) |
| `plx-job` | Start und Überwachung von CalculiX |
| `plx-render` | wgpu-Renderer und Kamera |
| `plx-app` | Oberfläche (egui), ausführbares Programm `prepolix` |

Die unteren Crates kennen keine Oberfläche und lassen sich einzeln testen (`cargo test`).

## Lizenz

GPL-3.0-or-later, siehe [LICENSE](LICENSE).

Die mitgelieferte Schrift Noto Sans (`crates/plx-app/assets/fonts`, auf Latein, Griechisch und
gängige Symbole reduziert) steht unter der SIL Open Font License 1.1, siehe
[OFL.txt](crates/plx-app/assets/fonts/OFL.txt).
