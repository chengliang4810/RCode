# RCode Windows text backend patch

Source: `zeronsh/zui`, commit `361ff20b919fdfb80199ee6d9aed725300cb7d54`,
`crates/gpui_windows` (Apache-2.0). The license is included beside this file.

The source is retained here so released builds include the fix without modifying
Cargo's checkout cache. Cargo.toml expands the upstream workspace dependencies;
path dependencies still use the same pinned Git revision.

RCode's change is limited to `src/direct_write.rs`: font fallback mappings search
the process's custom font collection before the system collection and pass the
selected collection to DirectWrite. This lets bundled Sarasa UI SC supply Chinese
glyphs inside Geist/Geist Mono without installing fonts in Windows.

Remove this patch when the pinned upstream backend includes the same behavior.
