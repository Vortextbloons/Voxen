param(
    [string]$Exe = (Join-Path (Split-Path -Parent $PSScriptRoot) "packaging\out\Voxen.exe"),
    [int]$IdleSeconds = 10,
    [double]$MaxIdleCpuPercent = 2.0,
    [switch]$KeepAlive
)

$ErrorActionPreference = "Stop"

function Get-Sample($process, [int]$seconds) {
    $process.Refresh()
    $cpu0 = $process.TotalProcessorTime.TotalSeconds
    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    Start-Sleep -Seconds $seconds
    $clock.Stop()
    $process.Refresh()
    $cpu1 = $process.TotalProcessorTime.TotalSeconds
    [pscustomobject]@{
        cpuPercent = [math]::Round((($cpu1 - $cpu0) / $clock.Elapsed.TotalSeconds) * 100, 2)
        workingSetMiB = [math]::Round($process.WorkingSet64 / 1MB, 1)
        privateMiB = [math]::Round($process.PrivateMemorySize64 / 1MB, 1)
    }
}

function Wait-Ready($process, [int]$timeoutMs) {
    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $inputReady = $false
    while ($clock.ElapsedMilliseconds -lt $timeoutMs) {
        $process.Refresh()
        if ($process.HasExited) { break }
        if (-not $inputReady -and $process.MainWindowHandle -ne 0) {
            try { $inputReady = $process.WaitForInputIdle(0) } catch { $inputReady = $true }
        }
        if ($inputReady) {
            $clock.Stop()
            return [math]::Round($clock.Elapsed.TotalMilliseconds, 0)
        }
        Start-Sleep -Milliseconds 10
    }
    $clock.Stop()
    return $null
}

$exeItem = Get-Item -LiteralPath $Exe
$zip = Join-Path $exeItem.DirectoryName "Voxen_0.1.0_x64.zip"

$process = Start-Process -FilePath $exeItem.FullName -PassThru
try {
    $readyMs = Wait-Ready $process 15000
    if ($null -eq $readyMs) { throw "Voxen did not become input-ready within 15 s" }
    Start-Sleep -Seconds 3

    $open = Get-Sample $process $IdleSeconds
    $process.Refresh()
    if ($process.HasExited) { throw "Voxen exited while idle" }

    $process.Refresh()
    if (-not $process.CloseMainWindow()) { throw "Could not close the Voxen window" }
    Start-Sleep -Seconds 2
    $process.Refresh()
    if ($process.HasExited) { throw "Voxen exited instead of staying in the tray" }
    $hidden = Get-Sample $process $IdleSeconds
    $process.Refresh()
    if ($process.HasExited) { throw "Voxen exited while hidden in the tray" }

    $result = [pscustomobject]@{
        exe = $exeItem.FullName
        exeBytes = $exeItem.Length
        exeMiB = [math]::Round($exeItem.Length / 1MB, 2)
        zipBytes = if (Test-Path $zip) { (Get-Item $zip).Length } else { $null }
        zipMiB = if (Test-Path $zip) { [math]::Round((Get-Item $zip).Length / 1MB, 2) } else { $null }
        startupReadyMs = $readyMs
        openIdleCpuPercent = $open.cpuPercent
        openWorkingSetMiB = $open.workingSetMiB
        openPrivateMiB = $open.privateMiB
        trayIdleCpuPercent = $hidden.cpuPercent
        trayWorkingSetMiB = $hidden.workingSetMiB
        trayPrivateMiB = $hidden.privateMiB
        cpuLimitPercent = $MaxIdleCpuPercent
        pass = ($open.cpuPercent -le $MaxIdleCpuPercent) -and ($hidden.cpuPercent -le $MaxIdleCpuPercent)
    }
    $result | Format-List
    if (-not $result.pass) {
        Write-Output "FAIL: idle CPU above $MaxIdleCpuPercent% of one core"
        exit 1
    }
    Write-Output "PASS: idle CPU at or below $MaxIdleCpuPercent% of one core"
}
finally {
    $process.Refresh()
    if (-not $process.HasExited -and -not $KeepAlive) {
        Stop-Process -Id $process.Id -Force
    }
}
