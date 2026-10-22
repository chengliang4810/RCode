# Windows Flutter test runner

The mobile package uses Flutter 3.47.7 / Dart 3.13.5. Its Windows
`flutter_tester.exe` has intermittently exited with native access violation
`0xc0000005` at RVA `0x35aaf0`, including an empty transcript test. No Dart
assertion accompanies those exits. Switching from software Skia to Impeller
did not prevent the crash. The tester's matching PDB is not in the SDK, so
that address has not been symbolized.

Flutter [issue #191069](https://github.com/flutter/flutter/issues/191069)
documents random Windows native crashes in the Dart VM sampling profiler.
The linked [Dart fix](https://github.com/dart-lang/sdk/commit/0a7701121edee26d7e175c53af52063b94652a88)
restricts native stack walking to the current stack pointer. The SDK's
[Dart revision](https://github.com/dart-lang/sdk/blob/04bcd1036cdc799ac6564988f159ee454d42c822/runtime/vm/profiler.cc)
does not contain that change. This is related evidence, not a symbolized proof
that every observed access violation has the same cause.

The Flutter tester enables `--enable-dart-profiling` by default. Its test CLI
does not expose a disabling option. The engine checks that option's presence,
so appending `=false` still enables it. When the option is absent, the engine
explicitly passes `--no-profiler` to the Dart VM.

Use the isolated Windows runner from the repository root:

```powershell
scripts/mobile/test-windows.ps1 -TestFiles test/session_page_test.dart -ToolVerbose
scripts/mobile/test-windows.ps1
```

It copies `flutter_tools` to a unique temporary directory, resolves its package
dependencies against the original SDK, and removes only the Windows tester's
profiling argument. It runs the tool through the SDK's Dart executable with
`FLUTTER_ROOT` pointing to the original SDK. Cleanup restores the environment
and removes only that temporary copy. The installed SDK, mobile application,
tests, assertions and original 45-message streaming fixture are unchanged.
The script rejects a different engine/Dart version or an unexpected source
branch instead of silently patching it.

`test/flutter_test_config.dart` also registers the package's complete Geist,
Geist Mono and Material Icons faces. The default tester disables asset-font
registration and would otherwise use Ahem. Loading actual fonts improves
visual fidelity; a successful font-loading comparison does not establish that
fonts caused the native crash. Optional QA-only CJK font loading remains in
the screenshot test.

Linux CI continues to use `flutter test --no-pub --concurrency=1` without this
Windows workaround. The workaround disables host CPU profiling only; it does
not skip a test or disable assertions, rendering or the native visual library.

## Controlled verification, 23 October 2026

The source was frozen before these four serial runs. Each session run retained
all six tests, including the original 45-message streaming fixture, immediate
reply anchoring, pinning and retry behavior. The full run also included the
complete-session file cache and persistence tests, frozen subagent transcripts,
and Chinese/English switching through the production app.

| Run | Result | Exit code | Wall time including isolated tool startup |
| --- | --- | --- | --- |
| Session page 1 | 6 passed | 0 | 24.46 s |
| Session page 2 | 6 passed | 0 | 24.39 s |
| Session page 3 | 6 passed | 0 | 24.07 s |
| Full package | 72 passed, 5 expected skips | 0 | 77.18 s |

`flutter analyze --no-pub` also completed with no issues (exit 0). The only
cleanup after the frozen test run added braces to an existing test-fixture
`if` branch; it did not change an assertion or runtime behavior.

The five skips in the default full run were:

| Test | Reason |
| --- | --- |
| `Dart controllers operate a real authenticated engine and recover completed offline turns` | Requires `RCODE_FLUTTER_RPC_URL` and `RCODE_FLUTTER_PROJECT`, supplied by the separate Rust `flutter_direct` integration gate. |
| `native visual reads the host script and handles controls after reattachment` | Requires `RCODE_NATIVE_VISUAL_TEST=1` and the built native visual library. |
| `scoped rows preserve controls through actual widget recycling and release on exit` | Same native visual gate. |
| `native status follows locale changes while preserving live script state` | Same native visual gate; this new locale regression accounts for the additional skip compared with the earlier suite. |
| `native math renders all six faces and rejects streaming fragments` | Same native visual gate. |

The native visual gate separately runs its four FFI tests and its ungated font
style test; enabling it removes these four native skips. It does not enable the
independent real-engine gate. No test was disabled to achieve the results above.

Verbose logs confirm that all tester processes used the original installed
`flutter_tester.exe`, retained `--enable-checked-mode`, `--verify-entry-points`
and software rendering, and omitted only `--enable-dart-profiling`. The original
SDK's profiling branch remained unchanged. Machine-readable results, launch
argument checks and verbose logs are saved locally under
`apps/mobile/build/qa/windows-profiler/`; these generated artifacts are not
committed.

These runs establish a successful stability comparison for the isolated
workaround on the pinned SDK. They do not prove the unsymbolized original
crashes share the upstream profiler defect, or that the ordinary Windows
runner has become reliable. Mac/iOS CI and Xcode builds were not run by this
Windows verification.
