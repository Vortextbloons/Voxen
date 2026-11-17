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
        label = "nemotron-en-0.6b"
        path = Join-Path $modelsDir "nemotron-speech-streaming-en-0.6b\ebe59e5a817142986528bbbee5dba8db7b38ed50\nemotron-speech-streaming-en-0.6b.q8_0.gguf"
    },
    @{
        label = "nemotron-3.5-0.6b"
        path = Join-Path $modelsDir "nemotron-3.5-asr-streaming-0.6b\1c8deaecc64b91f034d73e08dd8b64625eb3395d\nemotron-3.5-asr-streaming-0.6b.q8_0.gguf"
    }
)

foreach ($entry in $models) {
    $dir = Join-Path $Out "$($entry.label)\transcribe-stream"
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    Get-ChildItem $audio -Filter *.wav | Sort-Object Name | ForEach-Object {
        $target = Join-Path $dir "$($_.BaseName).json"
        $arguments = "transcribe `"$($_.FullName)`" --model `"$($entry.path)`" --device vulkan:0 --stream --json"
        Start-Process -FilePath $exe -ArgumentList $arguments -Wait -NoNewWindow `
            -RedirectStandardOutput $target -RedirectStandardError "$target.err" | Out-Null
        if ((Get-Item $target).Length -eq 0) {
            Write-Output "empty: $($entry.label) $($_.Name)"
        }
    }
    Write-Output "done: $($entry.label)"
}
