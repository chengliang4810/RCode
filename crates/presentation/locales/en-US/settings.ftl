settings-title = Settings
settings-appearance-effort-theme = Reasoning theme
settings-appearance-effort-accent = Follow theme
settings-appearance-effort-purple = Purple
settings-appearance-effort-terracotta = Terracotta
settings-appearance-effort-deep-blue = Deep blue
settings-appearance-effort-sky-blue = Sky blue
settings-appearance-effort-aurora = Aurora
settings-appearance-effort-silver = Silver
settings-appearance-effort-gold = Gold
settings-language = Language
settings-language-system = Follow system
settings-language-en = English
settings-language-zh = 简体中文
settings-language-restart = Takes effect after restarting.
# The confirmation that follows a language change: the language is fixed at
# startup, so the only way to see the new one is a fresh process.
settings-language-restart-title = Restart to switch language?
settings-language-restart-body = RCode picks its display language at startup. Restart now to see the new one; otherwise it applies the next time you open RCode.
settings-language-restart-action = Restart now
# Settings → General → Data location. The pointer file this row writes is read
# by the process at startup, so a change takes effect on restart; the config
# root (device id, lock, provider keys, settings) never moves with it.
settings-data-location = Data location
settings-data-location-default = Default
settings-data-location-detail-default = Sessions, uploads, cloned repositories and worktrees live in the config folder, beside your settings.
settings-data-location-detail-custom = Sessions, uploads, cloned repositories and worktrees live in the folder above. Your settings and provider keys stay in the config folder.
settings-data-location-change = Change…
settings-data-location-use-default = Use default location
settings-data-location-restart-title = Restart to use the new location?
settings-data-location-restart-body = RCode reads the data location at startup. Restart now to move the stores; otherwise it applies the next time you open RCode.
settings-data-location-restart-action = Restart now
settings-data-location-rejected = That folder cannot be used: { $reason }
settings-memory = Remember across sessions
settings-skills = Skills
settings-skills-empty = No skills found
settings-hooks = Hooks

# Settings → Notifications. The toggles are switches, so each row title is
# also the switch's accessible name; only the sub-options need their own.
settings-notifications-title = Notifications
settings-notifications-desktop-section = Desktop
settings-notifications-sounds-section = Sounds
settings-notifications-session-sounds = Session sounds
settings-notifications-completed = Task completed
settings-notifications-completed-toggle = Task completed sound
settings-notifications-input = Input required
settings-notifications-input-toggle = Input required sound
settings-notifications-errors = Errors and disconnections
settings-notifications-errors-toggle = Errors and disconnections sound
settings-notifications-desktop = Desktop notifications
settings-notifications-background = Only when in the background
settings-notifications-background-toggle = Only notify when RCode is in the background
settings-notifications-unavailable = Unavailable while its parent setting is off

# Settings → Appearance. The theme, accent and wallpaper rows. Theme names and
# accent preset names are the theme crate's data and keep their own spelling;
# only the labels around them are here.
settings-appearance-title = Appearance
settings-appearance-color-scheme = Color scheme
settings-appearance-material = Material and background
settings-appearance-motion = Motion
settings-appearance-fonts = Fonts and layout

settings-appearance-conversation-width = Conversation width
settings-appearance-conversation-width-detail = Maximum width for messages and the composer.
# The slider's two endpoint labels: a number and a unit, whose grouping is the
# only part a locale changes.
settings-appearance-width-min = 560 px
settings-appearance-width-max = 1,200 px

settings-appearance-font-interface = Interface font
settings-appearance-font-interface-detail = Menus and conversations
settings-appearance-font-terminal = Terminal font
settings-appearance-font-terminal-detail = Terminal output · monospace only
settings-appearance-font-code = Code & diff font
settings-appearance-font-code-detail = Code, diffs, and files
settings-appearance-font-size = { $font } size
settings-appearance-font-picker-aria = { $font }: { $value }
settings-appearance-font-search = Search fonts
settings-appearance-font-no-match = No matching fonts
settings-appearance-font-none = No fonts
settings-appearance-font-missing = { $font } "{ $requested }" isn't available on this device. Using { $effective }.

settings-appearance-light-theme = Light theme
settings-appearance-dark-theme = Dark theme
settings-appearance-light-themes = Light themes
settings-appearance-dark-themes = Dark themes
settings-appearance-variant-light = Light
settings-appearance-variant-dark = Dark

settings-appearance-accent = Accent color
settings-appearance-accent-wallpaper = Wallpaper colors are enabled; this accent is used when they are off.
settings-appearance-glass = Glass
settings-appearance-surface-theme-default = Theme default
settings-appearance-surface-frosted = Frosted
settings-appearance-surface-opaque = Opaque
settings-appearance-surface-default-frosted = Theme default: frosted
settings-appearance-surface-default-opaque = Theme default: opaque
settings-appearance-surface-translucent = Translucent surfaces
settings-appearance-surface-solid = Solid surfaces
settings-appearance-match-wallpaper = Match wallpaper colors
settings-appearance-match-wallpaper-detail = Use wallpaper colors for accents, highlights, and subtle surface tints.

settings-appearance-reduce-motion = Reduce motion
settings-appearance-reduce-motion-system-on = Following reduced system motion. Activity indicators use a gentle brightness pulse.
settings-appearance-reduce-motion-system-off = Following the system, which currently allows motion.
settings-appearance-reduce-motion-on = Animations skip straight to their final state.
settings-appearance-reduce-motion-off = Animations play even if the system asks for less motion.
settings-appearance-pause-animations = Pause animations in background
settings-appearance-pause-animations-detail = Hold animations still while RCode isn't the focused window.

settings-appearance-background = New thread background
settings-appearance-background-adjust = Adjust
settings-appearance-background-replace = Replace image
settings-appearance-background-remove = Remove
settings-appearance-background-choose = Choose image
settings-appearance-background-unavailable = Image unavailable
settings-appearance-background-choose-replacement = Choose a replacement or remove it.
settings-appearance-background-none = No image selected
settings-appearance-choose-background = Choose New Thread Composer Background

settings-appearance-adjust-background = Adjust background
settings-appearance-adjust-background-detail = Drag to reposition. Scroll or pinch to zoom.
settings-appearance-close-adjustment = Close background adjustment
settings-appearance-background-zoom = Background zoom
settings-appearance-background-changed = Background changed. Close and adjust the new image.
settings-appearance-background-preparing = Preparing preview…
settings-appearance-apply = Apply

settings-appearance-wallpaper-folder = Wallpaper folder
settings-appearance-wallpaper-folder-none = Choose a folder of wallpaper images.
settings-appearance-wallpaper-folder-detail = { $folder } · { $shortcut } picks a random image
settings-appearance-choose-folder = Choose folder
settings-appearance-choose-wallpaper-folder = Choose Wallpaper Folder
settings-appearance-shuffle = Shuffle
settings-appearance-background-effect = Background effect

settings-appearance-theme-library = Theme library
settings-appearance-add-theme = Add theme
settings-appearance-imported = Imported
settings-appearance-linked = Linked
settings-appearance-self-contained = Self-contained snapshot
settings-appearance-variant-count = { $count ->
    [one] { $count } variant
   *[other] { $count } variants
}
settings-appearance-theme-status = { $source } · { $variants } · { $path }
settings-appearance-theme-warning = Using last known good · { $message }
settings-appearance-theme-reload = Reload
settings-appearance-theme-reveal = Reveal
settings-appearance-theme-review = Review
settings-appearance-theme-duplicate = Duplicate as editable
settings-appearance-theme-unlink = Unlink
settings-appearance-theme-remove = Remove
settings-appearance-theme-mapping = Theme mapping
settings-appearance-done = Done

settings-appearance-import-title = Add a theme
settings-appearance-import-detail = Import a local theme into your library or keep it linked to its source.
settings-appearance-import-source = Source
settings-appearance-import-source-placeholder = Theme file, package.json, or extension folder
settings-appearance-import-source-required = Choose a local theme file or extension folder.
settings-appearance-import-choose-source = Choose Theme Source
settings-appearance-import-browse = Browse…
settings-appearance-import-keep-updated = Keep it up to date
settings-appearance-import-copy = Import a copy
settings-appearance-import-copy-detail = Works independently from the original file.
settings-appearance-import-link = Link to source
settings-appearance-import-link-detail = Reload changes from the file on disk.
settings-appearance-import-detected = Detected themes
settings-appearance-import-select-variant = Select at least one variant to import.
settings-appearance-import-details = Details
settings-appearance-import-hide-details = Hide details
settings-appearance-import-analyze = Analyze theme
settings-appearance-import-selected = Import selected
settings-appearance-import-auto-detect = RCode finds light and dark variants automatically.
settings-appearance-import-compile-failed = { $name } could not be compiled · { $message }

# The mapping report: a diagnostic listing of what the importer did. The role
# names, source keys and messages in it come from the theme crate and are not
# translated — only the labels introducing them are.
settings-appearance-report-summary = { $mapped } mapped · { $adjusted } adjusted · { $fallback } inferred/fallback · { $unsupported } unsupported · { $warnings } warnings · { $validation } validation
settings-appearance-report-adjusted = Adjusted · { $role } { $original } → { $resolved } · { $reason }
settings-appearance-report-fallback = Fallback · { $message }
settings-appearance-report-warning = Warning · { $message }
settings-appearance-report-unsupported = Unsupported · { $message }
settings-appearance-report-validation = Validation { $category } { $severity } · { $message }
settings-appearance-report-mapping = { $role } ← { $key }

# Settings → General, and the shortcut table.
settings-general-title = General
settings-shortcuts-title = Keyboard shortcuts
settings-shortcuts-subtitle = Click a binding, then press the new key combination.
settings-shortcuts-restore = Restore defaults
settings-shortcuts-unique = Shortcuts must be unique.
settings-shortcuts-press-escape = Press Escape to cancel.
settings-shortcuts-press-keys = Press keys…
settings-shortcuts-reset-aria = Reset { $name } shortcut
settings-shortcuts-change-aria = Change { $name } shortcut: { $combo }
settings-shortcuts-navigate-pane = Navigate within the focused pane.
settings-shortcuts-send-with = Send messages with
settings-shortcuts-compact-mode = Compact mode
settings-shortcuts-compact-mode-detail = Collapse thinking and tools.
settings-shortcuts-compact-picker = Compact model picker
settings-shortcuts-compact-picker-detail = Adjust effort with a slider, then open the model list when needed.
settings-shortcuts-escape = Stop agent with Escape
settings-shortcuts-escape-detail = When no dialog or menu is open.
settings-shortcuts-escape-aria = Stop active agent with Escape
settings-shortcuts-refused-key = Use { $modifiers } with a letter, number, function key or navigation key.
settings-shortcuts-modifiers-platform = Control, Option or Command
settings-shortcuts-modifiers-other = Control or Alt
settings-shortcuts-refused-reserved = { $combo } is reserved for the composer.
settings-shortcuts-refused-taken = { $combo } is already assigned to { $owner }.
settings-shortcuts-refused-taken-page = { $combo } is already assigned to { $owner } in Settings → { $page }.
settings-links = Links
settings-links-pull-requests = Open pull requests in
settings-links-pull-requests-detail = Applies to PR badges and the Pull requests board.

# Shortcut table section headings. The keys come from the page's own grouping,
# which is stable; these are only what the heading reads.
settings-shortcuts-group-appearance = Appearance
settings-shortcuts-group-files = Files
settings-shortcuts-group-browser = Browser
settings-shortcuts-group-panels = Panels
settings-shortcuts-group-sessions = Sessions
settings-shortcuts-group-projects = Projects
settings-shortcuts-group-jump = Jump to session
settings-shortcuts-group-appshots = Appshots

# Settings → Files.
settings-files-title = Files
settings-files-autosave = Autosave
settings-files-autosave-delay = Autosave delay
settings-files-word-wrap = Word wrap
settings-files-show-hidden = Show hidden and ignored files

# Settings → Appshots. The capability descriptions come from the appshots
# module; these are the row titles and the capture-setup copy.
settings-appshots-title = Appshots
settings-appshots-capture = Capture Appshots
settings-appshots-capture-detail = Captures are staged for review and never sent automatically.
settings-appshots-sound = Capture sound
settings-appshots-sound-detail = Play a sound when an Appshot is ready.
settings-appshots-shortcut = Global shortcut
settings-appshots-shortcut-preferred = Preferred shortcut
settings-appshots-destination = Destination
settings-appshots-destination-auto = Use the open session, or the new-session composer when no session is open.
settings-appshots-destination-last = Use the open session, or return to the last session used for an Appshot.
settings-appshots-destination-new = Stage captures in a new-session composer, keeping existing drafts intact.
settings-appshots-window-capture = Window capture
settings-appshots-application-text = Application text
settings-appshots-open-settings = Open System Settings
settings-appshots-allow-window = Allow window capture
settings-appshots-enable-text = Enable text capture
settings-appshots-check-again = Check again
settings-appshots-check-again-aria = Check permissions again
settings-appshots-check-hint = Changed a permission? Check again after returning to RCode.

# Settings → Devices.

# Settings → General → Memory and Thread naming.
settings-memory-section = Memory
settings-personalization-instructions = Custom instructions
settings-personalization-instructions-placeholder = Enter rules and preferences for the assistant, such as response language, code style, and workflow.
settings-marketplace = Plugin marketplace
settings-marketplace-official = Official marketplace
settings-marketplace-search = Search plugins
settings-marketplace-refresh = Refresh
settings-marketplace-cached = Showing the cached marketplace catalog
settings-marketplace-installed = Installed
settings-marketplace-installing = Installing…
settings-marketplace-unsupported = LSP not supported yet
settings-marketplace-empty = No matching plugins
settings-marketplace-load-failed = Could not load the plugin marketplace: { $error }
settings-marketplace-install-failed = Could not install plugin: { $error }
settings-marketplace-install-aria = Install plugin { $name }
settings-personalization-reload = Reload and discard unsaved changes
settings-personalization-load-failed = Could not load custom instructions: { $error }
settings-personalization-save-failed = Could not save custom instructions: { $error }
settings-memory-detail = Turning this off stops loading memory from the next message. Saved memories stay on disk.
settings-thread-naming = Thread naming
settings-thread-naming-reset-aria = Name threads with the session agent
settings-thread-naming-follows = Each thread is named by its own agent.
settings-thread-naming-model = A small model keeps titles fast and cheap.


# Settings → Skills. The two managed skill roots: the project's
# `.agents/skills` and the person's `~/.rcode/skills`. `settings-skills` and
# `settings-skills-empty` (above) name the page and its empty list.
settings-skills-subtitle = Markdown instructions the agent can load on demand. Project skills live in the project's `.agents/skills` folder; your skills live in your home folder and follow you across projects.
settings-skills-project-section = Project skills
settings-skills-user-section = Your skills
settings-skills-other-section = Other
settings-skills-new = New skill
settings-skills-new-aria = Create a skill
settings-skills-enable-aria = Enable { $name }
settings-skills-disable-aria = Disable { $name }
settings-skills-edit = Edit
settings-skills-edit-aria = Edit { $name }
settings-skills-delete-aria = Delete { $name }
settings-skills-empty-detail = Create a skill to give the agent a reusable instruction.
settings-skills-loading = Loading skills…
settings-skills-load-failed = Couldn't load skills: { $error }
settings-skills-create-title = New skill
settings-skills-edit-title = Edit skill
settings-skills-name-label = Name
settings-skills-name-placeholder = skill-name
settings-skills-description-label = Description
settings-skills-description-placeholder = What this skill does and when to use it
settings-skills-body-label = Instructions
settings-skills-body-placeholder = The instruction the agent follows when the skill is used
settings-skills-body-keeps-existing = Leave empty to keep the current instructions.
settings-skills-scope-label = Saved to
settings-skills-scope-project = Project
settings-skills-scope-user = Personal
settings-skills-scope-project-detail = Saved into this project's `.agents/skills` folder.
settings-skills-scope-user-detail = Saved into your skills folder, alongside every project.
settings-skills-name-invalid = Use letters, numbers, dashes and underscores.
settings-skills-description-required = A description is required: it is what the agent reads when deciding whether to use the skill.
settings-skills-create-failed = Couldn't create the skill: { $error }
settings-skills-update-failed = Couldn't save the skill: { $error }
settings-skills-delete-title = Delete skill?
settings-skills-delete-body = "{ $name }" will be permanently deleted. This can't be undone.
settings-skills-delete-failed = Couldn't delete the skill: { $error }
settings-skills-toggle-failed = Couldn't update { $name }: { $error }


# Settings → Subagents. The native agent's `Task` tool delegates to a
# definition file: the project's `.rcode/agents` or the person's
# `~/.rcode/agents`. The body is the child's system prompt; the model picker
# is the user's override on top of the definition's own `model:`.
settings-subagents = Subagents
settings-subagents-subtitle = Named agents the assistant can delegate a task to. A definition sets the child's instructions, the tools it may use, and the model it runs on.
settings-subagents-project-section = Project subagents
settings-subagents-user-section = Your subagents
settings-subagents-other-section = Other
settings-subagents-new = New subagent
settings-subagents-new-aria = Create a subagent
settings-subagents-enable-aria = Enable { $name }
settings-subagents-disable-aria = Disable { $name }
settings-subagents-edit-aria = Edit { $name }
settings-subagents-delete-aria = Delete { $name }
settings-subagents-empty = No subagents found
settings-subagents-empty-detail = Create a subagent to let the assistant hand a focused task to a fresh context.
settings-subagents-loading = Loading subagents…
settings-subagents-load-failed = Couldn't load subagents: { $error }
settings-subagents-create-title = New subagent
settings-subagents-edit-title = Edit subagent
settings-subagents-name-label = Name
settings-subagents-name-placeholder = agent-name
settings-subagents-description-label = When to use it
settings-subagents-description-placeholder = What this agent is for, in the words the model reads
settings-subagents-prompt-label = Instructions
settings-subagents-prompt-placeholder = The system prompt the subagent runs under
settings-subagents-tools-label = Allowed tools
settings-subagents-tools-placeholder = Read, Grep, Glob (empty allows every tool)
settings-subagents-disallowed-tools-label = Blocked tools
settings-subagents-disallowed-tools-placeholder = Bash, Write (applied after the allowlist)
settings-subagents-tools-summary = Tools: { $tools }
settings-subagents-disallowed-tools-summary = Blocked: { $tools }
settings-subagents-model-label = Model
settings-subagents-model-detail = A short alias, resolved against your providers. "inherit" runs on the conversation's model.
settings-subagents-model-aria = Change the model for { $name }, currently { $model }
settings-subagents-scope-project-detail = Saved into this project's `.rcode/agents` folder.
settings-subagents-scope-user-detail = Saved into your agents folder, alongside every project.
settings-subagents-name-invalid = Use letters, numbers, dashes and underscores.
settings-subagents-description-required = Describe when the assistant should delegate to this agent.
settings-subagents-create-failed = Couldn't create the subagent: { $error }
settings-subagents-update-failed = Couldn't save the subagent: { $error }
settings-subagents-delete-title = Delete subagent?
settings-subagents-delete-body = "{ $name }" will be permanently deleted. This can't be undone.
settings-subagents-delete-failed = Couldn't delete the subagent: { $error }
settings-subagents-toggle-failed = Couldn't update { $name }: { $error }
settings-subagents-model-failed = Couldn't change { $name }'s model: { $error }

# Presence label still used by device rows in state.rs.
settings-devices-never-seen = Never seen

# Shortcut row names (the group headings already have keys above).
settings-shortcut-name-random-wallpaper = Random wallpaper
settings-shortcut-name-capture-appshot = Capture Appshot
settings-shortcut-name-save-file = Save file
settings-shortcut-name-browser-reload = Reload browser page
settings-shortcut-name-toggle-sidebar = Toggle left sidebar
settings-shortcut-name-toggle-changes = Toggle right sidebar
settings-shortcut-name-toggle-files = Toggle files panel
settings-shortcut-name-toggle-terminal = Toggle terminal
settings-shortcut-name-new-session = New session
settings-shortcut-name-new-project = New project
settings-shortcut-name-open-model-picker = Open model picker
settings-shortcut-name-next-session = Next session or right pane tab
settings-shortcut-name-prev-session = Previous session or right pane tab
settings-shortcut-name-archive-session = Archive session
settings-shortcut-name-jump-session = Jump to session { $slot }
settings-shortcut-name-pull-request-view = Pull request view
settings-shortcut-name-default-browser = Default browser

# Settings → General → Links: where a pull request opens.
settings-pr-destination-view = Pull request view
settings-pr-destination-browser = Default browser

# Appshot destination select options.
settings-appshots-destination-automatic = Automatic
settings-appshots-destination-last-session = Last session
settings-appshots-new-session = New session

# Settings → MCP. The servers are configured in `mcp.json`, the shape other
# MCP clients use, so the page shows state rather than owning every field.
settings-mcp = MCP
settings-mcp-subtitle = Tools from Model Context Protocol servers. A server's entry is the same JSON other MCP clients read; the agent sees its tools as `mcp__server__tool`.
settings-mcp-project-section = Project servers
settings-mcp-user-section = Your servers
settings-mcp-empty = No MCP servers
settings-mcp-empty-detail = Choose “New MCP server” to add a server.
settings-mcp-loading = Loading servers…
settings-mcp-load-failed = Couldn't load MCP servers: { $error }
settings-mcp-write-failed = Couldn't save the server: { $error }
settings-mcp-configured = Configured
settings-mcp-not-connected = Not connected
settings-mcp-edit = Edit
settings-mcp-edit-title = Edit server
settings-mcp-json-placeholder = "command": "npx", "args": ["-y", "server"]
settings-mcp-json-invalid = That isn't valid JSON: { $error }

# Rows an installed plugin contributes; read-only.
settings-skills-plugin-section = From plugins
settings-subagents-plugin-section = From plugins

# Servers an installed plugin contributes; read-only.
settings-mcp-plugin-section = From plugins

# Settings → Plugins. A plugin is a directory with a manifest and the
# contribution folders beside it; the page manages its lifecycle, not its files.
settings-plugins = Plugins
settings-plugins-subtitle = Installed extensions. A plugin can add skills, subagents, hooks and MCP servers; its own files stay read-only here.
settings-plugins-empty = No plugins installed
settings-plugins-empty-detail = Install plugins from the plugin marketplace.
settings-plugins-loading = Loading plugins…
settings-plugins-load-failed = Couldn't load plugins: { $error }
settings-plugins-remove = Remove
settings-plugins-enable-aria = Enable { $name }
settings-plugins-disable-aria = Disable { $name }
settings-plugins-toggle-failed = Couldn't update { $name }: { $error }
settings-plugins-uninstall-failed = Couldn't remove the plugin: { $error }

# Appearance → Math.
settings-appearance-math-size-detail = Relative to the text around it.

settings-mcp-new = New MCP server
settings-mcp-form = Form
settings-mcp-name = Name
settings-mcp-transport = Transport
settings-mcp-command = Command
settings-mcp-args = Arguments
settings-mcp-env = Environment variables (JSON object)
settings-mcp-url = URL
settings-mcp-headers = HTTP headers (JSON object)
settings-mcp-json-label = Server configuration
settings-mcp-saving = Saving…
settings-mcp-name-required = Enter a server name, or include it in the JSON configuration.
settings-mcp-name-exists = A server with this name already exists.
settings-mcp-args-invalid = Use space-separated arguments with matching quotes, or a JSON array of strings.
settings-mcp-env-invalid = Environment variables must be a JSON object with string values.
settings-mcp-headers-invalid = Headers must be a JSON object with string values.
settings-mcp-object-required = The server configuration must be a JSON object.
settings-mcp-single-server = Add one MCP server at a time.
settings-mcp-config-invalid = Invalid server configuration: { $error }
settings-mcp-context-changed = The engine connection changed. Close this dialog and try again.

settings-subagents-inherit = Inherit default
settings-subagents-all-tools = All tools
settings-subagents-tool-count = { $count } tools
settings-subagents-group-count = { $count } items
settings-subagents-builtin = Built-in subagents

# Labels and errors used by the appearance and worktree controls.
audit-settings-appearance-system = System
audit-settings-appearance-light = Light
audit-settings-appearance-dark = Dark
audit-settings-motion-on = On
audit-settings-motion-off = Off
audit-settings-effect-none = None
audit-settings-effect-dither = Dither
audit-settings-effect-ascii = ASCII
audit-settings-effect-halftone = Halftone
audit-settings-effect-scanlines = Scanlines
audit-settings-effect-none-detail = Shows the original artwork.
audit-settings-effect-dither-detail = Rebuilds the artwork with a dithered color palette.
audit-settings-effect-ascii-detail = Recreates the artwork with colored characters on black.
audit-settings-effect-halftone-detail = Recreates the artwork with colored print dots on black.
audit-settings-effect-scanlines-detail = Adds a pronounced horizontal display-line texture.
audit-settings-math-font = Math font
audit-settings-math-size = Math size
audit-settings-math-render-detail = Typeset LaTeX in messages. Off shows the source as code.
audit-settings-math-sans = Sans
audit-settings-math-serif = Serif
audit-settings-math-latex = LaTeX
audit-settings-math-upright = Upright
audit-settings-math-fira-detail = A sans face that sits beside the interface font.
audit-settings-math-stix-detail = A Times-like serif face with the TeX look.
audit-settings-math-computer-modern-detail = A serif face with the classic LaTeX look.
audit-settings-math-libertinus-detail = A warm serif book face.
audit-settings-math-pagella-detail = A Palatino serif face, from TeX Gyre Pagella.
audit-settings-math-euler-detail = Zapf's upright, hand-drawn math.
audit-settings-custom-theme = Custom theme
audit-settings-theme-linked-file = Linked file
audit-settings-theme-linked-package = Linked package
audit-settings-theme-editable-file = Editable file
audit-settings-wallpaper-folder-unreadable = Unable to read the wallpaper folder. Choose an accessible folder in Appearance.
audit-settings-wallpaper-folder-empty = No readable wallpapers found. Add images such as PNG or JPEG, or choose another folder.
audit-settings-wallpaper-folder-required = Choose a wallpaper folder in Appearance first.
audit-settings-background-invalid = This image is unsupported or damaged. Choose a valid PNG or JPEG image.
audit-settings-background-save-unavailable = Unable to save the image. Restart RCode and try again.
audit-settings-background-save-failed = Unable to save the image. Check folder permissions and try again.
audit-settings-background-remove-unavailable = Unable to remove the image. Restart RCode and try again.
audit-settings-background-remove-failed = Unable to remove the image. Check folder permissions and try again.
audit-settings-worktrees-you = You
audit-settings-worktrees-device = Device
audit-settings-worktrees-disconnected = Device is not connected
audit-settings-worktrees-update-required = Update RCode on the selected device to configure worktree locations.
audit-settings-worktrees-up = Up
audit-settings-worktrees-no-subfolders = No subfolders
audit-settings-worktrees-more-folders = More folders exist than are shown here.
audit-settings-worktrees-loading-folders = Loading folders…
audit-settings-worktrees-choose-folder = Choose a folder for new worktrees
audit-settings-worktrees-load-failed = Couldn't load worktree settings
audit-settings-worktrees-environment-override = Set by RCODE_WORKTREES_DIR
audit-settings-worktrees-title = Worktrees
audit-settings-deleting = Deleting…



settings-subagents-model-deleted = Removed; inherits conversation

settings-commands = Commands
settings-commands-name = Name
settings-commands-body = Markdown prompt
settings-commands-new = New command
settings-commands-reload = Reload
settings-commands-delete = Delete command
settings-commands-edit = Edit
settings-commands-enable = Enable
settings-commands-disable = Disable
settings-commands-loading = Loading…
settings-commands-empty = No commands yet.
settings-commands-plugin = Plugin

settings-marketplace-previous = Previous
settings-marketplace-next = Next
settings-marketplace-page = Page { $page } of { $pages }

audit-settings-models-jump = Jump to { $name }
audit-settings-models-fast-on = Fast mode on · Turn off
audit-settings-models-fast-off = Fast mode off · Turn on
audit-settings-update-release-unavailable = The release feed no longer offers a newer version.
