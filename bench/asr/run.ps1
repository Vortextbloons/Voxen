param(
    [string]$Root = (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)),
    [string]$Out = (Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) "runtime\asr")
)

$ErrorActionPreference = "Stop"
$exe = Join-Path $Root "runtime\nemo-speech-0.2.0-windows-x86_64-vulkan\bin\nemo-speech.exe"
$audio = Join-Path $Root "bench\asr\audio"
$modelsDir = Join-Path $env:LOCALAPPDATA "NeMoSpeech\models\nvidia"
$models = @(
    @{
        label = "parakeet-tdt-0.6b-v3"
        path = Join-Path $modelsDir "parakeet-tdt-0.6b-v3\541d1f99c6b0c3cd0b11a95167540bb8edefd82b\parakeet-tdt-0.6b-v3.q8_0.gguf"
    },
    @{
        label = "nemotron-en-0.6b"
        path = Join-Path $modelsDir "nemotron-speech-streaming-en-0.6b\ebe59e5a817142986528bbbee5dba8db7b38ed50\nemotron-speech-streaming-en-0.6b.q8_0.gguf"
    },
    @{
        label = "nemotron-3.5-0.6b"
        path = Join-Path $modelsDir "nemotron-3.5-asr-streaming-0.6b\1c8deaecc64b91f034d73e08dd8b64625eb3395d\nemotron-3.5-asr-streaming-0.6b.q8_0.gguf"
    }
)

New-Item -ItemType Directory -Force -Path $Out | Out-Null
$commands = @()

function Invoke-Run($label, $model, $mode) {
    $runDir = Join-Path $Out $label
    New-Item -ItemType Directory -Force -Path $runDir | Out-Null
    $json = Join-Path $runDir "$mode.json"
    $err = Join-Path $runDir "$mode.err.txt"
    $save = Join-Path $runDir "$mode-output"
    New-Item -ItemType Directory -Force -Path $save | Out-Null
    $extra = if ($mode -eq "stream") { "--trace `"$(Join-Path $runDir 'stream-trace.jsonl')`"" } else { "" }
    $arguments = "bench asr `"$audio`" --model `"$model`" --device vulkan:0 --mode $mode -c 1 -n 3 --warmup 2 --save `"$save`" $extra --json"
    $commands += "`"$exe`" $arguments"
    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $process = Start-Process -FilePath $exe -ArgumentList $arguments -PassThru -NoNewWindow `
        -RedirectStandardOutput $json -RedirectStandardError $err
    $peak = 0
    while (-not $process.HasExited) {
        try {
            $process.Refresh()
            $peak = [Math]::Max($peak, $process.WorkingSet64)
        } catch {}
        Start-Sleep -Milliseconds 100
    }
    $process.WaitForExit()
    $clock.Stop()
    $summary = [pscustomobject]@{
        label = $label
        mode = $mode
        exitCode = $process.ExitCode
        wallMs = [math]::Round($clock.Elapsed.TotalMilliseconds, 1)
        peakRssMiB = [math]::Round($peak / 1MB, 1)
        modelBytes = (Get-Item -LiteralPath $model).Length
        model = $model
    }
    $summary | ConvertTo-Json | Set-Content -Path (Join-Path $runDir "$mode.run.json")
    $summary | Format-List
}

foreach ($entry in $models) {
    Invoke-Run $entry.label $entry.path "offline"
    if ($entry.label -ne "parakeet-tdt-0.6b-v3") {
        Invoke-Run $entry.label $entry.path "stream"
    }
}

$commands | Set-Content -Path (Join-Path $Out "commands.txt")
