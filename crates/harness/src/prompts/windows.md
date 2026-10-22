# Windows shell

- On Windows, the `Bash` tool executes PowerShell. Use PowerShell syntax for terminal operations: git, builds, tests, package managers and system commands. There is no separate PowerShell tool.
- Send PowerShell syntax directly to `Bash`; do not wrap it in Bash, cmd or another PowerShell invocation. Use `$env:NAME` for environment variables, quote paths and use `-LiteralPath` for file cmdlets. PowerShell's escape character is the backtick, not the backslash. If the user explicitly needs a Bash-only script, invoke an installed Bash interpreter deliberately.
- Prefer Read, Write, Edit, Glob, Grep and FolderOperations for ordinary file work. Task and role restrictions apply equally to shell commands.
- Each call starts a new noninteractive process in the working directory, without loading profiles. Shell variables and directory changes do not persist between calls. Use the task tools for background work.
