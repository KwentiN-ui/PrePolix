# prepolix

Ein FEM-Präprozessor und Postprozessor für [CalculiX](http://www.calculix.de/), geschrieben in Rust und lauffähig unter Linux und Windows.
Vorbild für Bedienung und Funktionsumfang ist [PrePoMax](https://prepomax.fs.um.si/).

> Status: frühes Grundgerüst (Meilenstein M0). Plan und Roadmap stehen in [docs/PLANUNG_TECH_STACK.md](docs/PLANUNG_TECH_STACK.md).

## Bauen und starten

Voraussetzung ist ein aktuelles stabiles Rust (`rustup`).

```sh
cargo run --release
```

Unter Linux braucht das Programm zur Laufzeit `libxkbcommon-x11` (X11) bzw. `libxkbcommon` (Wayland) sowie einen Vulkan- oder OpenGL-Treiber. Die meisten Desktop-Installationen bringen das bereits mit.

## Bedienung der 3D-Ansicht

| Aktion | Maus |
|---|---|
| Drehen | linke Taste ziehen |
| Verschieben | rechte oder mittlere Taste ziehen |
| Zoomen | Mausrad |
| Einpassen | Doppelklick |

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
