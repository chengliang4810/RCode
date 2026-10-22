param(
    [string]$Abis = 'arm64-v8a,x86_64',
    [string]$NdkDirectory,
    [string]$OutputDirectory,
    [string]$CargoProfile = 'mobile'
)
$ErrorActionPreference = 'Stop'
$taskRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../../..')).Path
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $taskRoot 'apps/mobile/build/rust-jni' }
if (-not $NdkDirectory) { $NdkDirectory = $env:ANDROID_NDK_HOME }
if (-not $NdkDirectory) {
    $sdkPath = $env:ANDROID_SDK_ROOT
    if (-not $sdkPath) { $sdkPath = $env:ANDROID_HOME }
    if (-not $sdkPath) { throw 'Set ANDROID_NDK_HOME or ANDROID_SDK_ROOT before building native visuals.' }
    $NdkDirectory = Join-Path $sdkPath 'ndk/27.0.12077973'
}
$ndkPath = (Resolve-Path -LiteralPath $NdkDirectory).Path
$compilerPath = Join-Path $ndkPath 'toolchains/llvm/prebuilt/windows-x86_64/bin'
if (-not (Test-Path -LiteralPath (Join-Path $compilerPath 'clang.exe'))) { throw 'The Android NDK Windows toolchain was not found.' }
$env:ANDROID_NDK_HOME = $ndkPath
$env:ANDROID_NDK_ROOT = $ndkPath
$env:CARGO_INCREMENTAL = '0'
foreach ($abi in $Abis.Split(',')) {
    $abi = $abi.Trim()
    $triple, $compiler = switch ($abi) {
        'arm64-v8a' { 'aarch64-linux-android', 'aarch64-linux-android24' }
        'x86_64' { 'x86_64-linux-android', 'x86_64-linux-android24' }
        default { throw "Unsupported native ABI: $abi" }
    }
    $targetEnvironment = $triple.Replace('-', '_')
    $upperTarget = $targetEnvironment.ToUpperInvariant()
    [Environment]::SetEnvironmentVariable("CARGO_TARGET_${upperTarget}_LINKER", (Join-Path $compilerPath "$compiler-clang.cmd"), 'Process')
    [Environment]::SetEnvironmentVariable("CARGO_TARGET_${upperTarget}_RUSTFLAGS", '-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-lc++_shared', 'Process')
    [Environment]::SetEnvironmentVariable("CC_$targetEnvironment", (Join-Path $compilerPath "$compiler-clang.cmd"), 'Process')
    [Environment]::SetEnvironmentVariable("CXX_$targetEnvironment", (Join-Path $compilerPath "$compiler-clang++.cmd"), 'Process')
    [Environment]::SetEnvironmentVariable("AR_$targetEnvironment", (Join-Path $compilerPath 'llvm-ar.exe'), 'Process')
    [Environment]::SetEnvironmentVariable("CXXSTDLIB_$targetEnvironment", 'c++_shared', 'Process')
    Push-Location -LiteralPath $taskRoot
    try {
        & rustup target add $triple
        if ($LASTEXITCODE -ne 0) { throw "Unable to install Rust target $triple" }
        & cargo rustc --locked -p rcode-mobile --lib --crate-type cdylib --profile $CargoProfile --target $triple
        if ($LASTEXITCODE -ne 0) { throw "Native visual build failed for $abi" }
    } finally { Pop-Location }
    $destination = Join-Path $OutputDirectory $abi
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    Copy-Item -LiteralPath (Join-Path $taskRoot "target/$triple/$CargoProfile/librcode_mobile.so") -Destination (Join-Path $destination 'librcode_mobile.so') -Force
    & (Join-Path $compilerPath 'llvm-strip.exe') --strip-unneeded (Join-Path $destination 'librcode_mobile.so')
    if ($LASTEXITCODE -ne 0) { throw "Unable to strip native visual symbols for $abi" }
    Copy-Item -LiteralPath (Join-Path $ndkPath "toolchains/llvm/prebuilt/windows-x86_64/sysroot/usr/lib/$triple/libc++_shared.so") -Destination (Join-Path $destination 'libc++_shared.so') -Force
}
