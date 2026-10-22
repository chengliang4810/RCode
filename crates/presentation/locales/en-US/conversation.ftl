# The transcript and the composer: what a run produced, and how it is sent.

conversation-empty-title = Ask anything
conversation-empty-body = Describe what you want done.
conversation-stop = Stop
conversation-send = Send
conversation-queued = Queued
conversation-thinking = Thinking
conversation-working = Working
conversation-interrupted = Interrupted
conversation-compacted = The earlier conversation was compacted to free context.
conversation-attached-images = { $count ->
    [one] { $count } image attached
   *[other] { $count } images attached
}

# Tool cards: the display NAME of a tool and the wording of the ToolGroup
# summary line. A tool's argument content — a path, a command, a URL, a search
# pattern — is never translated and never appears here; only these labels and
# the connective words. An unidentified tool still gets a card (tool-unknown),
# so a driver's brand-new tool is never silently dropped from the transcript.
tool-run = Run
tool-read = Read
tool-write = Write
tool-edit = Edit
tool-patch = Patch
tool-search = Search
tool-glob = Glob
tool-fetch = Fetch
tool-web = Web
tool-todo = Todo
tool-mcp = MCP
tool-agent = Agent
tool-unknown = Tool
# Shown when an ApplyPatch names no path: the whole workspace was patched.
tool-patch-workspace = workspace
tool-search-in = { $pattern } in { $path }
tool-todo-progress = { $done }/{ $total } done

# The ToolGroup summary line, one fragment per kind, joined by " · " and
# capitalized on the first fragment only. Fluent selects the singular so the
# line reads "ran 1 command", not "ran 1 commands".
tool-summary-ran = { $count ->
    [one] ran { $count } command
   *[other] ran { $count } commands
}
tool-summary-edited = { $count ->
    [one] edited { $count } file
   *[other] edited { $count } files
}
tool-summary-read = { $count ->
    [one] read { $count } file
   *[other] read { $count } files
}
tool-summary-searched = { $count ->
    [one] searched { $count } time
   *[other] searched { $count } times
}
tool-summary-fetched = { $count ->
    [one] fetched { $count } page
   *[other] fetched { $count } pages
}
tool-summary-todos = updated todos
tool-summary-called = { $count ->
    [one] called { $count } tool
   *[other] called { $count } tools
}
tool-summary-none = { $count ->
    [one] { $count } tool
   *[other] { $count } tools
}
tool-summary-failed = { $count } failed
# A thought chip and a note chip never reach the shared summary (they are
# UI-synthesized), so their fragments live here too. The singular is a NAME
# ("thought process"), not a count, which is why it is its own variant.
tool-summary-thoughts = { $count ->
    [one] thought process
   *[other] thought { $count } times
}
tool-summary-notes = { $count ->
    [one] wrote a note
   *[other] wrote { $count } notes
}
# The two chip labels in the group's compact fold, and the words for the two
# payloads a chip can offer to fetch in full.
conversation-thought-process = Thought process
conversation-wrote-a-note = Wrote
conversation-payload-diff = diff
conversation-payload-output = output
# The fetch affordance under an open chip. `what` is conversation-payload-diff
# or conversation-payload-output, already in the display language; `size` is a
# formatted byte count.
conversation-show-full = Show full { $what }
conversation-show-full-size = Show full { $what } ({ $size })
conversation-loading-full = Loading full { $what }…
conversation-load-full-failed = Couldn't load full { $what } — tap to retry


# The working trailer under a live turn, and the states a send passes through.
conversation-sending = Sending
conversation-queued-will-send = Queued — will send automatically
conversation-send-undelivered = Not delivered — click to retry
conversation-worked-for = Worked for { $duration }
# Elapsed time: a compact two-unit form, rounded down. The units are words a
# reader parses, so they are translated; the digits are not.
conversation-elapsed-seconds = { $seconds }s
conversation-elapsed-minutes = { $minutes }m { $seconds }s
conversation-elapsed-hours = { $hours }h { $minutes }m
conversation-elapsed-days = { $days }d { $hours }h

# Rotating status words beside the working spinner. Keep the same entries in
# every language; these describe general activity, not specific tool actions.
conversation-flavour-processing = Processing
conversation-flavour-thinking = Thinking
conversation-flavour-analyzing = Analyzing
conversation-flavour-pondering = Pondering
conversation-flavour-deliberating = Deliberating
conversation-flavour-organizing = Organizing
conversation-flavour-formulating = Formulating
conversation-flavour-reasoning = Reasoning
conversation-flavour-distilling = Distilling
conversation-flavour-untangling = Untangling

# Pieces of the transcript's own chrome: the media placeholders, the metadata
# strip, and the chips a resolved or pending interaction leaves behind. A name,
# a title or a device id inside one of these is data and stays verbatim.
conversation-loading-image = Loading image…
conversation-image-unavailable = Image unavailable
# The bare noun, for an attachment whose own name cannot be read.
conversation-image = Image
conversation-copy-path = Copy path
conversation-copy-image = Copy image
conversation-show-more = Show more
conversation-show-less = Show less
conversation-expand-message = Expand message
conversation-collapse-message = Collapse message
conversation-preview-generated-image = Preview generated image
conversation-loading-generated-image = Loading generated image…
conversation-generated-image-unavailable = Generated image unavailable
conversation-preview-name = Preview { $name }
conversation-remove-name = Remove { $name }
conversation-preview-appshot = Preview { $app } Appshot: { $title }
# The bare noun, for the places it rides beside a name or a title.
conversation-appshot = Appshot
conversation-uploading-appshot = Uploading Appshot…
conversation-loading-appshot = Loading Appshot…
conversation-appshot-unavailable = Appshot unavailable
conversation-uploading-percent = Uploading { $percent }%
conversation-uploading = Uploading…
conversation-forked-from = Forked from
conversation-question = Question
conversation-awaiting-answer = Awaiting your answer…
conversation-subagent = Subagent
# The attribution line on a message sent from another chat. The name is the
# sender's own title; the sentence around it is the translated part.
conversation-message-from = Message from { $name }

# The composer: its placeholder, its command palette, its completion popups and
# the failures it reports. A command name, a model id and a path stay verbatim.
conversation-do-anything = Do anything…
conversation-answer-placeholder = Type your own answer, or pick an option above
conversation-answer-placeholder-picked = Type your own answer, or leave this blank to use the selected option
conversation-select-multiple = Select one or more options.
conversation-wizard-back = Back
conversation-wizard-next = Next
conversation-wizard-submit = Submit
conversation-send-message = Send message
conversation-queue-message = Queue message
conversation-attach-images = Attach images
conversation-attach = Attach
conversation-commands-group = Commands
conversation-skills-group = Skills
conversation-command-model = RCode: choose agent, model, and reasoning
conversation-command-new = RCode: start a new conversation
conversation-command-resume = RCode: search and open conversations
conversation-command-settings = RCode: open settings
conversation-command-diff = RCode: open changes
conversation-command-files = RCode: open project files
conversation-command-terminal = RCode: open a terminal
conversation-command-rename = RCode: rename this conversation
conversation-command-stop = RCode: stop the active run
conversation-no-files-available = No files available
conversation-no-matching-files = No matching files
conversation-attached-image = Attached image
conversation-attached-file = Attached file
conversation-no-skills-project = No skills available for this project
conversation-agent-no-skills = This agent does not advertise skills
conversation-no-commands-or-skills = No commands or skills available
conversation-no-slash-commands = No slash commands available in this integration
conversation-no-matching-skills = No matching skills
conversation-no-matching-commands-or-skills = No matching commands or skills
conversation-no-matching-commands = No matching commands
conversation-search-older-device = The session's device runs an older RCode — update it to search its files
conversation-device-unreachable = The session's device is unreachable
conversation-file-search-failed = File search failed
conversation-skills-need-update = Skills require an updated engine on the selected device. Restart that device's RCode after updating.
conversation-commands-need-update = Commands require an updated engine on the selected device. Restart that device's RCode after updating.
conversation-couldnt-load-skills = Couldn't load this agent's skills
conversation-couldnt-load-commands = Couldn't load this agent's commands
conversation-command-discovery-needs-connection = Agent command discovery requires a connection
conversation-update-to-send-references = Update the selected device's RCode to send file, command, or skill references. Your draft is preserved.
conversation-engine-not-connected = Engine not connected
conversation-update-engine-to-queue = Update the chat's engine to queue messages during a response.
conversation-couldnt-stage-attachment = Couldn't stage the attachment locally.
conversation-couldnt-upload-attachment = Couldn't upload the attachment — the device may be offline.
conversation-couldnt-create-side-chat = Couldn't create the side chat.
conversation-remove-appshot-limit = Remove an Appshot before adding another (96 MB staged Appshot limit).
conversation-offline-will-send = Offline — messages will send when you're back online.
conversation-will-send-on-reconnect = Messages will send once the connection recovers.
conversation-send-failed = Send failed: { $error }
conversation-send-failed-no-queue-id = Send failed: the queue did not return an id
conversation-stop-failed = Stop failed: { $error }
conversation-answer-failed = Answer failed: { $error }
conversation-image-load-failed = Could not load the image. Try opening it again.

# The pending-message queue. A device id inside one of these is data.
conversation-queue-editing-on = Editing on { $device }
conversation-queue-needs-review = Needs review
conversation-queue-edit = Edit
conversation-queue-removing = Removing…
conversation-queue-remove = Remove
conversation-queue-save = Save to queue
conversation-queue-saving = Saving…
conversation-queue-editing-in-composer = Editing in composer
conversation-queue-steer = Steer
conversation-queue-send-next = Send next
conversation-queue-send-now = Send now
conversation-queue-steer-hint = Steer (keep current work running)
conversation-queue-send-next-hint = Send at the next turn (this agent cannot steer mid-turn)
conversation-queue-send-now-hint = Send now (interrupt)
conversation-queue-waiting-capabilities = Waiting for provider capabilities
conversation-more-attachments = { $count } more: { $names }
conversation-more-attachments-hint = { $names }; edit message to view all
conversation-queue-removal-unsupported = The chat host does not support safe queue removal
conversation-queue-already-left = That message had already left the queue
conversation-queue-remove-failed = Couldn't remove the message
conversation-queue-edit-unsupported = Update the chat host to edit queued messages safely
conversation-queue-load-failed = Couldn't load the queued attachments or Appshot context. Check the connection and update the chat host.
conversation-queue-invalid-lease = The chat host returned an invalid edit lease
conversation-queue-locked = That queued message is being edited on another device
conversation-queue-gone = That queued message is no longer available
conversation-queue-connect-to-edit = Connect to the chat host to edit this message
conversation-queue-lease-lost = The edit lease was lost; your text is still in the editor
conversation-queue-changed-elsewhere = This message changed on another device; your edit was kept locally
conversation-queue-removed-locally = The queued message was removed; your edit was kept locally
conversation-queue-lease-changed = The edit lease changed; your text is still in the editor
conversation-queue-host-unreachable = Couldn't reach the chat host; your edit is still in the editor
conversation-queue-protection-expired = Edit protection expired; review this message before sending
conversation-queue-actions-unsupported = The chat host does not support queue actions
conversation-queue-reorder-failed = Couldn't reorder the queue
conversation-queue-send-failed = Couldn't send that message
conversation-queue-edit-removed = The queued message was removed; your edit remains in the composer

# The checklist tray above the composer. Its own rows are the agent's text.
conversation-todo-expand = Expand todo list
conversation-todo-collapse = Collapse todo list
conversation-todo-all-done = All done
conversation-todo-dismiss = Dismiss todo list
conversation-todo-earlier = { $count } earlier
conversation-todo-later = { $count } later
conversation-todo-show-earlier = Show { $count } earlier items
conversation-todo-hide-earlier = Hide { $count } earlier items
conversation-todo-show-later = Show { $count } later items
conversation-todo-hide-later = Hide { $count } later items

# The context-occupancy popover. `used`, `total` and `remaining` arrive already
# grouped for reading; the words around them are what is translated.
conversation-context-window = Context capacity
conversation-context-tokens = { $used }/{ $total } ({ $percent }%)
conversation-context-used = { $used } tokens used
conversation-context-limit-unknown = Context limit not reported
conversation-context-capacity = { $total } token capacity
conversation-context-waiting = Waiting for context usage
conversation-context-not-reported = Context usage not reported by this harness yet

# Session billing and performance statistics.
conversation-stats-title = Session statistics
conversation-stats-counts = { $turns } turns { $steps } steps
conversation-stats-recorded-counts = Recorded { $turns } turns { $steps } steps
conversation-stats-recorded-usage = Recorded token usage
conversation-stats-model-time = Model time
conversation-stats-ttft = Avg first token (TTFT)
conversation-stats-speed = Output speed (TPS)
conversation-stats-seconds = { $seconds }s
conversation-stats-session-usage = Session token usage
conversation-stats-turn-usage = Turn usage
conversation-stats-turn-summary = Usage { $tokens }
conversation-stats-model = Provider / model
conversation-stats-input = Input
conversation-stats-uncached-input = Uncached input
conversation-stats-cache-read = Cache read
conversation-stats-cache-write = Cache write
conversation-stats-output = Output
conversation-stats-cache-hit = Cache hit
conversation-stats-cache-summary = Cache hit { $percent }%

conversation-goal-active = Active
conversation-goal-paused = Paused
conversation-goal-complete = Complete
conversation-goal-pause = Pause
conversation-goal-resume = Resume
conversation-goal-clear = Clear
conversation-goal-iteration = Iteration { $count }
conversation-goal-command = Set or manage the session goal
conversation-goal-input-hint = objective or pause / resume / clear
conversation-goal-active-title = Active goal
conversation-goal-paused-title = Paused goal
conversation-goal-complete-title = Completed goal

conversation-goal-edit = Edit goal
conversation-goal-save = Save
conversation-goal-reload = Reload
conversation-goal-missing = Goal removed
conversation-goal-invalid = Goal must contain 1 to 4000 characters
conversation-goal-duration-seconds = { $count }s
conversation-goal-duration-minutes = { $count }m
conversation-goal-duration-hours = { $count }h
conversation-goal-duration-days = { $count }d

conversation-load-earlier = Load earlier messages

# Mobile transcript chrome. Model/user content remains verbatim.
mobile-question-answered = Answered a question
mobile-question-awaiting = Awaiting your answer…
mobile-forked-from = Forked from { $title }
mobile-show-less = Show less
mobile-show-more = Show more
mobile-new-file = New file
mobile-diff-truncated = Diff truncated — showing first { $limit } of { $total } lines
mobile-more-lines = { $count ->
    [one] … { $count } more line
   *[other] … { $count } more lines
}
