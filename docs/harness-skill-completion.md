# Composer commands and skill completion

The composer combines RCode workspace actions with Native Agent commands,
custom commands and skills. `/` completion works at a word boundary in the
draft; code, URLs and paths retain their literal meaning.

## Workspace actions

| Command | Action |
| --- | --- |
| `/model` | Choose model and reasoning |
| `/new` | Start a conversation |
| `/resume` | Search and open conversations |
| `/settings` | Open settings |
| `/diff` | Open changes |
| `/files` | Open project files |
| `/terminal` | Open a terminal tab |
| `/rename` | Rename the conversation |
| `/stop` | Interrupt the current run |

Selecting an action runs it immediately and consumes only its trigger. Other
draft text, attachments and queued edits are preserved. Actions requiring a
conversation appear only in an existing chat. Typing an unselected slash word
does not execute a local action.

Native command names take precedence when a workspace action has the same name;
the action then uses the `rcode:` prefix. Agent commands and skills remain draft
references until Send.

## Skills and commands

NativeHarness discovers skills through `rcode-agent::skills` and enabled plugin
contributions. Custom Markdown commands use the directories and precedence in
[custom commands](custom-commands.md). Selecting a skill stores its identity in
the composer; literal dollar signs and unselected text remain ordinary input.

Settings → Shortcuts → Composer completion controls `$` skill completion and
whether skills are separated from `/` commands. The two preferences are
independent. They affect completion, not literal typed text.

Canonical references use the `composer-references-v1` capability. Unsupported
hosts preserve the draft and report an update requirement instead of silently
changing a file, command or skill reference. See `crates/ui/src/pickers.rs`,
`crates/harness/src/native.rs` and `crates/agent/src/skills.rs` for implementation.
