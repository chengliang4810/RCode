files-title = Files
files-rule-mutation-changed = The workspace changed. Refresh before deleting or moving entries.
files-rule-operation-identity = This file operation has an invalid identity. Reopen the operation and try again.
files-rule-delete-recursive = Confirm deletion of the folder and all its contents.
files-rule-mutation-cancelled = The file operation was cancelled before it started.
files-rule-canonical-spelling = Use the canonical workspace-relative spelling of the path.
files-rule-mutation-unsupported = Links and special files cannot be moved or deleted here.
files-rule-mutation-source-changed = The entry changed. Refresh before trying again.
files-rule-destination-spelling = The destination path is invalid.
files-rule-move-into-self = An entry cannot be moved into itself or its descendants.
files-rule-path-empty = The path must not be empty.
files-rule-path-too-long = The path is too long.
files-rule-path-invalid-character = The path contains an invalid character.
files-rule-path-relative = Use a path relative to the workspace.
files-rule-path-traversal = The path must not contain “.” or “..”.
files-rule-path-components = The path has too many components.
files-rule-path-utf8 = The path must use UTF-8.
files-rule-git-path = .git paths cannot be accessed.
files-rule-image-hash = Reload the image before requesting another chunk.
files-rule-image-offset = The image chunk offset is invalid.
files-rule-file-component = The file path contains an invalid component.
files-rule-directory-component = The directory path contains an invalid component.
files-rule-relative-path = The relative path is invalid.
files-rule-directory-cursor = The directory page cursor is invalid. Refresh the tree.
files-rule-directory-cursor-mismatch = The directory page no longer matches this request. Refresh the tree.
files-rule-directory-cursor-range = The directory page is out of range. Refresh the tree.
files-rule-query-empty = Enter a search query.
files-rule-unsupported-image = This workspace image format is not supported.
files-rule-image-too-large = The image exceeds the 8 MiB preview limit.
files-rule-symlink = Symbolic link targets are read-only.
files-rule-symlink-traversal = The file path passes through a symbolic link.
files-rule-symlink-directory = Symbolic link directories cannot be traversed.
files-rule-not-file = This path is not a regular file.
files-rule-parent-not-directory = The file’s parent path is not a directory.
files-rule-outside-workspace = The file path is outside this workspace.
files-rule-write-nul = Text containing NUL bytes cannot be saved.
files-rule-write-line-endings = Use normalized LF line endings before saving.
files-rule-write-hash = The file revision is missing. Reopen the file before saving.
files-rule-binary-write = Binary files cannot be edited here.
files-rule-encoding-write = Files with this text encoding cannot be edited.
files-rule-mixed-write = Files with mixed line endings cannot be edited.
files-rule-no-parent = The file has no parent directory.
files-rule-verify-write = The saved file could not be verified. Your editor buffer was preserved.
files-rule-file-changed-preparing = The file changed before saving. Review its current contents and try again.
files-rule-image-changed-limit = The image changed while loading or exceeds the preview limit.
files-rule-directory-cancelled = Directory loading was cancelled.
files-rule-directory-outside = The directory path is outside this workspace.
files-rule-entry-outside = The entry is outside this workspace.
files-rule-directory-not-directory = The directory path no longer points to a directory.
files-query-too-long = The search query must not exceed { $maximum } characters.
files-write-too-large = The file exceeds the { $maximum }-byte editing limit.
files-error-detail = { $summary }: { $reason }
files-connection-closed = The workspace connection closed. Reconnect and try again.
files-directory-changed = Workspace changed between directory pages. Refresh and try again.
files-directory-cursor-repeated = The workspace returned a repeated directory page. Refresh and try again.
files-image-changed = The image changed while loading. Open it again.
files-image-chunk-invalid = The workspace returned an invalid image chunk.
files-image-chunk-limit = The image exceeded the chunk limit.
files-request-object-required = The workspace request must be an object.
files-memory-unavailable = Automatic memory is unavailable.
files-memory-directory-unavailable = The memory directory is unavailable: { $reason }
files-invalid-target = Select exactly one chat or project workspace.
files-checkout-path-requires-space = A checkout path requires a project workspace.
files-chat-missing = This chat no longer exists.
files-space-missing = This project workspace no longer exists.
files-different-device = This workspace belongs to another device.
files-folder-absolute-required = The workspace folder must use an absolute path.
files-folder-unavailable = The workspace folder is unavailable: { $reason }
files-folder-not-directory = The workspace path is not a directory.
files-folder-missing = This chat has no workspace folder.
files-not-checkout = This folder is not a checkout of the selected workspace.
files-service-stopping = The workspace file service is shutting down.
files-search-cancelled = Workspace search was cancelled.
files-read-cancelled = File reading was cancelled.
files-write-cancelled = File saving was cancelled.
files-worker-failed = The workspace operation stopped: { $reason }
files-native-failed = The workspace operation failed: { $reason }
files-recovery-required = Rename failed ({ $reason }) and the original name could not be restored ({ $rollback }). Recover the entry at { $path }.
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

# Multi-entry operations and their recovery states.
files-selection-count = { $count ->
    [one] { $count } item
   *[other] { $count } items
}
files-delete-subject-two = 2 items ({ $first } and { $second })
files-delete-subject-many = { $count } items ({ $first }, { $second } and { $remaining } more)
files-operation-running = Another file operation is still running.
files-operation-wait-save = Wait for the pending save to finish and try again.
files-operation-workspace-changed = Workspace changed before the operation started. Refresh and try again.
files-operation-changed-saving = Workspace changed while saving. Refresh and try again.
files-operation-session-changed = File operation cancelled after switching sessions.
files-operation-refresh = Refresh the tree before trying again.
files-batch-move-failed = Moved { $completed } of { $total } — { $name }: { $reason }
files-batch-delete-failed = Deleted { $completed } of { $total } — { $name }: { $reason }
files-watch-decode-failed = File updates could not be decoded: { $reason }
files-watch-interrupted = File updates interrupted — retrying
files-request-encode-failed = The workspace request could not be prepared: { $reason }
files-response-invalid = The workspace returned an invalid response: { $reason }
files-connection-unavailable = The workspace connection is unavailable: { $reason }
files-request-failed = The workspace operation failed: { $reason }
files-host-unsupported = Update the workspace host to use this operation: { $reason }
files-mutation-invalid-path = This path cannot be changed.
files-mutation-workspace-changed = Workspace changed. Refresh before trying again.
files-mutation-source-missing = The source no longer exists. Refresh the tree.
files-mutation-source-changed = The entry changed. Refresh and confirm its current contents.
files-mutation-destination-exists = An entry already exists at the destination.
files-mutation-invalid-destination = The destination is unavailable or invalid.
files-mutation-unsupported = This file operation is not supported for the selected entry.
files-mutation-busy = The file operation was cancelled or another operation is running. Try again.
files-mutation-partial-failure = The operation did not finish. Refresh to see which entries remain.
