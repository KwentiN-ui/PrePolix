# Modellprüfung: typische Abbruchursachen von CalculiX

Was prepolix vor dem Lauf prüft (`crates/plx-model/src/checks.rs`) und nach einem
fehlgeschlagenen Lauf aus der Ausgabe von CalculiX erkennt. Nachgestellt mit CalculiX 2.21
(Debian-Paket `calculix-ccx`, Spooles als Löser).

| Problem | Meldung von CalculiX 2.21 | Verhalten | Prüfung in prepolix |
|---|---|---|---|
| Teile nicht oder unvollständig gelagert (Starrkörperbewegung), z. B. Part ohne Tie | Spooles: keine, Lauf "erfolgreich" mit unbrauchbaren Werten; Pardiso/PaStiX: `zero pivot` | stiller Unsinn oder Abbruch | Rang der Lagerung je zusammenhängendem Teil (Ties verbinden, Kontakte nur lose), nur statische Steps |
| Teil nur über Kontakt gehalten | `zero pivot`, `too many cutbacks` | oft Abbruch | Warnung |
| Elemente ohne Section | `no material was assigned to element N` | Abbruch | Elemente je Part gegen Sections |
| Material ohne Elastizität | `no elastic constants were assigned` | Abbruch | Materialien, die eine Section nutzt |
| Frequency Step ohne Dichte | `no density was assigned` | Abbruch | wie oben |
| Querkontraktionszahl >= 0,5 | `Poisson coefficient should be less than 0.5` | Abbruch beim Einlesen | E > 0, -1 < nu < 0,5 |
| Verzerrte/umgestülpte Elemente, 2D-Elemente im Uhrzeigersinn | `nonpositive jacobian determinant in element N` | Abbruch | Jacobi-Determinante an den Integrationspunkten (`plx-mesh/src/jacobian.rs`) |
| Anfangsinkrement > Step-Dauer | `initial increment size exceeds step size` | Abbruch | Step-Einstellungen |
| Gleiche Knoten in mehreren Randbedingungen mit verschiedenen Werten | keine | letzter Wert gilt | Warnung an der späteren Randbedingung |
| Rotationen (DOF 4-6) an Volumenelementen | keine | ignoriert | Warnung |
| Rotationen (DOF 4-6) in 2D-Modellen | `usermpc: mpc of type is unknown` | Abbruch (PR #59: auch Gleitkommafehler) | Writer schreibt nur DOF 1-2; Ausgabe wird erkannt |
| Freiheitsgrad in Randbedingung und abhängig in Gleichung | `dependent side of a MPC and a SPC` | Abbruch | Ausgabe wird erkannt |
| Randbedingung auf Tie-Slave-Knoten | `WARNING in gentiedmpc: DOF ... not active` | Tie dort aufgehoben, Lauf geht weiter | keine (meist harmlos) |
| Keine Konvergenz | `too many cutbacks`, `increment size smaller than minimum` | Abbruch | Ausgabe wird erkannt |
| Last nur auf festgehaltenen Knoten | keine | Last wirkungslos | Warnung |
| Statischer Step ohne Last | keine | Ergebnis null | Warnung |
| Frequency Step ohne Lagerung | keine | sechs Starrkörpermoden nahe 0 Hz | keine (gewollt) |

Quellen: CalculiX-Handbuch (ccx 2.21), [PrePoMax-Forum: Known CalculiX limitations](https://prepomax.discourse.group/t/known-calculix-limitations/3050),
[PrePoMax-Forum: too many cutbacks](https://prepomax.discourse.group/t/error-too-many-cutbacks/2602),
[CalculiX-Forum: nonpositive jacobian](https://calculix.discourse.group/t/help-error-in-e-c3d-nonpositive-jacobian/1320),
[CalculiX-Forum: increment size smaller than minimum](https://calculix.discourse.group/t/error-increment-size-smaller-than-minimum/199).
