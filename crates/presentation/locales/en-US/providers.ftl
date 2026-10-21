# Provider and model configuration.

providers-title = Model settings
providers-add = Add provider
providers-empty-title = No providers configured
providers-empty-body = Add one to choose a model.
providers-api-key = API key
providers-base-url = Endpoint
providers-model-add = Add model
providers-models-fetch = Fetch models
providers-models-loading = Fetching models…
providers-models-unavailable = The engine is not connected.
providers-models-empty = The provider returned no models.
providers-models-select-all = Select all
providers-models-owner = Owner
providers-models-import = Add selected models
providers-models-added = Added
providers-model-remove = Remove model

# Editing one provider. Names and identifiers the user typed are data and are
# not translated; the labels around them are.
providers-label = Name
providers-protocol = Protocol
providers-protocol-openai-chat = OpenAI Chat Completions
providers-protocol-openai-responses = OpenAI Responses
providers-protocol-anthropic = Anthropic Messages
providers-credential = API key
providers-credential-none = No key configured
providers-credential-stored = A key is stored
providers-model-name = Vendor model name
providers-model-context = Context window
providers-model-max-tokens = Max output tokens
providers-model-count = { $count ->
    [one] { $count } model
   *[other] { $count } models
}
providers-save = Save
providers-saving = Saving…
providers-delete = Delete provider
providers-delete-confirm = Delete { $name }?
providers-conflict = The configuration changed since this was opened.
providers-conflict-reload = Reload and lose your changes
providers-unsaved = You have unsaved changes.

# The two-pane provider editor: a list of providers on the left, the selected
# one's form on the right. Model ids, provider names and endpoints are the
# user's data and are not translated; only the labels around them are.
providers-subtitle = Manage your model providers. Configured models are
    available in the model picker.
providers-add-short = Add provider
providers-list-title = Providers
providers-new = New provider
providers-new-numbered = New provider { $number }
providers-base-url-label = Base URL
providers-api-format = API format
providers-protocol-chat = Chat Completions (/chat/completions)
providers-protocol-responses = Responses (/responses)
providers-protocol-messages = Anthropic Messages (/v1/messages)
providers-model-list = Models
providers-model-empty = No models configured yet. Add one to use it in a chat.
providers-model-count-badge = { $count ->
    [one] { $count } model
   *[other] { $count } models
}
providers-refresh = Refresh
providers-reveal = Show the key
providers-conceal = Hide the key
providers-model-edit = Edit model
providers-model-done = Done
providers-model-dialog-add-title = Add model
providers-model-dialog-edit-title = Edit model
providers-model-name-hint = e.g. claude-sonnet-5-5
providers-model-context-hint = e.g. 200000
providers-model-max-tokens-hint = e.g. 32000
providers-model-name-required = A model needs a vendor model name.
providers-model-count-invalid = Token counts must be positive integers or blank.
providers-model-output-exceeds-context = Maximum output tokens cannot exceed the context window.
providers-model-input = Input
providers-model-input-text = Text
providers-model-input-text-and-images = Text and images
providers-model-input-vision-badge = Vision
providers-model-reasoning = Reasoning levels
providers-model-reasoning-add = Add a level
providers-model-reasoning-remove = Remove { $level }
providers-model-reasoning-hint = The levels this model accepts, weakest first.
    Empty means the model gets no effort chip in a chat.
providers-model-delete = Delete model
providers-model-delete-confirm = Delete { $name }?
providers-required = Required
providers-optional = Optional

providers-model-reorder = Drag to reorder models
