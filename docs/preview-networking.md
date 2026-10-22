# Project previews

Run an HTTP development server in a project and open a Browser tab in that
session. The tab lists discovered local services. **Open** navigates to
`http://<device>.<project>.localhost:7331`; additional services receive persistent
names such as `<device>.<project>-api.localhost:7331`.

The engine owns discovery and the loopback proxy. Closing a browser tab does not
stop discovery or change an address. Service shutdown cancels proxy tasks and
active streams. A busy proxy port produces an inline error and a bounded retry.

## Discovery and identities

Linux discovery joins the current user's `/proc` process cwd, ancestry, creation
time and socket descriptors to listening TCP sockets. macOS uses `lsof` and `ps`.
Only loopback-reachable listeners belonging to a known local project are probed;
the deepest matching project wins.

Commands with explicit authentication arguments (`login`, `auth`, `authenticate`,
`signin`, `sign-in`, `sso`, `oauth`, `oauth2`, before `--`) are excluded before any
connection. This prevents discovery probes from consuming one-shot login
callbacks. A custom callback without an authentication argument is not identified
by that check.

Discovery runs every two seconds. Bounded HEAD probes accept valid HTTP status
responses, including application errors. Confirmed HTTP sockets are not probed
again during that socket's lifetime; non-HTTP results use capped backoff.
Before connecting, the engine rechecks process identity and cwd to reject stale
port reuse.

`previews.json` stores service IDs and hostname labels separately from PIDs and
ports. Command identities omit common port options. Persisted aliases let a
service retain its URL across a port change. `WatchPreviews` resolves the current
session checkout when building the local catalog.

## HTTP and stream transport

On macOS 14 and later, WebKit uses a per-domain HTTP CONNECT configuration for
preview hostnames. Only known names at the preview port are accepted; unrelated
websites retain their normal routing.

The proxy validates the Host against the catalog and opens a service ID through
the local multiplexer. The connector resolves that ID to a live local listener.
Frames carry bounded stream payloads and credit-based flow control, with explicit
end and cancellation states.

Hyper streams request and response bodies. The proxy removes hop-by-hop headers,
preserves Host and Origin together, sets X-Forwarded-Host and rewrites absolute
localhost redirects. End-to-end headers remain intact. WebSocket upgrades retain
their byte stream, subprotocol and close handshake for development-server HMR.

## Validation

```sh
cargo test --locked -p rcode-preview
```

Tests cover process/cwd isolation, HTTP filtering, service disappearance, alias
persistence, port changes, concurrent streams, cancellation, body streaming,
redirects and WebSocket traffic. Native browser validation requires a supported
platform and display. The Vite fixture setup is documented in
[current development validation](development-validation.md).
