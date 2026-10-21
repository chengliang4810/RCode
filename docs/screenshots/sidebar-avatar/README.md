Native GPUI screenshots from the isolated sidebar fixture.

```sh
RCODE_SIDEBAR_COMPACT=1 cargo run -p rcode-ui --example sidebar-fixture --features project-palette-fixture
```

The sidebar footer uses a circular 21px avatar button with a 13px circle matching the remote icon and a centered
monospace initial. It shows no account name or Alpha label. The menu opens to
the right, clamped within the window, and holds the identity line plus "Check
for updates" (only where there is no app menu to carry it).

The images were captured while the sidebar still had accounts, so the pill and
the menu read as a signed-in user. The build has one identity now — this device —
and both read `Local`.

Native X11 checks verified that clicking footer whitespace does not open the
menu and clicking the avatar opens and closes it. Screenshots show the resting
button, hover state, and open menu.
