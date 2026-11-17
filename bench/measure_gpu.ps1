param(
    [string]$Root = (Split-Path -Parent $PSScriptRoot),
    [string]$Out = (Join-Path (Split-Path -Parent $PSScriptRoot) "runtime")
)

$ErrorActionPreference = "Stop"
$lm = Join-Path $env:LOCALAPPDATA "NeMoSpeech\models\nvidia"
$llama = "C:\Users\isaac\.lmstudio\extensions\backends\llama.cpp-win-x86_64-vulkan-avx2-2.71.0\llama-server.exe"
$models = "C:\Users\isaac\.lmstudio\models"

function Get-GpuMiB([int]$processId) {
    try {
        $samples = (Get-Counter '\GPU Process Memory(*)\Dedicated Usage' -ErrorAction SilentlyContinue).CounterSamples
        $match = $samples | Where-Object { $_.InstanceName -like "pid_${processId}_*" }
        if ($match) {
            return [math]::Round((($match | Measure-Object -Property CookedValue -Sum).Sum) / 1MB, 1)
        }
    } catch {}
    return 0
}

function Measure-Process($label, $file, $arguments, [int]$maxSamples = 60) {
    $process = Start-Process -FilePath $file -ArgumentList $arguments -PassThru -NoNewWindow `
        -RedirectStandardOutput (Join-Path $Out "gpu-$label.out") `
        -RedirectStandardError (Join-Path $Out "gpu-$label.err")
    $peakGpu = 0
    $peakRss = 0
    $samples = 0
    while (-not $process.HasExited -and $samples -lt $maxSamples) {
        try {
            $process.Refresh()
            $peakRss = [math]::Max($peakRss, $process.WorkingSet64)
        } catch {}
        $peakGpu = [math]::Max($peakGpu, (Get-GpuMiB $process.Id))
        $samples++
        Start-Sleep -Milliseconds 200
    }
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
    }
    [pscustomobject]@{
        label = $label
        peakGpuDedicatedMiB = $peakGpu
        peakRssMiB = [math]::Round($peakRss / 1MB, 1)
        samples = $samples
    }
}

$results = @()
$asr = Join-Path $Root "runtime\nemo-speech-0.2.0-windows-x86_64-vulkan\bin\nemo-speech.exe"
$audio = Join-Path $Root "bench\asr\audio"
$asrModels = @(
    @{ label = "asr-parakeet-tdt"; path = Join-Path $lm "parakeet-tdt-0.6b-v3\541d1f99c6b0c3cd0b11a95167540bb8edefd82b\parakeet-tdt-0.6b-v3.q8_0.gguf" },
    @{ label = "asr-nemotron-en"; path = Join-Path $lm "nemotron-speech-streaming-en-0.6b\ebe59e5a817142986528bbbee5dba8db7b38ed50\nemotron-speech-streaming-en-0.6b.q8_0.gguf" },
    @{ label = "asr-nemotron-3.5"; path = Join-Path $lm "nemotron-3.5-asr-streaming-0.6b\1c8deaecc64b91f034d73e08dd8b64625eb3395d\nemotron-3.5-asr-streaming-0.6b.q8_0.gguf" }
)
foreach ($entry in $asrModels) {
    $arguments = "bench asr `"$audio`" --model `"$($entry.path)`" --device vulkan:0 -c 1 -n 2 --warmup 0 --json"
    $results += Measure-Process $entry.label $asr $arguments 80
}

$llamaModels = @(
    @{ label = "cleanup-qwen3.5-2b"; path = Join-Path $models "unsloth\Qwen3.5-2B-GGUF\Qwen3.5-2B-Q4_K_S.gguf" },
    @{ label = "cleanup-qwen3.5-4b"; path = Join-Path $models "unsloth\Qwen3.5-4B-GGUF\Qwen3.5-4B-Q4_K_S.gguf" },
    @{ label = "cleanup-qwen3.5-9b"; path = Join-Path $models "unsloth\Qwen3.5-9B-GGUF\Qwen3.5-9B-Q4_K_S.gguf" }
)
foreach ($entry in $llamaModels) {
    $arguments = "-m `"$($entry.path)`" --host 127.0.0.1 --port 18243 -c 2048 -t 8 -rea off --no-webui"
    $results += Measure-Process $entry.label $llama $arguments 40
}

$results | ConvertTo-Json | Set-Content (Join-Path $Out "gpu-memory.json")
$results | Format-Table -AutoSize
