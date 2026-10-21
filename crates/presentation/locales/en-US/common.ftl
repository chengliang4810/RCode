# Words with no domain of their own: actions a person takes anywhere in the app.
# Keys are lowercase-hyphenated and stable; they are not abbreviations of the
# English text, so a reworded message never needs a new key.

action-cancel = Cancel
action-back = Back
action-confirm = Confirm
action-retry = Try again
action-dismiss = Dismiss
action-copy = Copy
action-copy-message = Copy message
action-delete = Delete
action-save = Save
action-loading = Loading…
action-reset = Reset
action-zoom-in = Zoom in
action-zoom-out = Zoom out

# Counts. Fluent selects the plural form, so the caller passes the number and
# the resource decides the wording — an English "1 session / 2 sessions" and a
# Chinese "1 个会话 / 2 个会话" are both expressed here rather than in code.
count-sessions = { $count ->
    [one] { $count } session
   *[other] { $count } sessions
}
count-projects = { $count ->
    [one] { $count } project
   *[other] { $count } projects
}

# Relative times, for the rows that stamp "when". English is the full phrase
# with Fluent choosing the singular — "1 minute ago", "2 minutes ago" — rather
# than a compact "2m"; Chinese uses a complete quantity expression. A future
# timestamp is clamped to the past by the caller before it reaches these, so
# there is deliberately no "in N minutes" form to translate.
time-just-now = just now
time-minutes-ago = { $count ->
    [one] { $count } minute ago
   *[other] { $count } minutes ago
}
time-hours-ago = { $count ->
    [one] { $count } hour ago
   *[other] { $count } hours ago
}
time-days-ago = { $count ->
    [one] { $count } day ago
   *[other] { $count } days ago
}
time-weeks-ago = { $count ->
    [one] { $count } week ago
   *[other] { $count } weeks ago
}
time-months-ago = { $count ->
    [one] { $count } month ago
   *[other] { $count } months ago
}
time-years-ago = { $count ->
    [one] { $count } year ago
   *[other] { $count } years ago
}
compact-time-just-now = now
compact-time-minutes-ago = { $count ->
    [one] { $count }m
   *[other] { $count }m
}
compact-time-hours-ago = { $count ->
    [one] { $count }h
   *[other] { $count }h
}
compact-time-days-ago = { $count ->
    [one] { $count }d
   *[other] { $count }d
}
compact-time-weeks-ago = { $count ->
    [one] { $count }w
   *[other] { $count }w
}
compact-time-months-ago = { $count ->
    [one] { $count }mo
   *[other] { $count }mo
}
compact-time-years-ago = { $count ->
    [one] { $count }y
   *[other] { $count }y
}

action-rename = Rename
action-delete-ellipsis = Delete…
action-forward = Forward

action-install = Install
