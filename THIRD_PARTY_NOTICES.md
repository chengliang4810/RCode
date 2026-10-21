# Third-party notices

## Sarasa UI SC (SIL Open Font License 1.1)

Windows builds bundle Sarasa UI SC v1.0.42 (Regular, SemiBold, Bold), from
[be5invis/Sarasa-Gothic](https://github.com/be5invis/Sarasa-Gothic/releases/tag/v1.0.42).
The font files are unmodified. Copyright and license text are retained in
`crates/ui/assets/fonts/licenses/Sarasa-OFL.txt`. macOS builds use Apple's native
Chinese fonts and do not embed Sarasa.

## gpui_windows (Apache-2.0)

`vendor/gpui_windows` contains the Windows backend from zeronsh/zui revision
`361ff20b919fdfb80199ee6d9aed725300cb7d54`, with a local fix for bundled font
fallback collections. Its Apache-2.0 license and patch notes are included there.

## zeronsh/zeron (MIT)

The first version of RCode is based on
[zeronsh/zeron](https://github.com/zeronsh/zeron). Significant portions of the
source tree are copied or adapted from that project. RCode is maintained
independently and does not track its later changes.

The upstream project's MIT license and copyright notice are retained in
[LICENSE](LICENSE). This notice records RCode's derivation; it does not
relicense inherited code.

## Peri (Apache-2.0)

Parts of `crates/model` (provider protocol, transports, runtimes) and of
`crates/agent` (permission policy, message model, command parsing, skill
scanning, web search) are ported from the Peri project's `peri-model` and
`peri-acp-types` crates, licensed under the Apache License, Version 2.0. Those
files were modified for RCode; this notice records the derivation, and a copy of
the license is at [LICENSE-APACHE](LICENSE-APACHE).
