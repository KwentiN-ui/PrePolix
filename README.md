# prepolix

Ein FEM-Präprozessor und Postprozessor für [CalculiX](http://www.calculix.de/), geschrieben in Rust und lauffähig unter Linux und Windows.
Vorbild für Bedienung und Funktionsumfang ist [PrePoMax](https://prepomax.fs.um.si/).

> Status: Meilenstein M1, CalculiX-Netze (`.inp`) lassen sich öffnen und ansehen. Plan und Roadmap stehen in [docs/PLANUNG_TECH_STACK.md](docs/PLANUNG_TECH_STACK.md).

## Bauen und starten

Voraussetzung ist ein aktuelles stabiles Rust (`rustup`).

```sh
cargo run --release
```

Ein Modell öffnest du über *Datei → Öffnen* (Strg+O), indem du eine `.inp`-Datei ins Fenster ziehst oder sie beim Start angibst:

```sh
cargo run --release -- testdata/wuerfel_c3d10.inp
```

Unter Linux braucht das Programm zur Laufzeit `libxkbcommon-x11` (X11) bzw. `libxkbcommon` (Wayland) sowie einen Vulkan- oder OpenGL-Treiber. Die meisten Desktop-Installationen bringen das bereits mit.

## Bedienung der 3D-Ansicht

| Aktion | Maus |
|---|---|
| Drehen | linke Taste ziehen |
| Verschieben | rechte oder mittlere Taste ziehen |
| Zoomen | Mausrad |
| Einpassen | Doppelklick |

## Was schon geht

- `.inp`-Netze lesen: Knoten, Elemente (C3D4/6/8/10/15/20 inkl. R/I-Varianten, S3/4/6/8, M3D, CPS/CPE/CAX, B31/B32, T3D2/T3D3), `*NSET`, `*ELSET` (auch `GENERATE`), `*SURFACE`, `*INCLUDE`. Alle anderen Keywords werden übersprungen und in der Ausgabe aufgezählt.
- Parts entstehen aus dem `ELSET=` der `*ELEMENT`-Blöcke und lassen sich im Modellbaum ein- und ausblenden.
- Darstellung der Außenhaut mit Feature-Kanten und optionalen Netzkanten, auch für quadratische Elemente.

Beispielmodelle liegen in [testdata/](testdata/), erzeugt von `testdata/erzeugen.py`. Die Modelle mit Step lassen sich direkt mit `ccx` rechnen.

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
