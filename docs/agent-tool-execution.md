# Shell execution and exact file edits

## Shell transport and exit status

The agent exposes one shell tool named `Bash`. On Windows it runs PowerShell,
preferring `pwsh` and falling back to the system's Windows PowerShell 5.1. On
Unix/macOS it runs Bash (or the existing sh fallback). There is no separate
PowerShell tool. Both executors share output limits, process-tree ownership,
cancellation and background management.

On Windows, commands travel through a unique UTF-8-with-BOM `.ps1` file, with
profiles and interactive input disabled. The BOM preserves Unicode on Windows
PowerShell 5.1; console and native pipeline output use UTF-8. Only the file path
passes through argv quoting, avoiding nested shell interpretation and command
length limits. The execution future owns the file through background execution,
completion and cancellation. The execution-policy flag affects only this child
process. Subcommands receive EOF on stdin, never the script itself.

PowerShell cmdlet errors fail the call. Native exit codes survive filtering by
PowerShell cmdlets. Windows PowerShell 5.1 can turn redirected native stderr
into error records even on exit 0; on that version, cmdlet stopping is set via
default common parameters instead of globally stopping on native stderr.

POSIX shells which support `pipefail` enable it before executing the command.
A failing compiler piped into a successful `tail` therefore returns failure.
`errexit` is not enabled: explicit conditionals, `||` recovery and subsequent
commands retain normal shell semantics. Expected nonzero statuses (including
grep with no matches or an intentionally truncated producer receiving SIGPIPE)
must be handled explicitly by the submitted command. A legacy `sh` without
pipefail retains its platform's pipeline semantics.

Plain and managed Bash share a timeout schema and parser. Timeouts are positive
integer seconds, defaulting to 120 and capped at 600. Invalid types, zero and
negative values are rejected. Managed foreground waits still promote the same
process to the background, unless auto-background is disabled. This limit is
per foreground command/wait; the agent's iteration count remains unlimited.

## Platform prompt

Windows guidance is appended according to the current tool registry, outside
the immutable conversation snapshot. Resumed chats therefore learn the current
executor too. It states that Bash accepts PowerShell syntax on Windows, explains
the selected edition's operators and encoding, and retains dedicated file tools.
Subagents receive the same guidance only when their registry contains Bash;
the existing role/tool restrictions remain in force. Other platforms receive
neither a PowerShell tool nor this Windows-specific prompt section.

## Exact edits across line endings

Read presents lines using LF. Edit consequently treats CRLF and LF as equivalent
when locating an old string, while other whitespace and code remain exact.
All equivalent matches participate in the uniqueness check; a mixed-ending
file cannot hide a second match simply because one occurrence uses CRLF.

Match offsets are mapped back to the original bytes. Each replacement adopts
the local line ending, and bytes outside replacement ranges are copied exactly.
UTF-8 BOMs, Unicode text, mixed endings and a missing final newline are retained.
Edit diffs contain the actual original and resulting file contents.

An outdated or incorrect snippet still fails without writing. The error can
point to a matching context line or note that the requested new string already
exists. These are recovery hints, not fuzzy edits or proof that a prior edit
completed successfully. The model should read the current file before retrying.

## Local reference implementations

Comparison made against the local checkouts on 2026-10-22, not a claim about a
particular upstream release:

| Concern | `D:\projects\claude-code` | `D:\projects\peri` | RCode decision |
| --- | --- | --- | --- |
| Shell | Independent Bash and PowerShell tools; PowerShell defaults on for Windows, prefers pwsh, and uses a provider with `-Command` or encoded sandbox transport. | One Bash tool; `mcp-packages/common/src/shell.rs` selects PowerShell on Windows and Bash elsewhere. | One Bash tool with a platform backend, using a script file on Windows. |
| Platform guidance | `PowerShellTool/prompt.ts` and `powershellDetection.ts` distinguish pwsh from Windows PowerShell 5.1, including operators and encoding. | Bash's description documents platform-specific execution. | Append current Windows/edition guidance to main and eligible child prompts, including resumed sessions. |
| Pipeline status | The inspected provider does not explicitly enable pipefail; sourced shell configuration may affect options. | The inspected shell builder does not explicitly enable pipefail. | Enable it when supported so filtered compiler/test failures remain failures. |
| Line endings | `src/utils/fileRead.ts` normalizes CRLF and records original line endings; `src/utils/file.ts` restores them when writing. FileEdit uses these helpers. | `mcp-packages/workspace/src/filesystem/edit.rs` matches raw content literally, without CRLF normalization. | Normalize matching, preserve original bytes outside edits and local endings inside replacements. |
| Missing old text | FileEdit tracks read state and detects stale reads. | Edit provides prefix/closest-line hints; a target transaction protects its commit. | Add bounded context/already-present hints while retaining exact matching and uniqueness requirements. |

## Verification

- `cargo test --locked --offline -p rcode-agent --lib`
- `cargo test --locked --offline -p rcode-agent --test agent_management`

The regression fixtures execute real local shell processes and file edits.
They cover script bytes, large heredocs, EOF stdin, pipeline exit codes,
temporary script cleanup, line-ending preservation and managed task behavior.
They do not require provider requests or modification of either reference repo.
