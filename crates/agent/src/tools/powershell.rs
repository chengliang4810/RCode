//! Windows PowerShell scripts using the same process-tree and output lifecycle
//! as Bash. Script bytes never pass through a second shell's quoting rules.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::bash;
use crate::tool::ToolError;

pub fn program() -> &'static Path {
    static PROGRAM: OnceLock<PathBuf> = OnceLock::new();
    PROGRAM.get_or_init(|| {
        bash::which_static("pwsh")
            .or_else(|| bash::which_static("powershell"))
            .unwrap_or_else(|| {
                std::env::var_os("SystemRoot")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("C:/Windows"))
                    .join("System32/WindowsPowerShell/v1.0/powershell.exe")
            })
    })
}

pub(super) fn shell_command(
    command: &str,
) -> Result<(tokio::process::Command, Option<tempfile::TempPath>), ToolError> {
    command_for(program(), command)
}

fn command_for(
    program: &Path,
    command: &str,
) -> Result<(tokio::process::Command, Option<tempfile::TempPath>), ToolError> {
    let mut script = tempfile::Builder::new()
        .prefix("rcode-powershell-")
        .suffix(".ps1")
        .tempfile()
        .map_err(|error| {
            ToolError::Execution(format!("cannot create PowerShell script: {error}"))
        })?;
    // Windows PowerShell 5.1 treats BOM-less UTF-8 as the active ANSI code page.
    // Both it and pwsh understand a UTF-8 BOM; stdout/native pipeline input use
    // UTF-8 too, matching the shared executor's incremental decoder.
    // Desktop PowerShell turns redirected native stderr into NativeCommandError
    // records even on exit 0. Stop cmdlets through their common parameter on
    // 5.1, without treating those native stderr records as terminating errors.
    let source = format!(
        "\u{feff}$ErrorActionPreference = 'Stop'\n\
         if ($PSVersionTable.PSVersion.Major -lt 7) {{\n\
         $ErrorActionPreference = 'Continue'\n\
         $PSDefaultParameterValues['*:ErrorAction'] = 'Stop'\n\
         }}\n\
         [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)\n\
         $OutputEncoding = [Console]::OutputEncoding\n\
         $PSNativeCommandUseErrorActionPreference = $true\n\
         $global:LASTEXITCODE = 0\n\
         try {{\n& {{\n{command}\n}}\n\
         if ($LASTEXITCODE -ne 0) {{ exit $LASTEXITCODE }}\n\
         }} catch {{\n\
         [Console]::Error.WriteLine(($_ | Out-String))\n\
         if ($_.Exception.PSObject.Properties['ExitCode']) {{ exit $_.Exception.ExitCode }}\n\
         exit 1\n\
         }}\n"
    );
    script.write_all(source.as_bytes()).map_err(|error| {
        ToolError::Execution(format!("cannot write PowerShell script: {error}"))
    })?;
    let path = script.into_temp_path();
    let mut process = tokio::process::Command::new(program);
    process
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(path.as_os_str());
    // ExecutionPolicy applies only to this child invocation, not machine/user
    // configuration. Ownership of the file travels with the shared executor.
    Ok((process, Some(path)))
}

#[cfg(test)]
#[path = "powershell_tests.rs"]
mod tests;
