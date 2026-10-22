browser-title = Browser

browser-starting = Starting browser…
browser-cache-unavailable = Could not locate the browser cache directory.
browser-startup-failed = Could not start the browser: { $reason }. Check the WebKitGTK 4.1 runtime and display.
browser-startup-invalid = The browser returned an invalid startup response. Reopen this tab.
browser-startup-stopped = Browser initialization stopped unexpectedly. Reopen this tab.
browser-profile-invalid = The browser profile requires an absolute storage directory.
browser-profile-unwritable = Could not create or write the browser profile directory.
browser-profile-locked = The browser profile is in use or cannot be locked. Close the other RCode instance and reopen this tab.
browser-window-required = The browser requires a native Windows window.
browser-main-thread-required = The browser must be created on the main thread.
browser-parent-missing = The browser window is no longer available.
browser-temporary-logins = Logins are temporary on this macOS version. Use macOS 14 or later, or open this page in your default browser, to keep them.
browser-back = Back
browser-forward = Forward
browser-reload = Reload
browser-url-placeholder = Website or localhost:3000

# Chrome and toolbar.
browser-go-to-address = Go to address
browser-open-external = Open in default browser
browser-opens-externally = Opens in your default browser

# Address validation. Shown under the address bar; the rest of the
# address handling (scheme checks, link rejection) reports nothing to read.
browser-address-required = Enter a website or localhost address.
browser-address-invalid = This address contains invalid characters.
browser-address-not-valid = Enter a valid website or localhost address.
browser-address-scheme = Only http and https addresses are supported.
browser-address-credentials = Use an address without an embedded username or password.

# Empty tab: the preview list and the start state.
browser-running-locally = Running locally
browser-open = Open
browser-open-preview = Open { $name } preview
browser-finding-servers = Looking for dev servers…
browser-no-servers = Start a dev server in this project. It will appear here automatically, ready to open.
browser-connecting = Connecting to preview discovery…
browser-enter-address = Enter an address
browser-enter-website = Enter a website address
browser-or-enter-website = Or enter a website address
browser-empty-title = Preview your work
browser-empty-external-title = Opened in your browser
browser-empty-error-title = Couldn’t load this page
browser-empty-body = Preview your local app or keep a website beside your conversation.
browser-empty-external-body = Open a website or local app in your default browser. Embedded browsing is available on macOS and Linux.
browser-retry-page = Retry page
browser-remote-loopback = Localhost opens on this device. Open a detected preview from a new tab to reach your other device.

# The page's context menu on Linux, where the helper sends an action token and
# the chrome draws the row. A page-supplied option label is not here: it is the
# site's own text and is shown as sent.
browser-menu-open-link = Open link in new tab
browser-menu-copy-link = Copy link address
browser-menu-paste = Paste
browser-menu-select-all = Select all

# Failures. The reason is carried verbatim (a native host's own message).
browser-open-failed = Could not open this page: { $reason }
browser-error-file = This file can’t be previewed here. Open it in your default browser.
browser-error-connect = Check the address and make sure your server is running, then try again.
browser-error-interrupted = The connection was interrupted. Try loading this page again.
browser-error-unresponsive = The page stopped responding. Reload to continue.
browser-error-stopped = The browser stopped. Reload to continue.
browser-error-webview = The Microsoft Edge WebView2 Runtime is required to show pages here.
browser-error-helper = The browser helper stopped. Check that WebKitGTK 4.1 is installed, then reopen the tab.
browser-error-webkit = Could not start WebKitGTK: { $reason }. Install the WebKitGTK 4.1 runtime for your distribution.
