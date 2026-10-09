# PrePolix

Ein FEM-Präprozessor und Postprozessor für [CalculiX](http://www.calculix.de/), geschrieben in Rust und lauffähig unter Linux und Windows.
Vorbild für Bedienung und Funktionsumfang ist [PrePoMax](https://prepomax.fs.um.si/).

> Status: Meilenstein M2, CalculiX-Netze (`.inp`) und -Ergebnisse (`.frd`) lassen sich öffnen und ansehen. Plan und Roadmap stehen in [docs/PLANUNG_TECH_STACK.md](docs/PLANUNG_TECH_STACK.md).

## Bauen und starten

Voraussetzung ist ein aktuelles stabiles Rust (`rustup`). Unter Linux braucht der Build außerdem die ALSA-Entwicklungsdateien für die Audioausgabe (Debian/Ubuntu: `sudo apt install libasound2-dev`, Fedora: `alsa-lib-devel`).

```sh
cargo run --release
```

Ein Modell oder eine Ergebnisdatei öffnest du über *Datei → Öffnen* (Strg+O), indem du eine `.inp`- oder `.frd`-Datei ins Fenster ziehst oder sie beim Start angibst:

```sh
cargo run --release -- testdata/wuerfel_c3d10.inp
cargo run --release -- testdata/kragbalken_c3d8.frd
```

### Gmsh für Geometrie-Import und Vernetzung

STEP-, IGES- und BREP-Dateien importiert und vernetzt prepolix mit [Gmsh](https://gmsh.info/), das OpenCASCADE, Netgen und TetGen mitbringt. Gmsh wird nicht mitkompiliert, sondern zur Laufzeit als Bibliothek geladen; ohne sie startet prepolix normal, nur Import und Vernetzung fehlen. Für die Entwicklung holt ein Skript die offizielle Bibliothek (Gmsh 4.15, aus dem PyPI-Paket von Gmsh) nach `target/gmsh`, wo Debug-Builds und Tests sie finden:

```sh
python3 scripts/fetch_gmsh.py                        # Linux
python3 scripts/fetch_gmsh.py --platform windows     # Windows
```

Für eine Weitergabe gehören `libgmsh.so.4.15` bzw. `gmsh-4.15.dll` und `GMSH-LICENSE.txt` neben die ausführbare Datei. Alternativ lässt sich unter *Werkzeuge → Einstellungen → Gmsh* der Pfad zu einer vorhandenen Bibliothek angeben und testen. Unter Linux braucht die Gmsh-Bibliothek zusätzlich `libGLU` und `libXft`.

```sh
cargo run --release -- testdata/platte_mit_loch.step
```

Unter Linux braucht das Programm zur Laufzeit `libxkbcommon-x11` (X11) bzw. `libxkbcommon` (Wayland) sowie einen Vulkan- oder OpenGL-Treiber. Die meisten Desktop-Installationen bringen das bereits mit.

### Windows-Installer

Der Installer (`prepolix-<version>-setup.exe`, NSIS) wird unter Linux gebaut: `prepolix.exe` entsteht per Cross-Compile mit MinGW, dazu kommen die Gmsh-DLL, das Programmsymbol und die Lizenztexte. Er installiert nach `C:\Program Files\prepolix`, legt einen Startmenü-Eintrag an (Desktop-Symbol optional), verknüpft auf Wunsch `.plx`-Projekte und trägt sich unter *Apps* zum Deinstallieren ein. CalculiX gehört nicht dazu.

```sh
sudo apt install gcc-mingw-w64-x86-64 nsis
scripts/build_windows_installer.sh     # -> target/windows-installer/
```

Ein Tag `v*` baut den Installer über den Workflow *Release* und hängt ihn an das GitHub-Release; per *Run workflow* entsteht er als Artefakt. Unter `C:\Program Files` darf prepolix ohne Administratorrechte nicht schreiben, das Arbeitsverzeichnis für CalculiX liegt dann in `%TEMP%\prepolix`.

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

Die Standardansichten folgen PrePoMax: Y zeigt nach oben. "Vertikal" stellt die Achse senkrecht, die der Bildschirm-Hochrichtung am nächsten liegt, "Ansicht senkrecht zu" (Menü Ansicht oder Rechtsklick) schaut entlang X, Y oder Z auf die Ebene, deren Normale die Achse ist, ein Klick auf eine Achse des Koordinatenkreuzes unten rechts (Spitze oder grauer negativer Teil) schaut genauso entlang dieser Richtung, und "Isometrisch, Achse oben" zeigt die isometrische Ansicht mit X, Y oder Z nach oben.

## Was schon geht

- `.inp`-Netze lesen: Knoten, Elemente (C3D4/6/8/10/15/20 inkl. R/I-Varianten, S3/4/6/8, M3D, CPS/CPE/CAX, B31/B32, T3D2/T3D3), `*NSET`, `*ELSET` (auch `GENERATE`), `*SURFACE`, `*INCLUDE`. Alle anderen Keywords werden übersprungen und in der Ausgabe aufgezählt.
- Parts entstehen aus dem `ELSET=` der `*ELEMENT`-Blöcke und lassen sich im Modellbaum ein- und ausblenden.
- Darstellung der Außenhaut mit Feature-Kanten und optionalen Netzkanten, auch für quadratische Elemente.
- `.frd`-Ergebnisse lesen (ASCII und binär): Netz, Materialien als Parts, alle Steps und Inkremente inklusive Eigenformen. Komponenten heißen wie in PrePoMax (`U1`, `S11`, …); ergänzt werden Verschiebungsbetrag `ALL`, `MISES`, `TRESCA`, Hauptspannungen und Vergleichsdehnung.
- Ergebnisanzeige wie in PrePoMax: Konturplot in 9 Farbstufen (Regenbogen, einstellbar), Legende, Min/Max mit Knoten, verformte Darstellung mit automatischem, echtem oder eigenem Faktor.
- Schnittansicht (*Ansicht → Schnittansicht …*): Schnittebene als Grundebene XY/YZ/XZ an einer Koordinate oder als Punkt und Normale, beides auch per Klick ins Modell (Normale aus zwei Punkten). Im 3D-Fenster lässt sich die Ebene am Pfeil verschieben und an den Bögen kippen. Volumenelemente zeigen ihre Schnittfläche mit Netzkanten und Ergebnisfarben.

- Geometrie importieren (STEP, IGES, BREP) und mit Tetraedern 1. oder 2. Ordnung vernetzen, ein Part pro Volumenkörper. Wie in PrePoMax wird jedes Part einzeln vernetzt (Kontextmenü des Parts: *Netz erzeugen*). Der Mesh Setup nimmt Meshing Parameters pro Part, lokale Netzgrößen auf Flächen und Kanten (Local Mesh Size) und die Gmsh-Algorithmen pro Part (Tetrahedral Gmsh) auf. Die Geometrie wird samt Mesh Setup im Projekt gespeichert.

Beispielmodelle liegen in [testdata/](testdata/), erzeugt von `testdata/erzeugen.py`; die STEP-Dateien stammen aus Gmsh. Die Modelle mit Step lassen sich direkt mit `ccx` rechnen; zwei Ergebnisdateien (`*.frd`) liegen bei.

## Aufbau

| Crate | Inhalt |
|---|---|
| `plx-core` | Einheiten, Formeln, Selektions-Rezepte, gemeinsame Typen |
| `plx-mesh` | FE-Netz: Knoten, Elemente, Sets, Surfaces, Geometrie-Topologie |
| `plx-model` | FE-Modell: Materialien, Sections, Steps, Randbedingungen, Lasten, Commands |
| `plx-io` | Dateiformate: CalculiX `.inp`/`.frd`/`.dat` und weitere |
| `plx-results` | Ergebnis-Datenmodell und abgeleitete Größen |
| `plx-mesher` | Geometrie-Import und Vernetzung mit Gmsh (zur Laufzeit geladen) |
| `plx-job` | Start und Überwachung von CalculiX |
| `plx-render` | wgpu-Renderer und Kamera |
| `plx-app` | Oberfläche (egui), ausführbares Programm `prepolix` |

Die unteren Crates kennen keine Oberfläche und lassen sich einzeln testen (`cargo test`).

## Lizenz

GPL-3.0-or-later, siehe [LICENSE](LICENSE).

Gmsh steht unter der GPL-2.0-or-later mit einer Ausnahme für OpenCASCADE, Netgen und METIS, OpenCASCADE unter der LGPL-2.1 mit Ausnahme; beides ist mit der GPL-3.0 verträglich. Wer prepolix mit der Gmsh-Bibliothek weitergibt, legt `GMSH-LICENSE.txt` bei und verweist auf die Quellen: Gmsh unter <https://gitlab.onelab.info/gmsh/gmsh> (Tag `gmsh_4_15_2`), OpenCASCADE unter <https://github.com/Open-Cascade-SAS/OCCT>.

Die mitgelieferte Schrift Noto Sans (`crates/plx-app/assets/fonts`, auf Latein, Griechisch und
gängige Symbole reduziert) steht unter der SIL Open Font License 1.1, siehe
[OFL.txt](crates/plx-app/assets/fonts/OFL.txt).
