# The frame around a conversation: sidebar, tabs, the window's own chrome.

shell-new-session = New session
shell-search-placeholder = Search sessions
shell-empty-title = No sessions yet
shell-empty-body = Start one to begin.
shell-archive = Archive
shell-unarchive = Restore
shell-scope-local = Local

# The boot splash's status line, shown before any window has content.
shell-boot-status = Setting up RCode environment

# The native menu bar. Built once at startup, right after the language is
# fixed, so the labels are the ones the process will keep.
shell-menu-about = About RCode
shell-menu-check-for-updates = Check for Updates…
shell-menu-settings = Settings
shell-menu-services = Services
shell-menu-hide = Hide RCode
shell-menu-hide-others = Hide Others
shell-menu-show-all = Show All
shell-menu-quit = Quit RCode
shell-menu-edit = Edit
shell-menu-undo = Undo
shell-menu-redo = Redo
shell-menu-cut = Cut
shell-menu-copy = Copy
shell-menu-paste = Paste
shell-menu-select-all = Select All
shell-menu-view = View
shell-menu-appearance-system = Appearance: System
shell-menu-appearance-light = Appearance: Light
shell-menu-appearance-dark = Appearance: Dark
shell-menu-window = Window
shell-menu-minimize = Minimize
shell-menu-zoom = Zoom
shell-menu-close-window = Close Window

# The settings nav rail. These name the pages the rail opens; each page's own
# heading is a `settings-*` key. The rail's element ids come from
# `SettingsSection::slug`, never from these labels, so re-wording one cannot
# move a test selector.
shell-settings-nav-general = General
shell-settings-nav-appearance = Appearance
shell-settings-nav-notifications = Notifications
shell-settings-nav-shortcuts = Shortcuts
shell-settings-nav-providers = Model settings
shell-settings-nav-skills = Skills
shell-settings-nav-subagents = Subagents
shell-settings-nav-mcp = MCP
shell-settings-nav-plugins = Plugins
shell-settings-nav-personalization = Personalization
shell-settings-nav-devices = Devices
shell-settings-nav-files = Files
shell-settings-nav-appshots = Appshots
shell-settings-nav-archived = Archived sessions

# The composer's pickers: the model, project and ref lists, and the compact
# chip strip. The names in them — models, projects, branches — are the user's
# data and are not translated.
pickers-no-project = Don't work in a project
pickers-no-project-selected = No project selected
pickers-new-project = New project
pickers-no-refs = No refs found.
pickers-no-agents = No agents available
pickers-no-agents-hint = Enable an agent in Settings → Providers.
pickers-add-model = Add model
pickers-default = Default
pickers-fast-mode = Fast mode
pickers-reasoning-effort = Reasoning effort

# The session card: where this chat runs, its side-chat actions, and its
# device/project/branch names. Names are the user's data and are not
# translated.
shell-session-unknown-device = Unknown device
shell-session-no-project = No project
shell-session-new-side-chat = New side chat
shell-session-fork-chat = Fork chat
shell-session-actions = Actions

shell-engine-offline = Engine not connected

# Custom sidebar sections.
shell-sidebar-syncing = Sidebar is still syncing. Try again shortly.
shell-sidebar-section-placeholder = Section name
shell-sidebar-section-options = Section options
shell-sidebar-drop-here = Drop sessions here
shell-sidebar-edit-section = Edit section
shell-sidebar-new-section = New section
shell-sidebar-archive-all = Archive all
shell-sidebar-group-hint = Group sessions however you like
shell-sidebar-create-section = Create section
shell-sidebar-archive-failed = { $count ->
    [one] Could not archive { $count } session. Try again.
   *[other] Could not archive { $count } sessions. Try again.
}

# The drag targets over the composer.
shell-drop-to-attach = Drop to attach
shell-drop-to-mention = Drop to mention
# Closing the last window ends the process on Windows and Linux — and with it
# every run that was streaming. (On macOS the window is only a viewport: the
# engine keeps running, so closing it asks nothing.)
shell-exit-busy-title = Sessions are still running
shell-exit-busy-body = Closing RCode stops them and anything they are still working on. Close anyway?
shell-exit-busy-confirm = Close anyway

# Sidebar and workspace actions that had no localized string yet.
shell-onboarding-add-project = Add a project to get started
shell-onboarding-add-project-detail = A project is a folder on one of your devices.
shell-drop-to-unpin = Drop here to unpin
shell-star-on-github = Star on GitHub
shell-scroll-to-bottom = Scroll to bottom
shell-run-failed = Run failed
shell-sending = Sending…
shell-files-tab = Files
shell-diffs-tab = Diffs
shell-history-tab = History
shell-back = Back
shell-sidebar-view-options = Sidebar view options
# The sidebar view options menu: its three submenus, then one row per choice.
shell-sidebar-view-group-organize = Organize
shell-sidebar-view-group-sort = Sort
shell-sidebar-view-group-show = Show
shell-sidebar-view-by-project = By project
shell-sidebar-view-one-list = None
shell-sidebar-view-last-updated = Last updated
shell-sidebar-view-created = Created
shell-sidebar-view-branch = Branch
shell-sidebar-view-pull-request = Pull request
shell-sidebar-view-harness = Harness
shell-sidebar-view-project-icon = Project icon
shell-sidebar-view-project-label = Project name
shell-sidebar-view-project-filter = Project filter
shell-sidebar-view-archived = Archived
shell-in-home = In Home
shell-compact = Compact
shell-create-section = Create Section
shell-all-projects = All projects
shell-new-project = New project
shell-loading-locations = Loading locations…
shell-show-hidden-folders = Show hidden folders
shell-rename-menu = Rename…
shell-remove-menu = Remove…

# Ref trigger and switching labels in the composer's ref picker.
pickers-select-ref = Select ref
pickers-from-ref = From { $ref }
pickers-no-ref = No ref
pickers-switching = switching…

# Accessible names for icon-only controls.
shell-aria-settings-sections = Settings sections
shell-aria-close-tab = Close tab
shell-aria-new-chat-in-project = New chat in project
shell-aria-add-project = Add project

# Fallback labels when the registry has not supplied a name yet.
shell-unknown-device = Unknown device
shell-new-session-fallback = New session

# Subagent pane: stopping one child without stopping the conversation.
shell-subagent-stop = Stop subagent
shell-subagent-stop-aria = Stop subagent { $name }
shell-subagent-already-finished = That subagent already finished.
shell-subagent-stop-failed = Could not stop the subagent: { $error }

# Search-box placeholders, one per picker.
pickers-search-placeholder = Search…
pickers-search-refs = Search refs…
pickers-search-projects = Search projects…
pickers-search-models = Search models…
pickers-search-locations = Search locations…
pickers-search-folders = Search folders…
# The sidebar's grouped list header.
shell-sessions-header = Sessions
settings-worktrees-home = Home
settings-worktrees-use-this-folder = Use this folder
settings-worktrees-browse = Browse
surface-chrome-close-tab-aria = Close tab
files-workspace-files-aria = Workspace files
files-refresh-workspace-aria = Refresh workspace files now
settings-appearance-math-aria = Render math
settings-worktrees-use-default-aria = Use the default worktree location
settings-worktrees-choose-aria = Choose worktree folder
settings-worktrees-custom-aria = Custom worktree location
session-info-uncommitted = Uncommitted changes
shell-pull-requests = Pull requests
surface-chrome-new-tab = New tab
surface-chrome-close-tab = Close tab
settings-worktrees-reset-to-default = Reset to default
# Sidebar session status labels (the row's status glyph and its accessible name).
shell-status-working = Working
shell-status-input = Waiting for you
shell-status-failed = Failed
shell-status-queued = Queued
shell-status-done = Done
shell-status-idle = Idle
# The pin write was refused because the registry snapshot has not arrived yet.
shell-pins-syncing = Pins are still syncing
shell-pin = Pin
shell-unpin = Unpin
shell-remote-session = Remote session
shell-files-fallback-title = Files
shell-unrelated-archive-error = Unrelated archive error
shell-side-chats = Side chats
# Session-card lane controls.
shell-lanes-collapse-finished = { $count ->
    [one] Collapse { $count } finished subagent
   *[other] Collapse { $count } finished subagents
}
shell-lanes-expand-finished = { $count ->
    [one] Expand { $count } finished subagent
   *[other] Expand { $count } finished subagents
}
shell-lanes-finished-count = { $count ->
    [one] { $count } finished
   *[other] { $count } finished
}
# Project location picker and project management dialogs.
shell-project-home = Home
shell-project-no-locations = No locations found
shell-project-no-locations-hint = Try Home or a drive name.
shell-project-no-folders-here = No folders here
shell-project-no-folders-here-hint = Add this folder with { $shortcut }, or go back with ←.
shell-project-searching-home = Searching Home…
shell-project-searching-home-hint = Looking for matching folders anywhere under Home.
shell-project-no-home-matches-yet = No folders match yet
shell-project-home-indexing-hint = Home is still being indexed — keep typing or try again.
shell-project-no-folder-matches = No folders match
shell-project-no-folder-matches-hint = Type a path like ~/code or /mnt to jump there.
shell-project-back-to-commands = Back to commands
shell-project-open = Open
shell-project-select = Select
shell-project-navigate = Navigate
shell-project-close = Close
shell-project-adding = Adding…
shell-project-add = Add project
shell-project-name-placeholder = Project name
shell-project-rename-title = Rename project
shell-project-remove-title = Remove project?
shell-project-remove-fallback = this project
shell-project-remove-confirmation-one = Removing “{ $name }” permanently deletes its 1 session. This can’t be undone.
shell-project-remove-confirmation-many = Removing “{ $name }” permanently deletes its { $count } sessions. This can’t be undone.
shell-project-remove-confirm = Remove
shell-tabs-collapse-panel = Collapse panel
shell-tabs-expand-panel = Expand panel
shell-tabs-hide-files = Hide files panel
shell-tabs-show-files = Show files panel
shell-tabs-hide-session-details = Hide session details
shell-tabs-show-session-details = Show session details
shell-tabs-hide-pull-requests = Hide pull requests panel
shell-tabs-show-pull-requests = Show pull requests panel
shell-pinned = Pinned
shell-archived = Archived
shell-new-chat-in-project = New chat in project
shell-clear-project-filter-to-rename = Clear the project filter to rename this session
shell-searching-home-status = Searching…
shell-indexing-home-status = Indexing…
shell-home-folder-aria = { $name } in { $parent }
shell-settings-open-tooltip = Settings · { $shortcut }
shell-settings-close-tooltip = Close settings · { $shortcut }
shell-sync-queued = Sync queued — changes are saved
shell-syncing = Syncing…
shell-sync-offline = Offline — changes are saved
shell-sync-storage-error = Changes could not be saved
shell-sync-offline-saved = Offline — sends are saved
shell-sync-reconnecting = Reconnecting…
shell-aria-account-menu = Account menu: { $name }
shell-aria-settings = Settings
shell-aria-close-settings = Close settings
memory-empty = No memories yet
memory-list-truncated = File list truncated.

pickers-models-empty = No matching models

pickers-context-window = Context window
pickers-context-standard = Standard
pickers-change-model = Change model
pickers-reasoning-keys = Use Left and Right to adjust reasoning; Home and End select the first and last levels.
pickers-reasoning-minimal = Minimal
pickers-reasoning-low = Low
pickers-reasoning-medium = Medium
pickers-reasoning-high = High
pickers-reasoning-xhigh = X-High
pickers-reasoning-max = Max
pickers-reasoning-ultra = Ultra
pickers-reasoning-ultracode = Ultracode
pickers-reasoning-ultrathink = Ultrathink

shell-subagent-background = Run in background

shell-chat-delete-title = Delete session?
shell-chat-delete-body = “{ $title }” will be permanently deleted. This can’t be undone.
shell-chat-delete-disconnected = Can’t delete the session while the engine is disconnected.
shell-chat-delete-failed = Couldn’t delete the session: { $error }
shell-archived-delete-title = Delete all archived sessions?
shell-archived-delete-all = Delete all
shell-archived-delete-body = { $count ->
    [one] 1 archived session will be permanently deleted. This can’t be undone.
   *[other] { $count } archived sessions will be permanently deleted. This can’t be undone.
}
shell-archived-delete-failed = { $count ->
    [one] Couldn’t delete 1 archived session. Try again.
   *[other] Couldn’t delete { $count } archived sessions. Try again.
}

# Remaining UI display text. Stable identifiers are kept in code.
audit-command-new-chat = New chat
audit-command-new-project = New project
audit-command-settings = Open settings
audit-command-theme-system = Switch to system theme
audit-command-theme-light = Switch to light theme
audit-command-theme-dark = Switch to dark theme
audit-command-archive = Archive thread
audit-palette-all = All
audit-palette-threads = Threads
audit-palette-commands = Commands
audit-palette-files = Files
audit-palette-recent-files = Recent Files
audit-palette-quick-actions = Quick Actions
audit-palette-recent-threads = Recent Threads
audit-palette-in-files = In Files
audit-palette-indexing = Indexing…
audit-palette-searching = Searching…
audit-palette-search = Search threads, commands and files…
audit-palette-empty = No results
audit-palette-empty-hint = Try a command, chat title, project, or device.
audit-palette-update-content = Update RCode on the session’s device to search file contents.
audit-palette-update-files = Update RCode on the session’s device to search files.
audit-palette-unreachable = The session's device is unreachable
audit-palette-failed = Search failed
audit-notify-finished = Run finished
audit-notify-input = Waiting on your input
audit-notify-failed = Run failed
audit-notify-offline = Your device is offline
audit-notify-reconnecting = RCode is trying to reconnect
audit-notify-unavailable = Connection unavailable
audit-copy-conversation-link = RCode conversation link
audit-copy-session-id = Harness session ID
audit-copy-conversation-done = RCode conversation link copied
audit-copy-conversation-unready = Conversation link is not ready yet
audit-copy-session-done = Harness session ID copied
audit-copy-path-done = Path copied
audit-side-chat = Side chat
audit-side-chat-rename-parent = Open this side chat's parent to rename it
audit-session-title = Session title
audit-engine-disconnected = Engine not connected
audit-engine-disconnected-period = Engine is not connected.
audit-add-project = Add a project
audit-unarchive = Unarchive
audit-archive = Archive
audit-session-time = Session time
audit-discard-title = Discard working tree changes?
audit-discard-body = Discard all uncommitted changes in this working tree? This can’t be undone.
audit-discard = Discard changes
audit-discard-failed = Couldn’t discard changes
audit-copy-path = Path
audit-copy-named = { $name } copied
audit-unsaved-tab = { $name }, unsaved changes
audit-action-cancel = Cancel
audit-action-close = Close
audit-action-ok = OK
audit-action-later = Later
audit-action-hide = Hide
audit-update-checking = Checking for updates…
audit-update-check-failed = Couldn't check for updates
audit-update-current = RCode is up to date
audit-update-background = Downloading the update in the background — you can keep working.
audit-update-restart-body = Restart to finish updating. If you don't, it installs the next time you quit RCode.
audit-update-cli = Run `rcode update` in a terminal to install it.
audit-update-unmanaged = This copy of RCode wasn't set up by an installer (for example, a source build), so it can't update itself.
audit-try-again = Try again
audit-download = Download
audit-update-restart = Restart to update
audit-update-download-page = Open download page
audit-update-your-version = You’re on RCode { $version }.
audit-update-newest = Version { $version } is the newest release.
audit-update-available = RCode { $version } is available
audit-update-on-version = You’re on { $version }.
audit-update-ready-title = RCode { $version } is ready
audit-update-download-failed = The download failed: { $error }
audit-tab-close = Close tab
audit-tab-others = Close other tabs
audit-tab-left = Close tabs to the left
audit-tab-right = Close tabs to the right
audit-side-chat-start-first = Start a conversation before creating a side chat.
audit-pins-unconfirmed = The engine did not confirm the sidebar changes
audit-pins-offline = Engine not connected. Sidebar was not changed.
audit-pins-waiting = Waiting for the engine to confirm the previous sidebar change.
audit-pins-cancelled = Couldn't confirm sidebar changes. Queued edits were cancelled; waiting for the engine before allowing more changes.
audit-pins-failed = Couldn’t save sidebar changes: { $error }
audit-code-scroll = Use horizontal scrolling
audit-code-fit = Fit content
audit-diagram-complex = Diagram exceeds preview complexity limit
audit-diagram-large = Diagram output exceeds preview size limit
audit-diagram-failed = Diagram could not be rendered
audit-more-lines = … { $count ->
    [one] 1 more line
   *[other] { $count } more lines
}
audit-models-loading = Loading models…
audit-models-no-agents = No agents available
audit-models-select = Select model
audit-history-unknown = Unknown
audit-image = Image
audit-update-strip-available = Update available — v{ $version }
audit-update-strip-downloading = Downloading v{ $version }…
audit-update-strip-failed = Update failed: { $error }
audit-update-strip-cli = Update available — v{ $version } · run `rcode update`
audit-update-strip-download = Update available — v{ $version } · download from GitHub
audit-update-strip-ready = Update ready — restart to apply
audit-update-restarting = Restarting…
audit-update-read-only = Move RCode to Applications and reopen it to enable updates.
audit-update-not-writable = RCode cannot replace its installation in { $path }.
audit-copy-codex-link = Codex conversation link
shell-processes-title = Background processes
shell-processes-stopping = Stopping
shell-processes-recovered = Recovered
shell-processes-stop-aria = Stop { $command }

shell-output-open = View output for { $command }
shell-output-running = Running
shell-output-failed = Failed
shell-output-stopped = Stopped
shell-output-exited = Finished
shell-output-copy = Copy
shell-output-file = Open log file
shell-output-pause = Pause following
shell-output-follow = Follow output
shell-output-retry = Retry
shell-output-empty = No output yet
shell-output-error = Could not read log: { $error }
shell-output-rotated = Log rotated; only recent output is retained.
