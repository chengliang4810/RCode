# Flutter mobile migration

The requested end state replaces `apps/ios` with `apps/mobile`, targeting Android and iOS from the same Flutter UI. The original native app must be removed after its behavior, assets and build responsibilities have been transferred. Historical source remains available in Git.

## Acceptance scope

- Authenticated host/API-key pairing, masked secure storage, host-scoped caches, foreground recovery and sign-out cleanup.
- Sessions/Settings/Search phone tabs, new-session capsule and working/input summary; tablet sidebar and conversation split.
- Pins, named sections, ordering/assignment, collapse, project grouping, rename, archive/restore, search and refresh.
- New conversation: host/project/checkout, remote folder browser, branch/worktree, model/effort, retained drafts and images.
- Incremental transcripts, stable scroll position, jump to latest, Markdown/code/math, text selection/copy, remote images.
- Composer queue/steer/interrupt/stop, file mentions, model/effort context, attachment upload progress and failures.
- Multi-page questions, single/multiple choice, custom/pre-filled/multiline answers and answer validation.
- Shared queue preview/actions and host edit leases; renew/release, preserve hidden attachment/Appshot context and pre-edit drafts.
- Full tool output/diff, frozen and live subagents; no request may fall back to the wrong execution host.
- Complete Luau visuals: controls, vectors/charts, canvas gestures and GPU scenes/shaders with lifecycle suspension.
- Appearance, wallpaper effects, math settings, project grouping, archive browser and notification preferences/navigation.
- Accessibility labels, system text scaling, reduced motion, keyboard avoidance and tablet keyboard navigation.
- Android build and integration tests; iOS runner/build wiring and simulator verification when macOS/Xcode is available.
- Remove legacy native iOS sources, generated Swift bindings, obsolete platform build scripts and CI references. Retain only Flutter's required iOS runner/bootstrap and portable Rust components that the new app uses.

## Existing product boundaries

Direct headless mode did not implement server-originated background APNs delivery. Local notifications while connected and permission/navigation behavior must not be described as push delivery after the OS terminates the app. No terminal, local agent, file editor or full Git panel existed in the old mobile UI.

## Design reference

Light/dark backgrounds `#F3F3F5` / `#060606`, elevated `#FFFFFF` / `#111113`, text `#27272C` / `#E8E8EA`, accent `#5B43E8` / `#8B7CF6`. Sessions use 62-point rows and 16.5-point titles; the composer uses a 50-point capsule with a 25-point radius and expands to a focused card. Glass is recreated with cross-platform blur/translucency. Geist and Geist Mono remain bundled under their OFL license.

Implementation and verification are in progress; this checklist is the scope, not evidence of completion.
