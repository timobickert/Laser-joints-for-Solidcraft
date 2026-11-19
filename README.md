# Laser joints

Achtung: Dieses Plugin wurde komplett von GrokCode erstellt. Die Nutzung erfolgt auf eigene Gefahr.

Ich habe das Tool programmieren lassen um eine schnelle Lösung für ein Problem zu haben.

Das Tool liest keine Dateien, öffnet kein Netzwek und ruft nur feste SolidCraft-Befehle auf.

Fingerzinken und Schlitze für Laserschnitt-Boxen in [SolidCraft](https://github.com/tangency-labs/solidcraft). Das Plugin läuft als WebAssembly-Komponente (`craft:plugin@0.1.0`, Fähigkeit `commands@0.1`) und ruft nur SolidCraft-Befehle auf. Ein erfolgreicher Aufruf ist ein Undo-Schritt. Ein Fehler verwirft alles, was der Aufruf gebaut hat.

Jede Kante wird eine Skizze und eine Extrusion in der Timeline. Die Maße hängen an den Parametern `laser_finger`, `laser_kerf`, `laser_fit`, `laser_depth` und `laser_stock`. Ein späterer Aufruf schreibt dieselben Parameter neu; schon gebaute Zinken folgen mit.

## Was es baut

Die Platte ist ein eigener Körper, die gewählte Kante eine lange Kante, nicht die kurze Stirnseite. Die Wandstärke ist der Abstand der großen Fläche zu ihrer parallelen Gegenfläche.

Die nutzbare Länge ist die Kante abzüglich des Eckeneinzugs (voreingestellt eine Materialstärke je Ende, damit die Nachbarkante nicht in dieselbe Ecke schneidet). Darauf werden gleich breite Abschnitte verteilt, in ungerader Anzahl, solange die Anzahl nicht fest vorgegeben ist. Finger liegen auf den geraden Abschnitten, Schlitze auf den ungeraden, also um eine Nennbreite versetzt in den Lücken. So greifen beide Kanten ineinander, auch wenn eine vom anderen Ende her an der Ecke anliegt. An den Enden stehen bei den Fingern Finger und bei den Schlitzen Material. Was von der Nennbreite übrig bleibt, liegt zu gleichen Teilen an den Enden.

Die Nennbreite ist das 2,5-fache der Materialstärke, außer sie wird gesetzt. Sie muss größer sein als die Materialstärke.

Der Laser trägt auf jeder Flanke Material ab. Modelliert wird deshalb:

- Fingerbreite = Nennbreite + Kerf/2 + Pressung + Nennbreite × Pressung%/100
- Schlitzbreite = Nennbreite − Kerf/2 − Pressung − Nennbreite × Pressung%/100

Die Mitten bleiben. `kerf` 0, wenn LightBurn oder xTool den Kerf schon versetzt. Pressung ist die zusätzliche Enge nach diesem Ausgleich: 0 trifft sich genau, 0,05 mm ist eine leichte Presspassung (Finger breiter, Schlitze schmaler), ein negativer Wert gibt Spiel. Die Pressung in Prozent kommt obendrauf, als Anteil der Nennbreite.

Ein weiterer Aufruf nummeriert neue Skizzen und Extrusionen hinter den vorhandenen weiter (`Box joint profile 2`, `Box joint 2`, …). Kerf, Pressung und Fingerbreite bleiben gemeinsame Parameter: der neue Aufruf setzt sie neu, und die früheren Zinken folgen mit.

Finger werden an die Kante angefügt (`join`) und stehen um die Materialstärke über. Schlitze werden durch die Wand geschnitten (`cut`). Die Tiefe folgt `laser_depth`. Geht der Schnitt durch die erkannte Wand, folgt die Extrusionslänge `laser_stock`, auch wenn die Fingertiefe davon abweicht.

## Parameter

| Parameter | Voreinstellung | Bedeutung |
| --- | --- | --- |
| `edges` | — | Mindestens eine Kante |
| `mode` | `fingers` | `fingers` oder `slots` |
| `finger_width` | 0 | mm, 0 = 2,5 × Stärke |
| `finger_count` | leer | Abschnitte auf der nutzbaren Länge, 1 bis 31 |
| `kerf` | 0,15 | Strahlbreite in mm |
| `fit` | 0 | Pressung in mm nach dem Kerf. 0 genau, positiv enger, negativ Spiel |
| `fit_percent` | 0 | dieselbe Pressung zusätzlich in Prozent der Nennbreite |
| `material_thickness` | 0 | mm, 0 erkennt die Wand |
| `corners` | `inset` | `inset` oder `flush` |
| `corner_margin` | leer | mm, überschreibt die Ecken-Option |

## Installieren

SolidCraft 0.41 oder neuer (Plugin-Host). Dieses Paket ist gegen `commands@0.1` gebaut, das der Host weiter ausführt.

1. `dist/solidcraft-laserjoints.zip` bauen oder nehmen.
2. In SolidCraft: Preferences › Plugins › Install plugin… und die Zip wählen. Der Dialog zeigt Name, Version und die Befehle. Das Plugin braucht keine Dateien und kein Netzwerk.
3. Der Eintrag liegt im Menü Plugins und im Reiter Model. Kanten wählen, Werte setzen, Run.

Die Zip enthält `plugin.json` und `plugin.wasm` im Wurzelordner.

Kopieren ohne den Installer geht auch, dann fehlt die Freigabe und ein Lauf wird abgelehnt:

| System | Ordner |
| --- | --- |
| Windows | `%LOCALAPPDATA%\SolidCraft\plugins\solidcraft-laserjoints\` |
| macOS | `~/Library/Application Support/SolidCraft/plugins/solidcraft-laserjoints/` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/solidcraft/plugins/solidcraft-laserjoints/` |

## Beispiel über die Kommandozeile

`plugin.inspect` liefert den Hash der geprüften Paketdateien und führt keinen Plugin-Code aus. `plugin.run` braucht genau diesen Hash. `<cli>` ist `solidcraft-cli` aus der SolidCraft-Installation.

```powershell
$cli = "C:\Pfad\zu\solidcraft-cli.exe"
$pkg = (Resolve-Path "dist\solidcraft-laserjoints").Path
# Windows PowerShell entfernt Anführungszeichen in nativen Argumenten. cmd lässt sie stehen.
$arg = '"' + ('{"path":"' + ($pkg -replace '\\','\\') + '"}' -replace '"','\"') + '"'
$info = cmd /c "`"$cli`" exec plugin.inspect $arg" | ConvertFrom-Json
$approval = $info.approval
```

`make-slots.json`, Pfade und Hash eingesetzt:

```json
[
  {"command": "solid.box", "params": {
    "length": 100, "width": 60, "height": 3,
    "corner": [0, 0, 0], "body_name": "Panel", "name": "Panel"}},
  {"command": "plugin.run", "params": {
    "path": "<dist/solidcraft-laserjoints>",
    "approval": "<approval aus plugin.inspect>",
    "entry": "make-joints",
    "params": {
      "edges": [{"type": "edge", "body": "Panel", "index": 1, "point": [50, 0, 3]}],
      "mode": "slots",
      "kerf": 0.15
    }}},
  {"command": "inspect.measure", "params": {"bodies": ["Panel"]}}
]
```

```powershell
& $cli run make-slots.json --results --save box-slots.sc
```

Die Vorderkante einer 100×60×3-Platte bekommt fünf Schlitze von 7,425 mm (Nennbreite 7,5 mm, Ausgleich Kerf/2 = 0,075 mm), 3 mm tief, um eine Nennbreite gegenüber den sechs Fingern versetzt. Das Volumen fällt von 18000 mm³ auf 17665,875 mm³. Dieselbe Kante mit `mode: fingers` wächst auf 18409,05 mm³, und die Platte steht vorn 3 mm über.

`tests/run.ps1` prüft beide Fälle, zwei Kanten in einem Aufruf, das Nachziehen von `laser_kerf`, einen zweiten Aufruf an der nächsten Kante und eine abgelehnte Stirnkante.

## Bauen

Rust mit dem Target `wasm32-unknown-unknown`, und [wasm-tools](https://github.com/bytecodealliance/wasm-tools/releases) auf `PATH`, in `WASM_TOOLS` oder als `tools/wasm-tools.exe`.

```powershell
./build.ps1
```

```bash
./build.sh
```

Beides schreibt `dist/solidcraft-laserjoints/` und `dist/solidcraft-laserjoints.zip`.

`cargo test` prüft die Verteilung, die Kerf-Rechnung und die Ebene einer Plattenkante, ohne SolidCraft.

## Grenzen

Eine Kante muss gerade sein und zwischen zwei ebenen Flächen liegen. Gekrümmte Kanten, die Stirnseite und Kanten, die kürzer sind als Finger plus Eckeneinzug, werden abgelehnt. Mehr als 31 Abschnitte werden auf 31 gestreckt. Die Parameter gelten für den ganzen Entwurf: ein zweiter Aufruf mit anderer Stärke setzt `laser_stock` neu, und die früheren Zinken bauen sich mit um.
