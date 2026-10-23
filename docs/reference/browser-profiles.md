# Browser profiles and login persistence

The embedded browser keeps persistent website data on Linux and macOS 14 or
later. Closing tabs or restarting RCode does not deliberately clear persistent
cookies, localStorage, or IndexedDB. A website can still expire/revoke a login,
and session-only cookies are not converted into persistent cookies.

| Action | Linux and macOS 14+ | macOS 12/13 |
| --- | --- | --- |
| Switch tabs or hide the panel | Keeps website data | Keeps website data |
| Close all browser tabs, then reopen | Reuses the profile | Reuses its in-memory store while the profile context lives |
| Restart RCode | Reopens persistent data | Loses temporary data |
| Switch RCode profile | Opens a separate store; returning recovers saved data | Opens a separate temporary store |

Windows embeds Microsoft Edge WebView2. Tabs in one window/profile share an
InPrivate context: cookies and website storage remain available while that
context lives and are discarded when it closes. Restarting RCode creates a
new context. Its temporary user-data folder retains runtime caches, not a
persistent website login profile. See [Windows development](windows-development.md).

## Identity and storage

Tabs and windows in one RCode process share the same browser context for the
same profile. Chat, project, repository, and navigation changes do not choose a
new profile. Local workspace identity uses the local device ID. Signed-in
identity uses workspace scope, user ID, and organization ID. Local, synced, and
development scopes stay separate. Signing out/unresolving the identity closes
the old tabs without deleting their saved website data.

Website data stays on the UI device, including when viewing a remote engine's
chat. Browser data is not part of RCode's workspace synchronization.

On Linux, storage lives under:

```text
{RCODE_DATA_DIR or ~/.rcode}/browser/{profile-hash}/
  cookies.sqlite     # WebKitGTK's persistent cookie database
  data/              # localStorage, IndexedDB and other website data
  cache/             # disposable WebKit cache
  profile.lock       # prevents a second helper from using the same profile
```

The profile hash is derived from identity, not a raw user-supplied path. New
directories/files are private to the OS user. The helper executable remains in
`$XDG_CACHE_HOME/rcode/browser` (or `~/.cache/rcode/browser`); deleting that
executable cache does not clear logins.

The Linux helper stays alive between tabs while its profile context lives. On
normal teardown the parent closes its command pipe, and the helper destroys
pages and waits for pending cookie operations before leaving its event loop.
The parent allows three seconds before forced termination. Another process
opening the same profile gets an explicit error instead of a concurrent writer.
Startup runs on a dedicated background thread, including executable extraction,
profile coordination and the bounded ten-second handshake. Navigation keeps the
UI responsive and shows a loading state until initialization finishes. If the
address changes during startup, the latest address is loaded; closing the tab
cancels delivery of its pending result. Startup/storage failures are shown on
the browser page; they do not silently select temporary storage.

On macOS 14+, WebKit manages storage in the app's website-data location. RCode
uses a named `WKWebsiteDataStore` with a stable UUID derived from the canonical
RCode data directory and profile identity. The app bundle identifier and data
directory must stay stable to reuse the same store. Separate installations and
test data directories select separate stores. Copying/removing RCode's data
directory does not copy/remove the WebKit-managed stores.

macOS 12/13 only exposes one default persistent WebKit store. RCode keeps its
isolated temporary stores on these versions and displays a notice explaining
that keeping logins requires macOS 14+ or the default external browser. It does
not put unrelated accounts in the single default store.

References: [WebKit profile APIs](https://webkit.org/blog/14423/building-profiles-with-new-webkit-api/),
[WebKitGTK cookie persistence](https://webkitgtk.org/reference/webkit2gtk/stable/method.CookieManager.set_persistent_storage.html).

## Validation

Run the browser unit and shell regression tests:

```sh
cargo test --locked -p rcode-ui --lib browser --features browser-fixture -- --test-threads=1
```

The Linux startup regressions use Python 3 to gate a synthetic helper. They
check that two windows remain responsive, the latest address wins, startup
errors are delivered asynchronously, and closing a pending tab cancels its
attachment. These tests do not need a display.

On Linux, exercise the real helper lifecycle and shared process registry:

```sh
xvfb-run -a cargo test --locked -p rcode-ui --lib profile_contexts_share_a_live_helper -- --ignored --test-threads=1
```

Build the existing native shell fixture:

```sh
cargo build --locked -p rcode-ui --example browser-fixture --features browser-fixture
```

Then, in a logged-in macOS 14+ desktop session:

```sh
python3 scripts/test-browser-storage.py --fixture target/debug/examples/browser-fixture
```

Or on Linux with an X display (Xvfb is sufficient):

```sh
xvfb-run -a python3 scripts/test-browser-storage.py --fixture target/debug/examples/browser-fixture
```

The driver runs a loopback-only site and independent application launches with
temporary profiles. It checks persistent and HttpOnly cookies, cookie expiry
and deletion, localStorage, IndexedDB, closing/reopening the last tab, switching
profiles with live pages, and returning to the first profile. macOS runs use
the release Info.plist through the existing fixture bundle wrapper, and remove
their named WebKit stores afterward.

For a faster Linux storage check without building the UI:

```sh
cc -std=c11 -O2 -Wall -Wextra -Wno-unused-parameter \
  crates/ui/src/browser/linux/helper.c -o /tmp/rcode-webkit-storage \
  $(pkg-config --cflags --libs webkit2gtk-4.1 json-glib-1.0)
xvfb-run -a python3 scripts/test-browser-storage.py --helper /tmp/rcode-webkit-storage
```

This additionally checks profile locking, independent concurrent profiles,
invalid storage paths, and cookie file permissions. It does not exercise the
Rust shell. UI CI runs the native fixture on Linux and macOS.

The tests use synthetic tokens, not personal accounts. A manual GitHub/mail
check should sign in, close all tabs, restart RCode, revisit the site, switch
RCode profile, and return. Website-specific OAuth, MFA, expiry, or embedded
browser restrictions remain separate from storage persistence.

This change does not add password autofill, tab restoration, login import from
another browser, or a browsing-data deletion UI. Sessions already lost under
the old temporary configuration cannot be recovered; users must sign in again.
