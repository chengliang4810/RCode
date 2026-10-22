# Run the unchanged mobile tests with a Windows-host-only profiler workaround.
# Flutter #191069: https://github.com/flutter/flutter/issues/191069
# Dart fix: https://github.com/dart-lang/sdk/commit/0a7701121edee26d7e175c53af52063b94652a88
# The validated Dart 3.13.5 source predates that stack-boundary fix. The engine
# enables --profiler/--profile-vm when --enable-dart-profiling is present, and
# uses --no-profiler when absent. The test command has no public disabling flag.
# See docs/windows-flutter-tests.md. No installed SDK or product file is edited.
[CmdletBinding()]
param(
    [string[]]$TestFiles = @('test'),
    [string]$FlutterSdk,
    [switch]$ToolVerbose
)

$ErrorActionPreference = 'Stop'
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'This workaround is for Windows only. Use flutter test on other platforms.'
}
if ($TestFiles.Count -eq 0) { throw 'Specify at least one test file or directory.' }

$taskRepository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$taskMobile = Join-Path $taskRepository 'apps/mobile'
if (-not $FlutterSdk) {
    $taskFlutterCommand = (Get-Command flutter -ErrorAction Stop).Source
    $FlutterSdk = Join-Path (Split-Path -Parent $taskFlutterCommand) '..'
}
$taskSdk = (Resolve-Path -LiteralPath $FlutterSdk).Path
$taskKnownEngine = 'deb287481e3ce9468f3434937ced4240a70539ca'
$taskEngine = (Get-Content -LiteralPath (Join-Path $taskSdk 'bin/internal/engine.version') -Raw).Trim()
$taskDartVersion = (Get-Content -LiteralPath (Join-Path $taskSdk 'bin/cache/dart-sdk/version') -Raw).Trim()
if ($taskEngine -ne $taskKnownEngine -or $taskDartVersion -ne '3.13.5') {
    throw 'This workaround is validated only for Flutter 3.47.7 / Dart 3.13.5. Review the upstream fix before using it with another SDK.'
}
$taskTool = Join-Path $taskSdk 'packages/flutter_tools'
$taskOriginalConfig = Join-Path $taskTool '.dart_tool/package_config.json'
$taskDart = Join-Path $taskSdk 'bin/cache/dart-sdk/bin/dart.exe'
if (-not (Test-Path -LiteralPath $taskOriginalConfig) -or -not (Test-Path -LiteralPath $taskDart)) {
    throw 'Flutter SDK dependencies are missing. Initialize the SDK with flutter --version first.'
}

$taskTemporaryParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$taskTemporary = Join-Path $taskTemporaryParent ('rcode-flutter-tester-' + [Guid]::NewGuid().ToString('N'))
$taskIsolatedTool = Join-Path $taskTemporary 'flutter_tools'
$taskOldFlutterRoot = [Environment]::GetEnvironmentVariable('FLUTTER_ROOT', 'Process')
$taskPushed = $false
$taskExit = 1
try {
    New-Item -ItemType Directory -Path $taskIsolatedTool -Force | Out-Null
    foreach ($taskPart in @('lib', 'bin', 'pubspec.yaml')) {
        Copy-Item -LiteralPath (Join-Path $taskTool $taskPart) -Destination $taskIsolatedTool -Recurse
    }
    $taskDevice = Join-Path $taskIsolatedTool 'lib/src/test/flutter_tester_device.dart'
    $taskSource = [IO.File]::ReadAllText($taskDevice)
    $taskOriginalBranch = "if (debuggingOptions.enableDartProfiling) '--enable-dart-profiling',"
    $taskPatchedBranch = "if (debuggingOptions.enableDartProfiling && !platform.isWindows) '--enable-dart-profiling',"
    if ([regex]::Matches($taskSource, [regex]::Escape($taskOriginalBranch)).Count -ne 1) {
        throw 'Flutter tester source no longer matches the reviewed profiling branch. Refusing to alter other arguments.'
    }
    $taskPatched = $taskSource.Replace($taskOriginalBranch, $taskPatchedBranch)
    if ($taskPatched.Replace($taskPatchedBranch, $taskOriginalBranch) -cne $taskSource) {
        throw 'The isolated source change was not limited to the profiler argument.'
    }
    [IO.File]::WriteAllText($taskDevice, $taskPatched, [Text.UTF8Encoding]::new($false))

    $taskConfig = Get-Content -LiteralPath $taskOriginalConfig -Raw | ConvertFrom-Json
    $taskConfigUri = [Uri]::new($taskOriginalConfig)
    $taskFoundTool = $false
    foreach ($taskPackage in $taskConfig.packages) {
        if ($taskPackage.name -eq 'flutter_tools') {
            $taskPackage.rootUri = [Uri]::new($taskIsolatedTool + [IO.Path]::DirectorySeparatorChar).AbsoluteUri
            $taskFoundTool = $true
        } else {
            $taskPackage.rootUri = [Uri]::new($taskConfigUri, [string]$taskPackage.rootUri).AbsoluteUri
        }
    }
    if (-not $taskFoundTool) { throw 'The SDK package configuration does not contain flutter_tools.' }
    $taskIsolatedConfig = Join-Path $taskTemporary 'package_config.json'
    [IO.File]::WriteAllText($taskIsolatedConfig, ($taskConfig | ConvertTo-Json -Depth 30), [Text.UTF8Encoding]::new($false))
    $env:FLUTTER_ROOT = $taskSdk
    Push-Location -LiteralPath $taskMobile
    $taskPushed = $true
    $taskArguments = @('test', '--no-pub', '--concurrency=1') + $TestFiles
    if ($ToolVerbose) { $taskArguments += '-v' }
    Write-Output 'Windows host tester: native sampling profiling disabled in an isolated Flutter tool copy; tests and fixtures are unchanged.'
    & $taskDart --disable-dart-dev "--packages=$taskIsolatedConfig" (Join-Path $taskIsolatedTool 'bin/flutter_tools.dart') @taskArguments
    $taskExit = $LASTEXITCODE
} finally {
    if ($taskPushed) { Pop-Location }
    [Environment]::SetEnvironmentVariable('FLUTTER_ROOT', $taskOldFlutterRoot, 'Process')
    if (Test-Path -LiteralPath $taskTemporary) {
        $taskResolvedTemporary = (Resolve-Path -LiteralPath $taskTemporary).Path
        $taskExpectedParent = $taskTemporaryParent.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
        if (-not $taskResolvedTemporary.StartsWith($taskExpectedParent, [StringComparison]::OrdinalIgnoreCase) -or
            -not [IO.Path]::GetFileName($taskResolvedTemporary).StartsWith('rcode-flutter-tester-')) {
            throw 'Temporary cleanup target is outside the expected test directory.'
        }
        Remove-Item -LiteralPath $taskResolvedTemporary -Recurse -Force
    }
}
exit $taskExit
