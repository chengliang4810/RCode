# Terminal program status (OSC 7501)

RCode's terminal accepts the [Program Status Protocol, revision 0.4](https://www.superlogical.com/rex/docs/build/program-status).
CLI programs must emit the protocol themselves. RCode does not infer their state
from titles, spinners, screen text, or process names.

The terminal tab uses a compact state icon. Hovering the tab shows the state,
blocking reason, progress, message, and child records. A blocked child takes
precedence over a working parent. Labels follow the app's selected language;
program-supplied text remains in its original language.

## Supported reports

```text
ESC ] 7501 ; state=working:app=my-agent:progress=40 ESC \
ESC ] 7501 ; state=blocked:kind=permission:app=my-agent ESC \
ESC ] 7501 ; state=done:app=my-agent ESC \
```

`ESC` is byte `0x1b`. Both `ESC \` and BEL (`0x07`) terminate a report.
Required `state` values are `idle`, `working`, `blocked`, `done`, `error`,
or `clear`. Optional fields are `kind`, `progress`, `app`, `id`, `title`, and
`msg`. `title` and `msg` use standard base64 of single-line UTF-8 text,
with optional padding. A report replaces the entire addressed record.

Hierarchical ids support simultaneous parent and worker states, inherited app
identity, and subtree clearing. There is no heartbeat. A new shell prompt
(`OSC 133;A`) or the attached process exiting clears transient states. Completed
and failed results remain until replaced, cleared, or acknowledged by keyboard
input in that terminal. Switching screens and soft resets preserve records;
a full reset clears them.

Feature detection:

```text
program  → ESC ] 7501 ; ? ESC \
terminal → ESC ] 7501 ; ?:states=idle,working,done,blocked,error:kinds=permission,question,auth ESC \
```

Reports travel through the existing PTY stream, including remote terminals.
Query replies use the same terminal's input route. Reports do not approve an
action, answer a question, or otherwise control a CLI program.

## Bounds and validation

- At most 4,096 bytes per report, including the introducer and terminator.
  Oversized and unterminated payloads have bounded buffering.
- At most 256 records per terminal; overflow evicts the least recently updated.
- Key, id, app, title, and message limits follow the linked specification.
- Invalid base64, UTF-8, control characters, or size violations reject a report
  atomically. Text is displayed as plain text with invisible formatting disarmed.
- Unknown fields and malformed pairs are ignored; an invalid id cannot replace
  the root record. Unrecognized states are ignored.
- Status reports do not ring the bell or appear in the terminal grid.

## Verification

Protocol and emulator regressions live in `program_status_tests.rs` and
`emulator.rs`. Run them with `cargo test -p rcode-ui --lib terminal::`.
They cover fragmented input, report replacement, hierarchy, malformed payloads,
limits, lifecycle, query ordering, and existing ANSI rendering behavior.

For a manual check, run this from a PowerShell terminal inside a build containing
the change:

```powershell
$esc = [char]27
[Console]::Write("${esc}]7501;state=working:app=demo:progress=40${esc}\")
Start-Sleep -Seconds 2
[Console]::Write("${esc}]7501;state=blocked:kind=question:app=demo${esc}\")
$null = Read-Host 'Continue'
[Console]::Write("${esc}]7501;state=done:app=demo${esc}\")
```

The icon should change as each report arrives. The completed result stays visible
after the prompt returns; the next terminal keypress acknowledges it.
