files-title = Files
files-empty = This folder is empty
files-image-preview = Image preview
files-open-external = Open in default app
files-unsaved = Unsaved changes
files-save-failed = Could not save: { $reason }

# Explorer chrome and headers.
files-surface-label = Workspace files
files-label-projectless = Files in { $path } · { $device }
files-label-projectless-device = Files · { $device }
files-label-projectless-path = Files in { $path }
files-refresh-now = Refresh now
files-refresh-aria = Refresh workspace files now
files-retry = Retry
files-search-placeholder = Search files
files-hide-ignored = Hide hidden and ignored files
files-show-all-files = Show all files (even hidden)
files-mutation-changed = Workspace changed before operation completed

# Empty and loading states. The file tree shows one per row; the search
# results show their own; the preview shows a centered state.
files-empty-folder = Empty folder
files-loading = Loading…
files-loading-file = Loading file…
files-loading-preview = Loading preview…
files-loading-image = Loading image…
files-load-more = Load more…
files-no-workspace = No workspace available for this chat.
files-service-starting = Workspace service is still starting.
files-service-unavailable = Workspace service is unavailable.

# Workspace file search.
files-searching = Searching…
files-no-files-found = No files found.
files-search-results-label = Fuzzy workspace file results
files-search-limit = Showing the first { $count } matches

# Tree, context menus and inline rename/delete.
files-file-tree-label = Workspace file tree
files-menu-add-to-chat = Add to chat
files-menu-copy-path = Copy path
files-menu-copy-paths = Copy paths
files-menu-copy-full-path = Copy full path
files-menu-copy-full-paths = Copy full paths, one per line
files-menu-rename = Rename…
files-menu-delete = Delete…
files-menu-delete-many = Delete { $count } items…
files-tree-error-retry = { $message } — Retry
files-rename-invalid-name = Enter a single file name without path separators
files-rename-entry-changed = Entry changed; cancel and reopen Rename
files-delete-entry-changed = Entry changed; reopen Delete to confirm its current contents
files-delete-title = Delete permanently?
files-delete-message-directory = Permanently delete { $name }? All current folder contents will be deleted. Open editor buffers will be kept for recovery.
files-delete-message-file = Permanently delete { $name }? This cannot be undone. Open editor buffers will be kept for recovery.
files-delete-message-many-directories = Permanently delete { $names }? Folders are deleted with all their current contents. Open editor buffers will be kept for recovery.
files-delete-message-many-files = Permanently delete { $names }? This cannot be undone. Open editor buffers will be kept for recovery.
files-drag-move-to-root = Move to workspace root
files-drag-workspace-root = Workspace root

# Editor context menu.
files-editor-cut = Cut
files-editor-paste = Paste
files-editor-select-all = Select All

# Preview read-only and error states. A message with no key of its own falls
# back to files-cannot-preview; the failed read keeps the transport's own
# wording except for a missing file, which names itself.
files-file-not-found = File not found.
files-cannot-preview = This file cannot be previewed.
files-readonly-binary = Binary files cannot be previewed.
files-readonly-encoding = This file encoding is not supported.
files-readonly-symlink = Symlink targets are read-only.
files-readonly-permission = Permission denied.
files-readonly-too-large = This file is too large to preview.
files-readonly-mixed-endings = Files with mixed line endings are read-only.
files-readonly-outside = Read-only: outside this chat's folder.
files-truncated-readonly = Large file preview is truncated and read-only.
files-image-removed = This image was removed from the workspace.
files-checkout-unavailable = Workspace checkout identity unavailable
files-image-timeout = Image preview timed out
files-image-too-large = Image exceeds preview memory limit
files-image-preview-suspended = Workspace changed. Image preview suspended.

# Preview lifecycle banners: unsaved edits while closing or switching target,
# and a file that changed on disk underneath the editor.
files-changed-banner = Workspace changed. Switch back to save, or discard these edits.
files-changes-not-saved = Changes could not be saved safely.
files-saving-before-close = Saving changes before closing…
files-keep-open = Keep Open
files-discard-changes = Discard Changes
files-keep-editing = Keep Editing
files-reload-from-disk = Reload from Disk
files-discard-reload = Discard & Reload
files-discard-unsaved = Discard unsaved changes?
files-changed-outside = This file changed outside RCode.

# The editor breadcrumb's save-status chip.
files-save-failed-label = Save failed
files-save-conflict = Save conflict
files-conflict-detail = The file changed on disk. Your editor buffer was preserved.
files-deleted-on-disk = Deleted on disk
files-deleted-detail = The file was removed on disk. Your editor buffer was preserved.
files-changed-on-disk = Changed on disk
files-changed-detail = The file changed on disk. Review it before saving.

# Editor toolbar buttons.
files-show-markdown-code = Show Markdown code
files-preview-markdown = Preview Markdown
files-save-file-aria = Save file
files-reveal-in-tree = Reveal file in tree
files-open-default-editor = Open in default editor
files-disable-word-wrap = Disable word wrap
files-enable-word-wrap = Enable word wrap

# Editor review comments.
files-remove-comment = Remove comment
files-comment-placeholder = Request a change…
files-comment-placeholder-add = Add a comment…
files-comment = Comment
files-open-comment-aria = Open comment on line { $line }
files-comment-line-aria = Comment on line { $line }

# Markdown preview media.
files-media-connection-unavailable = Workspace image connection unavailable
files-image-outside-workspace = Image path is outside the workspace
files-media-memory-limit = Document media preview memory limit reached
files-mermaid-diagram = Mermaid diagram
files-rendering-diagram = Rendering diagram…
files-diagram-limit = Document diagram preview limit reached
files-open-image-link = Open image link
files-image-limit = Document image preview limit reached

# The explorer's footer sections.
files-section-subagents = Subagents
files-section-chats = Chats
files-new-side-chat = New side chat
files-fork-this-chat = Fork this chat
files-fork = Fork
files-collapse = Collapse
files-expand = Expand
files-subagents-empty = Subagents will appear here when they are created
files-side-chats-empty = Side chats will appear here when they are created
files-open-subagent-aria = Open subagent { $title }
files-open-side-chat-aria = Open side chat { $title }
files-section-toggle-aria = { $action } { $label }
files-group-header-aria = { $action } { $label } subagents
files-show-more = Show { $count } more
files-group-finished = Finished
files-group-completed = Completed
files-group-failed = Failed

# The Git-status notice row in the explorer.
files-git-unavailable-incomplete = Git status unavailable or incomplete
files-git-unavailable = Git status unavailable

# The rail text and staging errors for attachments. The attachment-only body
# and the refs trailer that ride the prompt are NOT here: they are the
# persisted message text, not display copy.
files-rail-attached-image = { $count ->
    [one] Attached image
   *[other] { $count } attached images
}
files-rail-attached-file = { $count ->
    [one] Attached file
   *[other] { $count } attached files
}
files-attach-not-image = { $name } is not a valid image.
files-attach-unreadable = { $name } could not be read.
files-attach-not-file = { $name } is not a file.
files-attach-too-large = { $name } is too large (24 MB max).
files-attach-unsupported = { $name } is not a supported image.

# Decode failures from the shared image path. They reach the same error slot as
# the read failures above, so they are display copy like the rest.
files-image-size-limit = Image exceeds preview size limit
files-image-svg-size-limit = Prepared SVG exceeds preview size limit
files-generated-image-unsupported = Unsupported generated image
files-generated-image-mismatch = Generated image format does not match its metadata

# The chrome around rendered markdown and images — code fences, diagrams and
# the lightbox affordance on an inline image.
files-copy-code = Copy code
files-copied = Copied
files-show-diagram = Show diagram
files-show-source = Show source
files-enlarge-image = Enlarge image

# The menu on a link in rendered markdown. "Open in RCode" and the destination
# row belong to the reading surfaces, so they live with them.
files-link-open-rcode = Open in RCode
files-link-show-in-folder = Show in folder
files-link-open-external = Open in external browser
files-open-links-in-rcode = Open links in RCode
files-open-links-in-rcode-checked = Open links in RCode, checked
files-open-links-in-rcode-unchecked = Open links in RCode, unchecked
