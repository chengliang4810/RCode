# Contributing to RCode

Thanks for helping build RCode. This guide covers how to set up, what a good
pull request looks like, and the handful of rules that keep a local-first,
application working across its desktop, headless and mobile surfaces.

## Before you start

- For anything larger than a focused fix, open an issue or draft PR first so
  the approach can be agreed before you invest in it.
- Keep a pull request to one concern. A bug fix, a refactor, and a new feature
  are three PRs.
- Read [ARCHITECTURE.md](ARCHITECTURE.md) for the crate map and data model, and
  [CONTEXT.md](CONTEXT.md) for domain vocabulary. `docs/` holds design notes for
  most subsystems (sync, previews, performance, themes); check there before
  redesigning something.

## Upstream

`origin` is RCode itself (`github.com/chengliang4810/RCode`). The project RCode
was first derived from is the read-only `upstream` remote
(`github.com/zeronsh/zeron`): never push to it, and do not merge its later
changes — the first version is based on it, and RCode is maintained
independently from there on. Attribution for everything inherited lives in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Setup

The toolchain is pinned by [`rust-toolchain.toml`](rust-toolchain.toml)
(stable, with `rustfmt` and `clippy`).

**Linux** needs the GPUI system libraries CI installs:

```sh
sudo apt-get install -y libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libx11-dev libxcb1-dev libx11-xcb-dev libfontconfig1-dev libfreetype-dev \
  libasound2-dev libvulkan-dev pkg-config cmake libwebkit2gtk-4.1-dev libjson-glib-dev
```

**macOS** needs Xcode. **Windows** is covered in
[docs/reference/windows-development.md](docs/reference/windows-development.md).

Build and run the app with `cargo run -p rcode`.

### Running a dev build next to an installed RCode

An installed daemon holds the default data directory and IPC port. Give your
dev build its own so the two never share state:

```sh
RCODE_DATA_DIR=~/.rcode-dev RCODE_IPC_PORT=27700 cargo run -p rcode
```

Useful knobs for exercising the UI with deterministic agent output:

| Variable | Effect |
| --- | --- |
| `RCODE_HARNESS=mock` | Offers the mock harness, which streams canned turns |
| `RCODE_MOCK_SUBAGENT=1` | Mock turns spawn subagents |
| `RCODE_MOCK_THINKING=1` | Mock turns include markdown-heavy thinking |
| `RCODE_MOCK_TODO=1` | Mock turns work through an 8-item checklist (pair with `RCODE_MOCK_DELAY_MS=900` to watch the todo panel) |

## Tests

Run the suites for the crates you touched before opening a PR. These mirror CI:

```sh
cargo test --locked -p rcode-ui --lib -- --test-threads=1
cargo test --locked -p rcode-engine --lib
cargo test --locked -p rcode-harness            # includes tests/ fixtures, not just --lib
cargo test --locked -p rcode-sync --lib
cargo test --locked -p rcode-preview
```

- CI does not cover every crate on every platform. For engine, harness, and sync
  changes, the local suite is the gate, so say in the PR what you ran.
- Every bug fix comes with a test that fails without the fix. Every behavior
  change updates or adds the tests that pin it.
- Tests must be deterministic and offline: no real network, no real agent
  CLIs, no reliance on your home directory. Use the in-memory RPC client,
  fixtures under `tests/`, and temp dirs.
- A catalog or protocol change often touches both `--lib` tests and
  integration fixtures under `crates/*/tests/`. Run the whole crate.

## Code

- Match the surrounding code: its naming, its idioms, and its comment density.
  Comments explain *why*, not what.
- `cargo clippy --all-targets` should introduce no new warnings in code you
  touched.
- Format only your own changes. Parts of the tree are not rustfmt-clean, and
  whole-file reformatting buries the real diff. Drop formatting-only hunks in
  code you didn't otherwise change.
- Don't add a dependency without saying why in the PR. Dependencies ship to
  every user's machine.
- Don't bump the version. Releases are cut by maintainers; the version lives
  only in `[workspace.package]` in `Cargo.toml`.

### Compatibility across versions

A UI and its local daemon may use different builds. Shared client and engine
protocols, as well as persisted documents, need explicit compatibility rules:

- New fields on wire and document types (`rcode-proto`, `rcode-doc`) must be
  optional or `#[serde(default)]`. Older peers must still parse newer frames,
  and newer code must accept frames without the field.
- New behavior that needs the other side's cooperation is gated on a
  capability (`rcode_proto::capabilities`) or a device version check. Features
  degrade when a peer lacks them; they don't fail.
- Never rename or repurpose persisted keys, RPC method names, or document
  fields. Add new ones instead.
### Agent and providers

- `NativeHarness` connects the engine to the in-process agent; `MockHarness`
  supplies deterministic test events. See [the harness guide](crates/harness/README.md).
- Provider protocols belong in `crates/model`; agent tools and loop behavior
  belong in `crates/agent`. Keep engine adaptation in `crates/harness`.

## UI changes

- Reuse the app's existing idioms exactly: rows, menus, seams, hover
  treatments, empty states. If something similar already exists, copy it.
  Don't invent new visual elements as polish.
- Do what the change calls for and nothing more. Unrequested restyling of
  nearby UI gets reverted in review.
- Check light and dark mode, frosted and opaque surfaces. Hover and selection
  on glass lift toward white, never a dark wash.
- Menus mark the selected row with a wash, not a check glyph. Dropdowns open
  below their trigger.
- The bundled Geist font lacks most exotic whitespace (em space, thin space,
  and so on). Reserve space with characters it actually has.
- Animations use the shared motion kit (`crate::motion`) so they match the
  rest of the app.

## Pull requests

**Title:** a short imperative summary, for example
`Explorer: newest subagents first`.

**Description:** two sections.

- **Summary:** what changed and why, in a few bullets. Call out anything
  reviewers should look at closely, such as compatibility, migrations, or
  behavior that changed on purpose.
- **Test plan:** the tests you added and the suites you ran, as a checklist.
  List manual checks you did, and any you didn't do.

**Screenshots are required for every visible change.**

- Show the new state, and a before/after for changes to existing UI. Include
  light and dark mode when colors or surfaces are involved. Use a short
  recording for animation, drag, or scrolling changes.
- Drag the images into the PR description so GitHub hosts them as attachments.
  **Never commit screenshots to the repository** for a PR.
- Redact personal data before uploading: email addresses, account names,
  tokens, private repo names, and home-directory paths. Pixelate them; don't
  just crop around them.

**Keep it reviewable:**

- Keep the branch mergeable with `main`. PRs are squash-merged, so the PR
  title and description become the commit.
- Respond to review by pushing new commits rather than force-pushing over
  history the reviewer already read.
- CI must pass, or the failure must be a known flake that you name in the PR.

## Security

- Never commit secrets, tokens, or real account data, including in fixtures,
  screenshots, and logs.
- Code that spawns processes, handles credentials, or opens network listeners
  gets extra scrutiny in review. Explain in the PR why it needs to exist.
- To report a vulnerability, contact a maintainer privately. Please don't open
  a public issue.

## License

RCode is [MIT licensed](LICENSE). By contributing, you agree that your
contributions are licensed under the same terms.
