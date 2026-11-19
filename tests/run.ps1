# Prüft das gebaute Plugin an einer 100×60×3-Platte.
# Zwei Schlitze-Kanten, dann laser_kerf = 0 (die Skizzen müssen mitziehen),
# Finger an der Vorderkante, und eine abgelehnte Stirnkante.
# Jeder Fall startet einen eigenen solidcraft-cli-Prozess.
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$cli = Join-Path (Split-Path -Parent $root) "solidcraft-cli.exe"
$pkg = Join-Path $root "dist\solidcraft-laserjoints"
if (-not (Test-Path $cli)) { throw "solidcraft-cli fehlt: $cli" }
if (-not (Test-Path (Join-Path $pkg "plugin.wasm"))) { throw "zuerst ./build.ps1 ausführen" }

$utf8 = New-Object System.Text.UTF8Encoding $false
$work = Join-Path $env:TEMP "solidcraft-laserjoints-tests"
New-Item -ItemType Directory -Force -Path $work | Out-Null

function Write-Json($path, $value) {
    $json = ConvertTo-Json -InputObject $value -Depth 12 -Compress
    [System.IO.File]::WriteAllText($path, $json, $utf8)
}

function Read-JsonText([string]$text) {
    if ($null -eq $text -or $text.Trim().Length -eq 0) { return $null }
    return ($text | ConvertFrom-Json)
}

function Invoke-Cli([string[]]$argList) {
    $out = Join-Path $work ("out-" + [guid]::NewGuid().ToString("n") + ".txt")
    $err = Join-Path $work ("err-" + [guid]::NewGuid().ToString("n") + ".txt")
    $p = Start-Process -FilePath $cli -ArgumentList $argList -Wait -PassThru -NoNewWindow `
        -RedirectStandardOutput $out -RedirectStandardError $err
    return @{
        code = $p.ExitCode
        out = if (Test-Path $out) { [System.IO.File]::ReadAllText($out) } else { "" }
        err = if (Test-Path $err) { [System.IO.File]::ReadAllText($err) } else { "" }
    }
}

function Invoke-Sc($script) {
    $path = Join-Path $work ("script-" + [guid]::NewGuid().ToString("n") + ".json")
    $result = Join-Path $work ("result-" + [guid]::NewGuid().ToString("n") + ".json")
    Write-Json $path $script
    $run = Invoke-Cli @("run", $path, "--results", "--json-out", $result)
    $parsed = $null
    if (Test-Path $result) {
        $parsed = Read-JsonText ([System.IO.File]::ReadAllText($result))
    }
    return @{ code = $run.code; out = $run.out; err = $run.err; result = $parsed; script = $path }
}

# Die Anführungszeichen müssen die Kommandozeile von CreateProcess überleben.
$inspectArg = '"' + ('{"path":"' + ($pkg -replace '\\', '\\') + '"}' -replace '"', '\"') + '"'
$inspect = Invoke-Cli @("exec", "plugin.inspect", $inspectArg)
if ($inspect.code -ne 0) {
    throw "plugin.inspect fehlgeschlagen ($($inspect.code)): $($inspect.err) $($inspect.out)"
}
$info = Read-JsonText $inspect.out
$approval = [string]$info.approval
if (-not $approval) { throw "plugin.inspect hat keinen approval-Hash geliefert" }
Write-Host "approval $approval"

function Edge($index, $point) {
    return @{ type = "edge"; body = "Panel"; index = $index; point = $point }
}

function Box() {
    return @{
        command = "solid.box"
        params = @{
            length = 100; width = 60; height = 3
            corner = @(0, 0, 0); body_name = "Panel"; name = "Panel"
        }
    }
}

function Joints($edges, $mode) {
    return @{
        command = "plugin.run"
        params = @{
            path = $pkg
            approval = $approval
            entry = "make-joints"
            params = @{ edges = @($edges); mode = $mode; kerf = 0.15 }
        }
    }
}

function StepMeasure() {
    return @{ command = "inspect.measure"; params = @{ bodies = @("Panel") } }
}

function Volume($step) {
    return [double]$step.bodies[0].volume_mm3
}

function BBox($step) {
    return $step.bodies[0].bbox
}

function Near([double]$got, [double]$want, [double]$tol, [string]$label) {
    $delta = [Math]::Abs($got - $want)
    if ($delta -gt $tol) {
        throw "$label gemessen $got, erwartet $want (Abweichung $delta)"
    }
    Write-Host "ok $label = $got"
}

function Coord($bbox, $side, $i) {
    return [double]$bbox.$side[$i]
}

function Show-Fail($run, $label) {
    Write-Host $run.err
    Write-Host $run.out
    throw "$label fehlgeschlagen (Exit $($run.code)), Skript $($run.script)"
}

Write-Host "--- Schlitze an Vorder- und Hinterkante ---"
$slots = Invoke-Sc @(
    (Box),
    (Joints @((Edge 1 @(50, 0, 3)), (Edge 3 @(50, 60, 3))) "slots"),
    (StepMeasure),
    @{ command = "parameters.change"; params = @{ name = "laser_kerf"; expression = 0 } },
    (StepMeasure)
)
if ($slots.code -ne 0 -or $null -eq $slots.result) { Show-Fail $slots "Schlitze-Lauf" }
$plugin = $slots.result[1].result
$made = 0
foreach ($joint in @($plugin.joints)) {
    $made += [int]$joint.features
    if ($joint.operation -ne "cut") { throw "Schlitz-Operation ist $($joint.operation)" }
    if ($null -ne $joint.dof -and [int]$joint.dof -ne 0) { throw "Kante $($joint.edge) dof $($joint.dof)" }
}
if ($made -ne 10) { throw "erwartet 10 Schlitze, gebaut $made" }
Near (Volume $slots.result[2]) 17331.75 0.05 "Volumen mit Kerf 0,15"
Near (Volume $slots.result[4]) 17325.0 0.05 "Volumen nach laser_kerf = 0"
$box = BBox $slots.result[2]
Near (Coord $box "min" 1) 0 0.05 "Schlitz-Platte min y"
Near (Coord $box "max" 1) 60 0.05 "Schlitz-Platte max y"

Write-Host "--- Finger an der Vorderkante ---"
$fingers = Invoke-Sc @(
    (Box),
    (Joints @((Edge 1 @(50, 0, 3))) "fingers"),
    (StepMeasure)
)
if ($fingers.code -ne 0 -or $null -eq $fingers.result) { Show-Fail $fingers "Finger-Lauf" }
$joint = @($fingers.result[1].result.joints)[0]
if ([int]$joint.features -ne 6) { throw "erwartet 6 Finger, gebaut $($joint.features)" }
if ($joint.operation -ne "join") { throw "Finger-Operation ist $($joint.operation)" }
if ($joint.direction -ne "negative") { throw "Finger-Richtung ist $($joint.direction)" }
Near (Volume $fingers.result[2]) 18409.05 0.05 "Finger-Volumen"
$box = BBox $fingers.result[2]
Near (Coord $box "min" 1) -3 0.05 "Finger stehen 3 mm über"
Near (Coord $box "min" 0) 0 0.05 "Finger min x"
Near (Coord $box "max" 0) 100 0.05 "Finger max x"
Near (Coord $box "min" 2) 0 0.05 "Finger min z"
Near (Coord $box "max" 2) 3 0.05 "Finger max z"

Write-Host "--- Zweiter Aufruf an der nächsten Kante ---"
$again = Invoke-Sc @(
    (Box),
    (Joints @((Edge 1 @(50, 0, 3))) "fingers"),
    (Joints @((Edge 3 @(50, 60, 3))) "slots"),
    (StepMeasure)
)
if ($again.code -ne 0 -or $null -eq $again.result) { Show-Fail $again "Zweiter Aufruf" }
$first = @($again.result[1].result.joints)[0]
$second = @($again.result[2].result.joints)[0]
if ($first.sketch -ne "Box joint profile 1") { throw "erste Skizze heißt $($first.sketch)" }
if ($second.sketch -ne "Box joint profile 2") { throw "zweite Skizze heißt $($second.sketch)" }
if ($second.feature -ne "Box joint 2") { throw "zweite Extrusion heißt $($second.feature)" }
Near (Volume $again.result[3]) 18074.925 0.05 "Volumen nach Finger und Schlitzen"

Write-Host "--- Stirnkante wird abgelehnt ---"
$end = Invoke-Sc @(
    (Box),
    (Joints @((Edge 8 @(0, 0, 1.5))) "slots"),
    (StepMeasure)
)
$blob = $end.err + $end.out
if ($end.code -eq 0) { throw "Stirnkante wurde akzeptiert: $blob" }
if ($blob -notmatch "Stirnseite" -and $blob -notmatch "lang") {
    throw "Stirnkante ohne klare Ablehnung (Exit $($end.code)): $blob"
}
if ($null -ne $end.result -and @($end.result).Count -ge 3) {
    Near (Volume $end.result[2]) 18000 0.05 "Volumen nach abgelehnter Stirnkante"
}
Write-Host "ok Stirnkante abgelehnt"
Write-Host "alle Fälle bestanden"
