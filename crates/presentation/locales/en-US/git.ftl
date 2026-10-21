git-changes = Changes
git-history = History
git-branch = Branch
git-worktree = Worktree
git-pull-request = Pull request
git-no-changes = No changes
git-diff-empty = Nothing to compare

# Changes pane: per-file notices the patch parser derives.
git-notice-mode-changed = Mode changed to { $mode }
git-notice-new-file = New file
git-notice-deleted-file = Deleted file
git-notice-renamed-from = Renamed from { $from }
git-notice-binary = Binary file — contents not shown
git-notice-truncated = Diff truncated — showing first { $max } of { $total } lines

# The header label above an uncommitted working-tree diff.
git-uncommitted-changes = { $count ->
    [one] { $count } uncommitted change
   *[other] { $count } uncommitted changes
}

# Scope names, as the surface title and the scope menu.
git-scope-working-tree = Working tree
git-scope-branch-changes = Branch changes
git-scope-latest-turn = Latest turn
git-scope-commit = Commit

# Scope header-strip summaries. The branch summary is joined to git-scope-versus
# in code, because the base ref is a second value a plural selector cannot share
# a message with.
git-scope-summary-branch = { $count ->
    [one] { $count } changed file
   *[other] { $count } changed files
}
git-scope-versus = vs { $base }
git-scope-summary-turn = { $count ->
    [one] { $count } changed file this turn
   *[other] { $count } changed files this turn
}
git-scope-summary-commit = { $count ->
    [one] { $count } changed file in this commit
   *[other] { $count } changed files in this commit
}

# Empty-state copy, one per scope.
git-clean-worktree = No uncommitted changes
git-clean-branch = No branch changes
git-clean-branch-vs = No changes vs { $base }
git-clean-turn = No changes this turn
git-clean-history = No commits found
git-clean-commit = Empty commit

# The watch stream failing or being unavailable.
git-stream-interrupted = Diff stream interrupted — retrying
git-watch-unavailable = Diff watch unavailable: { $reason }

# Pane-header controls.
git-refresh = Refresh
git-split-view = Split view
git-wrap-long-lines = Wrap long lines
git-discard-changes = Discard changes
git-expand-all-files = Expand all files
git-collapse-all-files = Collapse all files
git-open-in-file-browser = Open in file browser

# The branch base-ref picker.
git-search-branches-placeholder = Search branches…
git-no-branches = No branches
git-no-matching-branches = No matching branches

# Scoped-diff content-area states.
git-partial-snapshot = Partial snapshot
git-no-turn-yet = No turn recorded yet — send a message first
git-device-outdated = This chat's device is running an older RCode — update it to view branch and turn diffs
git-preparing-diff = Preparing diff…
git-comment-placeholder = Request a change…
git-comment-add-aria = Add comment
git-comment-edit-aria = Edit comment
git-comment-remove = Remove comment
git-comment-submit = Comment

# History pane: ref kinds (git-branch covers the branch kind) and ref labels.
git-ref-remote = Remote branch
git-ref-tag = Tag
git-ref-with-label = { $kind }: { $label }
git-fetch-all = Fetch all
git-fetching = Fetching…
git-show-all-commits = Show all commits
git-show-branch-tips = Show branch tips
git-search-commits = Search commits
git-close-search = Close search
git-search-placeholder = Search
git-commit-count = { $count ->
    [one] { $count } commit
   *[other] { $count } commits
}
git-comparison-ahead = { $count } ahead
git-comparison-behind = { $count } behind
git-comparison-tooltip = Compared with { $base }: { $ahead } ahead, { $behind } behind
git-load-more = Load more
git-loading = Loading…
git-retry = Retry
git-no-repository = No repository selected
git-loading-history = Loading history…
git-no-matching-commits = No matching commits
git-no-branch-tips = No branch tips found
git-fetch-failed = Fetch failed: { $error }
git-column-author = Author
git-column-date = Date
git-column-sha = SHA
git-column-commit = Commit
git-columns = Columns
git-column-name = Name
git-column-reset = Reset
git-branch-expand = Expand { $label }
git-branch-expand-hidden = Expand { $label } ({ $count } hidden)
git-branch-collapse = Collapse { $label }

# Settings → Archived sessions.
archive-unarchive = Unarchive
archive-unarchiving = Unarchiving…
archive-unarchive-failed = Unarchive failed: { $reason }
archive-untitled-session = Untitled session
archive-nothing = Nothing archived
archive-nothing-detail = Right-click a session in the sidebar to archive it.
archive-previous-aria = Previous archived sessions
archive-next-aria = Next archived sessions
archive-previous = Previous
archive-next = Next
archive-page-range = { $start }–{ $end } of { $total }
archive-title = Archived sessions
archive-subtitle = Hidden from the sidebar until restored.

# The pull-request badge (sidebar and composer).
pr-state-open = Open
pr-state-merged = Merged
pr-state-closed = Closed
pr-badge-label = PR #{ $number } · { $state }
pr-open-aria = Open pull request { $number }: { $title }

# Pull-request list (pull_requests.rs).
pr-list-group-needs-attention = Needs attention
pr-list-group-awaiting-review = Awaiting review
pr-list-group-approved = Approved
pr-list-group-drafts = Drafts
pr-list-star = Star pull request
pr-list-unstar = Unstar pull request
pr-list-search-placeholder = Search pull requests
pr-list-error-detect-repository = Couldn’t detect this project’s repository. Choose a repository to continue.
pr-list-error-enter-repository = Enter a repository as owner/name.
pr-list-error-connect-device = Connect to a device to open its project repository.
pr-list-error-device-offline = This device is offline. Reconnect it to open the project.
pr-list-error-unreadable-repository = The device returned an unreadable repository. Try again.
pr-list-error-no-remote = This project has no GitHub remote. Enter owner/repository to open it directly.
pr-list-error-read-repository = Couldn’t read this project’s repository. Check the device connection and try again.
pr-list-error-detect-timeout = Repository detection timed out. Try again or enter owner/repository.
pr-list-detecting-repository = Detecting repository…
pr-list-choose-repository = Choose repository
pr-list-this-device = This device
pr-list-repository-aria = Repository: { $label }
pr-list-projects = Projects
pr-list-finding-repository = Finding repository…
pr-list-open-project-aria = Open { $name } repository on { $device }: { $path }
pr-list-open-project-aria-offline = Open { $name } repository on { $device }: { $path }, offline
pr-list-recent-repositories = Recent repositories
pr-list-open-recent-aria = Open { $repo }
pr-list-open-aria = Open repository
pr-list-open = Open
pr-list-sort-updated-desc = Recently updated
pr-list-sort-updated-asc = Least recently updated
pr-list-sort-opened-desc = Newest first
pr-list-sort-opened-asc = Oldest first
pr-list-sort-changes-desc = Largest changes
pr-list-sort-changes-asc = Smallest changes
pr-list-sort-aria = Sort pull requests: { $label }
pr-list-loading-more = Loading more…
pr-list-load-more-count = Load { $count } more
pr-list-load-more = Load more
pr-list-desktop-device-aria = Desktop device: { $label }
pr-list-desktop-devices = Desktop devices
pr-list-device-offline-aria = { $name }, offline
pr-list-offline = Offline
pr-list-empty-choose-repository-title = Choose a repository
pr-list-empty-choose-repository-body = No GitHub repository was found for this project. Choose one to load your open pull requests.
pr-list-empty-idle-title = Ready when you are
pr-list-empty-idle-body = Refresh to load this repository on the selected device.
pr-list-empty-authored-title = No pull requests authored by you
pr-list-empty-authored-body = Your open pull requests in this repository will appear here.
pr-list-empty-reviewing-title = No reviews requested
pr-list-empty-reviewing-body = Pull requests waiting for your review will appear here.
pr-list-empty-all-title = No open pull requests
pr-list-empty-all-body = This repository has no open pull requests. New ones will appear here.
pr-list-show-all = Show all pull requests
pr-list-load-pull-requests = Load pull requests
pr-list-showing-last-results = Showing the last loaded results.
pr-list-loaded-just-now = Loaded just now · Refresh pull requests
pr-list-loaded-minutes-ago = Loaded { $count }m ago · Refresh pull requests
pr-list-refresh-pull-requests = Refresh pull requests
pr-list-title = Pull requests
pr-list-loading-pull-requests = Loading pull requests
pr-list-refresh-aria = Fetch the latest results from GitHub
pr-list-filter-aria = Pull requests to show
pr-list-filter-authored = Authored
pr-list-filter-reviewing = Reviewing
pr-list-filter-all = All
pr-list-clear-search = Clear search
pr-list-needs-you = Needs you
pr-list-starred = Starred
pr-list-shown-of-total = { $shown } of { $total } pull requests
pr-list-nothing-needs-attention = Nothing needs your attention
pr-list-no-starred = No starred pull requests
pr-list-no-matching = No matching pull requests
pr-list-count = { $count ->
    [one] { $count } pull request
   *[other] { $count } pull requests
}
pr-list-row-aria = { $title }: { $repo } #{ $number }. { $status }. Open pull request
pr-list-date-opened = Opened
pr-list-date-updated = Updated
pr-list-date-tooltip = { $label } { $relative } · { $date } at { $time } UTC
pr-list-opened-when = Opened { $when }
pr-list-status-draft = Draft
pr-list-status-open = Open
pr-list-status-approved = Approved
pr-list-unknown-author = Unknown author
pr-list-ci-commit-unavailable = unavailable
pr-list-ci-no-action = No action suggested
pr-list-ci-commit-line = { $state } · Commit: { $commit }
pr-list-ci-next-line = Next: { $action }
pr-list-diff-aria = { $additions } additions, { $deletions } deletions
pr-list-error-cli-title = GitHub CLI isn’t available on this device
pr-list-error-cli-body = Install gh and sign in to view your pull requests.
pr-list-error-auth-title = Sign in to GitHub on this device
pr-list-error-auth-body = Run gh auth login, then refresh this page.
pr-list-error-rate-title = GitHub’s rate limit was reached
pr-list-error-rate-body = Requests are paused for 15 minutes. Your loaded results stay available.
pr-list-error-network-title = Couldn’t load pull requests
pr-list-error-network-body = Check the repository name and your connection, then try again.
pr-list-error-offline-title = { $name } is offline
pr-list-error-offline-body = Reconnect the device and try again.
pr-list-error-update-title = Update RCode on { $name }
pr-list-error-update-body = This device doesn’t support pull requests yet.
pr-list-compact-now = now
pr-list-compact-minutes = { $count }m ago
pr-list-compact-hours = { $count }h ago
pr-list-compact-days = { $count }d ago
pr-list-compact-weeks = { $count }w ago
pr-list-compact-months = { $count }mo ago

# Pull-request detail (pull_request_detail.rs).
pr-detail-title-aria = Pull request { $number }: { $title }
pr-detail-refresh = Refresh pull request
pr-detail-copy-link = Copy link
pr-detail-link-copied = Link copied
pr-detail-open-browser = Open in default browser
pr-detail-tab-summary = Summary
pr-detail-tab-code = Code
pr-detail-tab-activity = Activity
pr-detail-sections = Pull request sections
pr-detail-search-placeholder = Find a changed file…
pr-detail-comment-placeholder = Write a comment… Type @ to mention someone
pr-detail-loaded = Loaded { $age }
pr-detail-field-status = Status
pr-detail-field-review = Review
pr-detail-no-review-decision = No review decision
pr-detail-status-draft = Draft
pr-detail-status-open = Open
pr-detail-status-merged = Merged
pr-detail-status-closed = Closed
pr-detail-status-approved = Approved
pr-detail-status-passed = Passed
pr-detail-status-changes-requested = Changes requested
pr-detail-status-awaiting-review = Awaiting review
pr-detail-status-not-reported = Not reported
pr-detail-file-count = { $count ->
    [one] { $count } file
   *[other] { $count } files
}
pr-detail-checks-label = Checks
pr-detail-checks-aria = Checks, { $count }, { $status }
pr-detail-open-check = Open { $name }
pr-detail-current-assessment = Current assessment
pr-detail-no-blockers = No blockers reported
pr-detail-assessment-incomplete = Assessment incomplete
pr-detail-next = Next: { $action }
pr-detail-commit-tooltip-empty = Current commit unavailable
pr-detail-commit-unavailable = Commit unavailable
pr-detail-commit = Commit { $sha }
pr-detail-no-description = No description provided.
pr-detail-retry-diff = Retry diff
pr-detail-deleted-account = Deleted account
pr-detail-resolved = Resolved
pr-detail-unresolved = Unresolved
pr-detail-outdated = Outdated
pr-detail-reply = Reply
pr-detail-open-comment = Open review comment on GitHub
pr-detail-activity-unavailable = Inline review threads are unavailable. Refresh or open this pull request on GitHub to read them.
pr-detail-no-activity = No activity yet
pr-detail-no-conversation = No conversation yet
pr-detail-activity-empty-body = Comments and reviews will appear here. Start the conversation below.
pr-detail-image-load-failed = Couldn’t load image. Click to open it in your browser · Escape to close
pr-detail-image-loading = Loading image… · Escape to cancel
pr-detail-ci-unavailable = CI unavailable
pr-detail-ci-no-checks = No checks reported
pr-detail-ci-empty-metadata = CI metadata is unavailable. Refresh to try again.
pr-detail-ci-empty-no-checks = No checks reported.
pr-detail-ci-empty-open = Check details are unavailable. Open this pull request on GitHub.
pr-detail-error-gh-cli-missing = Install GitHub CLI (gh) on this device and sign in.
pr-detail-error-auth = Run gh auth login on this device, then refresh.
pr-detail-error-rate-limited = GitHub’s rate limit was reached. Try again in 15 minutes.
pr-detail-error-timeout = GitHub didn’t respond in time. Try again.
pr-detail-error-decode = GitHub’s response was too large or unreadable. Open it on GitHub instead.
pr-detail-error-update-device = Update RCode on the selected device.
pr-detail-error-generic = Check the link and your connection, then try again.
pr-detail-error-load-failed = Couldn’t load this pull request. { $reason }
pr-detail-error-unreadable-detail = The device sent a pull request this version can’t read. Update RCode on the selected device.
pr-detail-error-connect-detail = Connect to your device to load this pull request.
pr-detail-error-load-diff-failed = Couldn’t load the diff. { $reason }
pr-detail-error-invalid-diff = The device returned an invalid diff.
pr-detail-error-connect-diff = Connect to your device to load the diff.

# Pull-request detail skeleton placeholders (pull_request_skeleton.rs).
pr-skel-status = Status
pr-skel-review = Review
pr-skel-current-assessment = Current assessment
pr-skel-checks = Checks
pr-skel-handoff-title = Hand off to an agent
pr-skel-handoff-body = Opens a new session with the prompt ready to send.
pr-skel-verification = Verification
pr-skel-checkout = Checkout
pr-skel-loading-list = Loading pull requests
pr-skel-loading-detail = Loading pull request
pr-skel-loading-diff = Loading diff

# Pull-request media (pull_request_media.rs).
pr-media-image-failed = { $alt } · Couldn’t load image. Open in browser ↗
pr-media-image-loading = Loading image…
pr-media-image-unavailable = Image unavailable here. Open the pull request in your browser to see it.
pr-media-open-image-aria = Open image: { $alt }
pr-media-load-image-aria = Load image from { $host }: { $alt }
pr-media-hosted = { $alt } · hosted on { $host } · Click to load

# Pull-request code pane (pull_request_code.rs).
pr-code-choose-file = Choose a changed file
pr-code-collapse-all = Collapse all files
pr-code-copy-diff = Copy diff
pr-code-copy-path = Copy path
pr-code-expand-all = Expand all files
pr-code-file-collapsed = { $path }, collapsed
pr-code-file-expanded = { $path }, expanded
pr-code-file-position = File { $position } of { $total }
pr-code-files-title = Files
pr-code-loading = Loading changes…
pr-code-next-file = Next file
pr-code-no-changed-files = No changed files
pr-code-no-matching-files = No matching files
pr-code-open-diff-aria = Open diff for { $path }
pr-code-previous-file = Previous file
pr-code-show-split = Show split diff
pr-code-show-unified = Show unified diff
pr-code-tree-label = Changed files

# Pull-request comment interactions (pull_request_interactions.rs).
pr-interact-comment-too-long = Comment is too long (maximum 60,000 bytes).
pr-interact-connect-to-send = Connect to the pull request’s device to send your comment.
pr-interact-no-mentions = No matching participants. You can type any @username.
pr-interact-post-unconfirmed = Couldn’t confirm your comment was posted. { $reason } Your draft is kept; check GitHub before sending again.
pr-interact-post-unread = Your comment was posted, but its reply couldn’t be read. Refresh to see it.
pr-interact-posting = Posting comment
pr-interact-pull-request-image = Pull request image
pr-interact-send = Send comment
pr-interact-sending = Sending comment

# Pull-request hand-off chrome (pull_request_handoff.rs). The prompt sent to
# the agent is not a resource — it is model input, not UI copy.
pr-handoff-action-address-feedback = Address feedback
pr-handoff-action-fix-checks = Fix failing checks
pr-handoff-action-resolve-conflicts = Resolve conflicts
pr-handoff-action-review = Review
pr-handoff-add-project = Add project
pr-handoff-add-project-aria = Add project on this device
pr-handoff-check-projects-failed = Couldn’t check this device’s projects. Check its connection and try again.
pr-handoff-check-timeout = Checking projects timed out. Check the device connection and try again.
pr-handoff-checkout = Checkout
pr-handoff-connect = Connect to the pull request’s device to start a session.
pr-handoff-copy-checkout = Copy checkout command
pr-handoff-finding-checkout = Finding a checkout…
pr-handoff-no-checkout = No checkout of { $repo } is available on this device. Add its project, then try again.
pr-handoff-platform-linux = Linux
pr-handoff-platform-macos = macOS
pr-handoff-platform-relevant = Relevant platforms
pr-handoff-platform-windows = Windows
pr-handoff-start-session-aria = { $action } in a new session
pr-handoff-subtitle = Opens a new session with the prompt ready to send.
pr-handoff-summary-checks = Reproduce and fix the failing checks
pr-handoff-summary-conflicts = Resolve merge conflicts and verify the combined changes
pr-handoff-summary-feedback = Make the changes reviewers asked for
pr-handoff-summary-review = Find bugs, regressions, and missing tests
pr-handoff-title = Hand off to an agent
pr-handoff-verification = Verification
