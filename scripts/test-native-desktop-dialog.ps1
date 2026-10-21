# 原生 agent 桌面验证：隔离 profile 启动真实 GUI，通过本机 RPC 建会话、发真实
# prompt，观察标题与流式文本，只截取该应用窗口。不使用其他账号数据。
param(
    [string]$Exe,
    [Parameter(Mandatory)][int]$Port,
    [Parameter(Mandatory)][string]$ProvidersJsonPath
)
$ErrorActionPreference = 'Stop'
# -File 下 param 默认值里可能没有 $PSScriptRoot，因此这里解析脚本目录。
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
if ([string]::IsNullOrWhiteSpace($Exe)) { $Exe = Join-Path $ScriptDir '../target/debug/rcode.exe' }
if ($env:OS -ne 'Windows_NT') { throw '需要在 Windows 交互式桌面运行' }
$Exe = (Resolve-Path -LiteralPath $Exe).Path
$ProvidersJsonPath = (Resolve-Path -LiteralPath $ProvidersJsonPath).Path
$runId = [guid]::NewGuid().ToString('N')
$root = [IO.Path]::GetFullPath((Join-Path $ScriptDir ("../target/native-e2e-" + $runId)))
$data = Join-Path $root 'data'
$project = Join-Path $root 'project'
$shots = Join-Path $root 'screenshots'
New-Item -ItemType Directory -Force -Path $data | Out-Null
New-Item -ItemType Directory -Force -Path $project | Out-Null
New-Item -ItemType Directory -Force -Path $shots | Out-Null
Copy-Item -LiteralPath $ProvidersJsonPath -Destination (Join-Path $data 'providers.json')

$overrides = @{
    RCODE_DATA_DIR = $data
    RCODE_IPC_PORT = "$Port"
    RCODE_EDGE_URL = 'http://127.0.0.1:1'
    RCODE_EDGE_TOKEN = $null
    RCODE_ORG_ID = $null
    RCODE_OPEN_ROUTE = $null
    RCODE_DEMO_UPLOAD = $null
    RUST_BACKTRACE = '1'
}
$saved = @{}
foreach ($key in $overrides.Keys) {
    $saved[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
    [Environment]::SetEnvironmentVariable($key, $overrides[$key], 'Process')
}
$capture = Join-Path $ScriptDir 'test-windows-capture-client.ps1'
$p = $null
try {
    $stdout = Join-Path $root 'stdout.txt'
    $stderr = Join-Path $root 'stderr.txt'
    $p = Start-Process -FilePath $Exe -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    $null = $p.Handle
    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    do {
        $p.Refresh()
        if ($p.HasExited) { throw ('窗口前退出：{0}' -f $p.ExitCode) }
        if ($p.MainWindowHandle -ne 0) { break }
        Start-Sleep -Milliseconds 150
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($p.MainWindowHandle -eq 0) { throw '60 秒内未出现窗口' }
    do {
        $p.Refresh()
        if ($p.HasExited) { throw ('engine 启动期退出：{0}' -f $p.ExitCode) }
        if ((Get-Content -LiteralPath $stdout -Raw) -match 'engine core assembled') { break }
        Start-Sleep -Milliseconds 150
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not ((Get-Content -LiteralPath $stdout -Raw) -match 'engine core assembled')) { throw 'engine 未装配完成' }

    & $capture -WindowHandle $p.MainWindowHandle.ToInt64() -ExpectedProcessId $p.Id `
        -OutputFile (Join-Path $shots 'ready.png') -MetadataFile (Join-Path $shots 'ready.json')

    # Node 原生 WebSocket 不带 Origin，能通过本地 IPC 的跨源拒绝；驱动脚本同时
    # 观察标题与 transcript，因此这里只负责启动、截图和收尾。
    $evidence = Join-Path $root 'drive.json'
    $driver = Join-Path $ScriptDir 'native-desktop-drive.mjs'
    $driverLog = Join-Path $root 'driver.log'
    # 通过 cmd 执行：PowerShell 会把 node 的 stderr 当作 NativeCommandError 抛出。
    & cmd.exe /c ('node "{0}" "{1}" "" "{2}" "{3}" > "{4}" 2>&1' -f $driver, $Port, $project, $evidence, $driverLog)
    if (-not (Test-Path -LiteralPath $evidence)) {
        $logged = if (Test-Path -LiteralPath $driverLog) { Get-Content -LiteralPath $driverLog -Raw } else { '(无日志)' }
        throw ('驱动未产出证据文件；日志：' + $logged)
    }
    $drive = Get-Content -LiteralPath $evidence -Raw | ConvertFrom-Json

    $p.Refresh()
    & $capture -WindowHandle $p.MainWindowHandle.ToInt64() -ExpectedProcessId $p.Id `
        -OutputFile (Join-Path $shots 'completed.png') -MetadataFile (Join-Path $shots 'completed.json')
    "root=$root`ntitleObserved=$($drive.titleObserved)`nanswerObserved=$($drive.answerObserved)`ndevice=$($drive.device)" |
        Set-Content -Encoding utf8 (Join-Path $root 'result.txt')
    Get-Content (Join-Path $root 'result.txt')
} finally {
    if ($null -ne $p) {
        $p.Refresh()
        if (-not $p.HasExited) {
            if ($p.CloseMainWindow()) {
                if (-not $p.WaitForExit(20000)) { $p.Kill() }
            } else { $p.Kill() }
        }
    }
    foreach ($key in $overrides.Keys) { [Environment]::SetEnvironmentVariable($key, $saved[$key], 'Process') }
}
