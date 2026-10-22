# Transcript file links

An agent's Markdown link that names a file on disk opens it in the conversation's own right-hand file panel: an editor tab at the line the link names. This is the same panel the file explorer opens documents in.

## What counts as a file link

- **Workspace-relative** — `[lib.rs](src/lib.rs)`. Resolved against the linking chat's checkout; when that root cannot own it, its parent chat's and this device's project roots are tried in order.
- **Absolute** — `[lib.rs](/abs/path/lib.rs)` on a POSIX host, `[lib.rs](D:\work\repo\src\lib.rs)` on a Windows one. Either separator works (`D:\…` or `D:/…`), and so does the `file://` URL spelling (`file:///abs/path`, `file:///D:/work/repo/src/lib.rs`).
- **Inline code** — a code span that names an existing file becomes the same link, shown under its file name. A span with nothing behind it keeps its inline-code look.

The destination is lexed, never resolved with the viewing machine's own filesystem rules: a macOS viewport showing a Windows checkout reads `D:\…` as a Windows path, and a Windows viewport showing a Linux checkout reads `/home/…` as a POSIX one.

## Where it opens

| Destination | Opens as |
| --- | --- |
| Absolute path inside a known root | the workspace-relative path in that root's chat — editable |
| Absolute path outside every known root | the absolute path — read-only, served by the device that owns the chat |
| Relative path | the linking chat's own checkout |

A link from a chat on another device is read through that device. Only an absolute path resolves outside a root: a relative one that escapes its checkout is not a file link at all.

Line anchors `:12`, `:12:5`, `#L12`, `#L12C5` and ranges (`#L12-L20`, `:12-20`) open at the first line they name. A zero line and a reversed range make the whole target not a file link.

## Context menu

Right-click a file link (or Shift+F10 on a focused one) for **Open in RCode**, **Open with default app**, **Show in folder** and **Copy path**. The three system-level rows appear only when the file is on this device; **Copy path** is offered whenever the link resolves, and copies the resolved absolute path rather than the link's label. Web links keep their browser rows; see [transcript web links](transcript-browser-links.md).

## Shape rules

Two Markdown-level traps sit between a Windows path and this grammar, and the parser closes both before the link is read: inside a destination a backslash is an escape character, so `D:\dir\.config\a.md` would lose its dot, and a literal space ends the destination, so `D:\My Docs\a.md` would not be a link at all. A drive destination is therefore rewritten to the `/` spelling — the same path to the host — and bracketed when it still holds a space. A link, an inline code span and the request the panel sends then all agree on one string (`D:/…`).

A destination stays plain text — not a link — for: a `?` query, control characters, a `.` or `..` step, a doubled or trailing separator, a drive-relative `C:x`, a UNC `\\server\share`, a bare filesystem root, and a file name without a dot unless the destination spells `file://`.

## Validation

```sh
cargo test -p rcode-ui --lib workspace_links
cargo test -p rcode-ui --lib drive_path_links
cargo test -p rcode-engine --lib drive_paths
cargo test -p rcode-engine --test workspace_files
```

Recorded for this change, on Windows x86_64:

- `workspace_links` unit tests cover drive paths resolving under a drive root, case-insensitive and separator-insensitive name comparison, sibling and other-drive paths falling outside, the strict shape rejections, and a drive code span linking to the file it names.
- `Shell::activate_session_link` opens an owned drive path as the workspace-relative editor path and an unowned one as the absolute read-only path.
- The engine reads a drive path inside the workspace as the equivalent relative path and an outside one as a read-only host file with no checkout identity, in either separator spelling; space targets still reject absolute paths.
