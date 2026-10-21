//! Slash commands: `/name` the user types, resolved before the model sees it.
//!
//! A command is a named action with its own arguments, distinct from a prompt.
//! `/compact` does not ask the model to compact — it compacts. The distinction
//! matters because some things the user wants are not requests to the model at
//! all, and expressing them as prompts makes the outcome depend on the model
//! agreeing.
//!
//! ```text
//!   user input ──parse──▶ CommandInvocation ──registry lookup──▶ CommandRoute
//!                                                                    │
//!                          ┌─────────────────────────────────────────┤
//!                          ▼                     ▼                   ▼
//!                    Done (handled)      Inject (goes to model)  NotACommand
//! ```
//!
//! # Naming
//!
//! A command name is `name`, `domain:name`, or `domain:namespace:name` — at
//! most two colons, split on the rightmost one. Domains exist so a command's
//! origin is visible in its name (`plugin:acme:deploy`), which is what keeps a
//! project's command from silently shadowing a built-in of the same word.
//!
//! The lexer and argument parser are kept intact and the machinery is dropped. What is
//! kept is the part with rules worth preserving — the rightmost-colon split,
//! the lowercase key, the refusal of `__` and empty segments. What is dropped
//! is the ACP-era baggage: the `ui` domain (commands that reached back into a
//! TUI panel), the `DependencyBag` (`TypeId`-keyed service locator), and the
//! provenance validation from the former dispatcher. File-backed user, project,
//! and plugin prompt commands are discovered by `custom_commands`; this module
//! retains the built-in action registry and its argument grammar.

use std::collections::HashMap;

/// A parsed command name.
///
/// The variants carry the split rather than a re-parsed string: `domain` is the
/// origin, `namespace` the source within it, `name` the command itself. Keeping
/// them separate is what lets the registry index by full name while the UI
/// still groups by origin.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CommandName {
    /// No colon. Resolved against the built-in domain only — a bare name is a
    /// shortcut, not a unique key.
    Bare { name: String },
    /// `domain:name`, e.g. `core:compact`.
    Qualified { domain: String, name: String },
    /// `domain:namespace:name`, e.g. `plugin:acme:deploy`.
    Namespaced {
        domain: String,
        namespace: String,
        name: String,
    },
}

/// Why a command name did not parse.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CommandNameError {
    #[error("a command name cannot be empty")]
    Empty,
    /// More than two colons. The cap keeps a name's origin readable: past
    /// three segments the domains stop meaning anything.
    #[error("a command name has at most two colons (three segments)")]
    TooManySegments,
    #[error("a command name cannot have an empty segment")]
    EmptySegment,
    /// `__` is the deprecated separator from before the colon syntax. Refused
    /// rather than accepted so a stale name fails loudly instead of resolving
    /// to something unexpected.
    #[error("`__` is no longer a command separator; use `:`")]
    DoubleUnderscore,
    #[error("a command name cannot contain {0:?}")]
    IllegalCharacter(char),
}

impl CommandName {
    /// Parse `input`, which must not include the leading `/` or any arguments.
    ///
    /// Segments are lowercased, because the registry indexes by lowercase and
    /// two names differing only in case would be two entries the user cannot
    /// tell apart.
    pub fn parse(input: &str) -> Result<Self, CommandNameError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(CommandNameError::Empty);
        }
        if trimmed.contains("__") {
            return Err(CommandNameError::DoubleUnderscore);
        }

        let mut segments: Vec<&str> = trimmed.split(':').collect();
        if segments.len() > 3 {
            return Err(CommandNameError::TooManySegments);
        }
        if segments.iter().any(|segment| segment.is_empty()) {
            return Err(CommandNameError::EmptySegment);
        }
        for segment in &segments {
            if let Some(bad) = segment
                .chars()
                .find(|character| character.is_whitespace() || *character == '/')
            {
                return Err(CommandNameError::IllegalCharacter(bad));
            }
        }

        let name = segments.pop().expect("at least one segment").to_lowercase();
        Ok(match segments.len() {
            0 => Self::Bare { name },
            1 => Self::Qualified {
                domain: segments[0].to_lowercase(),
                name,
            },
            _ => Self::Namespaced {
                domain: segments[0].to_lowercase(),
                namespace: segments[1].to_lowercase(),
                name,
            },
        })
    }

    /// The domain, when the name has one.
    pub fn domain(&self) -> Option<&str> {
        match self {
            Self::Bare { .. } => None,
            Self::Qualified { domain, .. } | Self::Namespaced { domain, .. } => Some(domain),
        }
    }

    /// The command's own segment, without any domain.
    pub fn name(&self) -> &str {
        match self {
            Self::Bare { name } | Self::Qualified { name, .. } | Self::Namespaced { name, .. } => {
                name
            }
        }
    }

    /// The full lowercase form — the registry's key.
    pub fn full(&self) -> String {
        match self {
            Self::Bare { name } => name.clone(),
            Self::Qualified { domain, name } => format!("{domain}:{name}"),
            Self::Namespaced {
                domain,
                namespace,
                name,
            } => format!("{domain}:{namespace}:{name}"),
        }
    }

    /// Whether this name is a bare shortcut.
    pub fn is_bare(&self) -> bool {
        matches!(self, Self::Bare { .. })
    }
}

/// What kind of value an argument takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgKind {
    /// Free text.
    Text,
    /// A whole number.
    Integer,
    /// One of a fixed set. An empty set means the handler supplies candidates
    /// at runtime, so nothing is checked here.
    Choice(Vec<String>),
    /// A filesystem path that must already exist.
    Path,
}

/// A positional or `--named` argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgSpec {
    pub name: String,
    pub kind: ArgKind,
    pub required: bool,
    pub description: Option<String>,
}

impl ArgSpec {
    pub fn required(name: &str, kind: ArgKind) -> Self {
        Self {
            name: name.to_owned(),
            kind,
            required: true,
            description: None,
        }
    }

    pub fn optional(name: &str, kind: ArgKind) -> Self {
        Self {
            name: name.to_owned(),
            kind,
            required: false,
            description: None,
        }
    }

    pub fn described(mut self, description: &str) -> Self {
        self.description = Some(description.to_owned());
        self
    }

    /// Check one value against this argument's kind.
    fn validate(&self, value: &str) -> Result<(), String> {
        match &self.kind {
            ArgKind::Text => Ok(()),
            ArgKind::Integer => value
                .parse::<i64>()
                .map(|_| ())
                .map_err(|_| format!("`{}` needs a whole number, got `{value}`", self.name)),
            ArgKind::Choice(candidates) => {
                if candidates.is_empty() || candidates.iter().any(|candidate| candidate == value) {
                    Ok(())
                } else {
                    Err(format!(
                        "`{}` must be one of {}, got `{value}`",
                        self.name,
                        candidates.join(", ")
                    ))
                }
            }
            ArgKind::Path => {
                if std::path::Path::new(value).exists() {
                    Ok(())
                } else {
                    Err(format!(
                        "`{}` names a path that does not exist: `{value}`",
                        self.name
                    ))
                }
            }
        }
    }
}

/// A `--flag` that takes no value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagSpec {
    /// The long form, without dashes.
    pub name: String,
    /// The short form including its dash, e.g. `-f`.
    pub short: Option<String>,
    pub description: Option<String>,
}

impl FlagSpec {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            short: None,
            description: None,
        }
    }

    pub fn with_short(mut self, short: &str) -> Self {
        self.short = Some(short.to_owned());
        self
    }

    pub fn described(mut self, description: &str) -> Self {
        self.description = Some(description.to_owned());
        self
    }
}

/// What arguments a command accepts.
///
/// All three lists default to empty, which means "accept anything as free
/// text" — the right default for a command that forwards its arguments
/// verbatim.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArgsSchema {
    pub positionals: Vec<ArgSpec>,
    pub named: Vec<ArgSpec>,
    pub flags: Vec<FlagSpec>,
}

/// Arguments parsed against an [`ArgsSchema`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedArgs {
    /// Positional values, in declaration order.
    pub positionals: Vec<String>,
    /// `--name value` pairs, keyed by the declared name.
    pub named: Vec<(String, String)>,
    /// Flags that were present, by declared name.
    pub flags: Vec<String>,
}

impl ParsedArgs {
    /// The nth positional, if it was given.
    pub fn positional(&self, index: usize) -> Option<&str> {
        self.positionals.get(index).map(String::as_str)
    }

    /// A named argument's value, if it was given.
    pub fn named_value(&self, name: &str) -> Option<&str> {
        self.named
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Whether a flag was present.
    pub fn has_flag(&self, name: &str) -> bool {
        self.flags.iter().any(|flag| flag == name)
    }

    /// Every positional joined by a space — for a command that forwards text.
    pub fn joined(&self) -> String {
        self.positionals.join(" ")
    }
}

impl ArgsSchema {
    /// Whether this schema declares nothing, meaning free-form input.
    fn is_free_form(&self) -> bool {
        self.positionals.is_empty() && self.named.is_empty() && self.flags.is_empty()
    }

    /// Parse `args` against this schema.
    ///
    /// Tokens are split on whitespace, so a value containing a space must be
    /// quoted by the caller's own convention — there is no shell here, and
    /// inventing one would make `--path "a b"` mean something different from
    /// every other place the user types a path.
    ///
    /// A free-form schema accepts everything: that is what makes `/bg do the
    /// thing` work without declaring a schema for every possible sentence.
    pub fn parse(&self, args: &str) -> Result<ParsedArgs, String> {
        let tokens: Vec<&str> = args.split_whitespace().collect();
        if self.is_free_form() {
            return Ok(ParsedArgs {
                positionals: tokens.into_iter().map(str::to_owned).collect(),
                ..ParsedArgs::default()
            });
        }

        let mut parsed = ParsedArgs::default();
        let mut index = 0;
        while index < tokens.len() {
            let token = tokens[index];
            if let Some(long) = token.strip_prefix("--") {
                if let Some(spec) = self.named.iter().find(|spec| spec.name == long) {
                    let value = tokens
                        .get(index + 1)
                        .ok_or_else(|| format!("`--{long}` needs a value"))?;
                    spec.validate(value)?;
                    parsed.named.push((spec.name.clone(), (*value).to_owned()));
                    index += 2;
                } else if let Some(flag) = self.flags.iter().find(|flag| flag.name == long) {
                    parsed.flags.push(flag.name.clone());
                    index += 1;
                } else {
                    return Err(format!("unknown option `--{long}`"));
                }
            } else if token.starts_with('-') && token.len() > 1 {
                // The short form is stored with its dash, so it matches as
                // written rather than needing the dash re-added here.
                if let Some(flag) = self
                    .flags
                    .iter()
                    .find(|flag| flag.short.as_deref() == Some(token))
                {
                    parsed.flags.push(flag.name.clone());
                    index += 1;
                } else {
                    return Err(format!("unknown option `{token}`"));
                }
            } else if parsed.positionals.len() < self.positionals.len() {
                let spec = &self.positionals[parsed.positionals.len()];
                spec.validate(token)?;
                parsed.positionals.push(token.to_owned());
                index += 1;
            } else {
                return Err(format!("unexpected argument `{token}`"));
            }
        }

        for (position, spec) in self.positionals.iter().enumerate() {
            if spec.required && parsed.positionals.len() <= position {
                return Err(format!("`{}` is required", spec.name));
            }
        }
        for spec in &self.named {
            if spec.required && parsed.named_value(&spec.name).is_none() {
                return Err(format!("`--{}` is required", spec.name));
            }
        }
        Ok(parsed)
    }
}

/// Where a command's output goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FeedbackChannel {
    /// Shown to the user only. The default, because most command output is a
    /// status line the model has no use for, and putting it in the transcript
    /// spends context on it every turn afterwards.
    #[default]
    UiOnly,
    /// Added to the conversation. For output the model genuinely needs — the
    /// result of `/compact`, say.
    Session,
}

/// What running a command produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandFeedback {
    pub message: String,
    pub channel: FeedbackChannel,
    /// Whether the command failed. The message is the reason.
    pub is_error: bool,
}

impl CommandFeedback {
    /// A message for the user's eyes only.
    pub fn ui(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            channel: FeedbackChannel::UiOnly,
            is_error: false,
        }
    }

    /// A message that joins the conversation.
    pub fn session(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            channel: FeedbackChannel::Session,
            is_error: false,
        }
    }

    /// A failure, shown to the user.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            channel: FeedbackChannel::UiOnly,
            is_error: true,
        }
    }
}

/// What the loop should do after a command ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    /// The command handled itself; nothing goes to the model.
    Done(CommandFeedback),
    /// The command rewrote the input. The text replaces what the user typed
    /// and is sent as a normal prompt — this is how a command expands into a
    /// longer instruction (`/review` becoming the full review prompt).
    Inject(String),
}

/// A registered command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRoute {
    /// The full lowercase name, as [`CommandName::full`] produces.
    pub full: String,
    /// The domain this came from, for grouping in the UI. `None` for a
    /// built-in with no explicit domain.
    pub domain: Option<String>,
    pub description: String,
    /// Extra names that resolve here.
    pub aliases: Vec<String>,
    pub args: ArgsSchema,
    /// The placeholder shown in the composer when the command takes input.
    pub argument_hint: Option<String>,
}

/// The registry the loop resolves against.
///
/// A flat map keyed by lowercase full name, plus an alias index. Flat rather
/// than nested because lookup is by exact name after parsing — the hierarchy
/// lives in the *name*, not in the data structure, so a lookup is one hash
/// probe regardless of how deep the name is.
#[derive(Debug, Default)]
pub struct CommandRegistry {
    routes: Vec<CommandRoute>,
    /// Full name → position in `routes`.
    index: HashMap<String, usize>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a command, or replace one with the same full name.
    ///
    /// Replacing keeps the position, so a host can shadow a built-in without
    /// reordering the rest — the same rule [`crate::tool::ToolRegistry`] uses,
    /// and for the same reason: the model-facing list should be stable.
    pub fn register(&mut self, route: CommandRoute) {
        let full = route.full.to_lowercase();
        match self.index.get(&full) {
            Some(&at) => self.routes[at] = route,
            None => {
                self.index.insert(full, self.routes.len());
                self.routes.push(route);
            }
        }
    }

    /// Find the command `name` resolves to.
    ///
    /// A bare name is looked up in the built-in domain: `/compact` finds
    /// `core:compact`. An explicit name must match exactly, so a project
    /// cannot register `foo` and answer for `/core:foo`.
    pub fn resolve(&self, name: &CommandName) -> Option<&CommandRoute> {
        match name {
            CommandName::Bare { name } => {
                // A bare name prefers the unqualified entry, then the built-in
                // domain. Both spellings mean the same thing to the user, and
                // which one an author wrote should not change what runs.
                self.get(name).or_else(|| self.get(&format!("core:{name}")))
            }
            other => self.get(&other.full()),
        }
    }

    fn get(&self, full: &str) -> Option<&CommandRoute> {
        self.index
            .get(&full.to_lowercase())
            .map(|&at| &self.routes[at])
    }

    /// Find a command by an alias.
    pub fn resolve_alias(&self, alias: &str) -> Option<&CommandRoute> {
        let alias = alias.to_lowercase();
        self.routes
            .iter()
            .find(|route| route.aliases.iter().any(|a| a.to_lowercase() == alias))
    }

    /// Every command, in registration order.
    pub fn all(&self) -> &[CommandRoute] {
        &self.routes
    }

    pub fn len(&self) -> usize {
        self.routes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }
}

/// A command the user typed, with its arguments parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandInvocation {
    pub name: CommandName,
    pub args: ParsedArgs,
    /// The raw argument text, kept because a command that injects a prompt
    /// wants what the user typed rather than a re-joined token list.
    pub raw_args: String,
}

/// The result of looking at one line of user input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedInput {
    /// Not a command; send it to the model as written.
    Prompt(String),
    /// A command, resolved.
    Command(Box<CommandInvocation>),
    /// It looked like a command but the name or arguments did not fit.
    Invalid { message: String },
}

/// Parse one line of composer input.
///
/// A line is a command only when it *starts* with `/` — a slash mid-sentence
/// is prose, and a path like `src/lib.rs` must not be mistaken for one. The
/// leading `/` is stripped before the name is parsed.
///
/// `registry` is consulted for argument validation only; an unknown name still
/// parses (as a bare name with free-form args) so the caller can report "no
/// such command" against a real name rather than a parse error.
pub fn parse_input(input: &str, registry: &CommandRegistry) -> ParsedInput {
    let trimmed = input.trim_start();
    let Some(rest) = trimmed.strip_prefix('/') else {
        return ParsedInput::Prompt(input.to_owned());
    };
    // `/` alone, or `//path`, is not a command.
    if rest.is_empty() || rest.starts_with('/') {
        return ParsedInput::Prompt(input.to_owned());
    }

    let (name_text, raw_args) = match rest.split_once(char::is_whitespace) {
        Some((name, args)) => (name, args.trim().to_owned()),
        None => (rest, String::new()),
    };

    let name = match CommandName::parse(name_text) {
        Ok(name) => name,
        Err(error) => {
            return ParsedInput::Invalid {
                message: error.to_string(),
            };
        }
    };

    let route = registry
        .resolve(&name)
        .or_else(|| registry.resolve_alias(&name.full()));

    // Validate against the command's schema when there is one. An unknown
    // command is handed back with unvalidated args so the caller's "unknown
    // command" message can name it.
    let args = match route {
        Some(route) => match route.args.parse(&raw_args) {
            Ok(args) => args,
            Err(message) => {
                return ParsedInput::Invalid {
                    message: format!("/{}: {message}", name.full()),
                };
            }
        },
        None => ArgsSchema::default().parse(&raw_args).unwrap_or_default(),
    };

    ParsedInput::Command(Box::new(CommandInvocation {
        name,
        args,
        raw_args,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_name_parses_without_a_domain() {
        let name = CommandName::parse("compact").expect("parses");
        assert_eq!(
            name,
            CommandName::Bare {
                name: "compact".to_owned()
            }
        );
        assert_eq!(name.domain(), None);
        assert_eq!(name.name(), "compact");
        assert_eq!(name.full(), "compact");
        assert!(name.is_bare());
    }

    #[test]
    fn a_qualified_name_splits_on_the_colon() {
        let name = CommandName::parse("core:compact").expect("parses");
        assert_eq!(
            name,
            CommandName::Qualified {
                domain: "core".to_owned(),
                name: "compact".to_owned()
            }
        );
        assert_eq!(name.domain(), Some("core"));
        assert_eq!(name.full(), "core:compact");
    }

    #[test]
    fn a_namespaced_name_splits_on_the_rightmost_colon() {
        // Splitting leftmost would put `acme:deploy` in the name, which is not
        // a command anyone registered.
        let name = CommandName::parse("plugin:acme:deploy").expect("parses");
        assert_eq!(
            name,
            CommandName::Namespaced {
                domain: "plugin".to_owned(),
                namespace: "acme".to_owned(),
                name: "deploy".to_owned()
            }
        );
        assert_eq!(name.name(), "deploy");
        assert_eq!(name.full(), "plugin:acme:deploy");
    }

    #[test]
    fn names_are_lowercased_so_two_casings_are_one_command() {
        assert_eq!(
            CommandName::parse("Core:Compact").expect("parses").full(),
            "core:compact"
        );
    }

    #[test]
    fn three_colons_is_refused() {
        let error = CommandName::parse("a:b:c:d").expect_err("refused");
        assert_eq!(error, CommandNameError::TooManySegments);
    }

    #[test]
    fn an_empty_name_or_segment_is_refused() {
        assert_eq!(
            CommandName::parse("").expect_err("refused"),
            CommandNameError::Empty
        );
        assert_eq!(
            CommandName::parse("  ").expect_err("refused"),
            CommandNameError::Empty
        );
        assert_eq!(
            CommandName::parse("core:").expect_err("refused"),
            CommandNameError::EmptySegment
        );
        assert_eq!(
            CommandName::parse(":compact").expect_err("refused"),
            CommandNameError::EmptySegment
        );
        assert_eq!(
            CommandName::parse("a::b").expect_err("refused"),
            CommandNameError::EmptySegment
        );
    }

    #[test]
    fn the_deprecated_double_underscore_is_refused() {
        // A name from before the colon syntax must fail loudly rather than
        // resolve to a command the user did not mean.
        assert_eq!(
            CommandName::parse("mcp__server__tool").expect_err("refused"),
            CommandNameError::DoubleUnderscore
        );
    }

    #[test]
    fn whitespace_and_slashes_inside_a_name_are_refused() {
        assert!(matches!(
            CommandName::parse("a b").expect_err("refused"),
            CommandNameError::IllegalCharacter(' ')
        ));
        assert!(matches!(
            CommandName::parse("a/b").expect_err("refused"),
            CommandNameError::IllegalCharacter('/')
        ));
    }

    // === Arguments ===

    #[test]
    fn a_free_form_schema_accepts_anything_as_positionals() {
        // `/bg do the thing` must work without a schema per sentence.
        let schema = ArgsSchema::default();
        let parsed = schema.parse("do the thing").expect("parses");
        assert_eq!(parsed.positionals, vec!["do", "the", "thing"]);
        assert_eq!(parsed.joined(), "do the thing");
    }

    #[test]
    fn positionals_match_in_declaration_order() {
        let schema = ArgsSchema {
            positionals: vec![
                ArgSpec::required("from", ArgKind::Text),
                ArgSpec::required("to", ArgKind::Text),
            ],
            ..ArgsSchema::default()
        };
        let parsed = schema.parse("main release").expect("parses");
        assert_eq!(parsed.positional(0), Some("main"));
        assert_eq!(parsed.positional(1), Some("release"));
        assert_eq!(parsed.positional(2), None);
    }

    #[test]
    fn too_many_positionals_is_an_error() {
        let schema = ArgsSchema {
            positionals: vec![ArgSpec::required("only", ArgKind::Text)],
            ..ArgsSchema::default()
        };
        assert_eq!(
            schema.parse("one two").expect_err("refused"),
            "unexpected argument `two`"
        );
    }

    #[test]
    fn a_missing_required_positional_is_an_error() {
        let schema = ArgsSchema {
            positionals: vec![ArgSpec::required("target", ArgKind::Text)],
            ..ArgsSchema::default()
        };
        assert_eq!(
            schema.parse("").expect_err("refused"),
            "`target` is required"
        );
    }

    #[test]
    fn an_integer_argument_must_parse_as_one() {
        let schema = ArgsSchema {
            positionals: vec![ArgSpec::required("count", ArgKind::Integer)],
            ..ArgsSchema::default()
        };
        assert_eq!(schema.parse("5").expect("parses").positional(0), Some("5"));
        assert!(
            schema
                .parse("five")
                .expect_err("refused")
                .contains("whole number")
        );
    }

    #[test]
    fn a_choice_argument_must_be_one_of_its_candidates() {
        let schema = ArgsSchema {
            positionals: vec![ArgSpec::required(
                "mode",
                ArgKind::Choice(vec!["fast".to_owned(), "thorough".to_owned()]),
            )],
            ..ArgsSchema::default()
        };
        assert!(schema.parse("fast").is_ok());
        let error = schema.parse("reckless").expect_err("refused");
        assert!(error.contains("fast, thorough"), "got: {error}");
    }

    #[test]
    fn an_empty_choice_set_accepts_anything() {
        // An empty candidate list means the handler supplies candidates at
        // runtime, so the parser cannot check and must not guess.
        let schema = ArgsSchema {
            positionals: vec![ArgSpec::required("target", ArgKind::Choice(Vec::new()))],
            ..ArgsSchema::default()
        };
        assert!(schema.parse("whatever").is_ok());
    }

    #[test]
    fn a_path_argument_must_exist() {
        let dir = tempfile::tempdir().expect("temp dir");
        let schema = ArgsSchema {
            positionals: vec![ArgSpec::required("target", ArgKind::Path)],
            ..ArgsSchema::default()
        };
        let existing = dir.path().to_string_lossy().into_owned();
        assert!(schema.parse(&existing).is_ok());
        assert!(
            schema
                .parse("definitely-not-here-12345")
                .expect_err("refused")
                .contains("does not exist")
        );
    }

    #[test]
    fn named_arguments_take_the_next_token_as_their_value() {
        let schema = ArgsSchema {
            named: vec![ArgSpec::required("model", ArgKind::Text)],
            ..ArgsSchema::default()
        };
        let parsed = schema.parse("--model gpt-5").expect("parses");
        assert_eq!(parsed.named_value("model"), Some("gpt-5"));
    }

    #[test]
    fn a_named_argument_without_a_value_is_an_error() {
        let schema = ArgsSchema {
            named: vec![ArgSpec::required("model", ArgKind::Text)],
            ..ArgsSchema::default()
        };
        assert_eq!(
            schema.parse("--model").expect_err("refused"),
            "`--model` needs a value"
        );
    }

    #[test]
    fn a_missing_required_named_argument_is_an_error() {
        let schema = ArgsSchema {
            named: vec![ArgSpec::required("model", ArgKind::Text)],
            ..ArgsSchema::default()
        };
        assert_eq!(
            schema.parse("").expect_err("refused"),
            "`--model` is required"
        );
    }

    #[test]
    fn an_unknown_option_is_refused_rather_than_treated_as_text() {
        let schema = ArgsSchema {
            named: vec![ArgSpec::optional("model", ArgKind::Text)],
            ..ArgsSchema::default()
        };
        // Silently accepting a typo'd flag would run the command with the
        // option missing and no indication why.
        assert!(
            schema
                .parse("--modle x")
                .expect_err("refused")
                .contains("unknown option")
        );
    }

    #[test]
    fn flags_are_presence_only_in_both_forms() {
        let schema = ArgsSchema {
            flags: vec![FlagSpec::new("force").with_short("-f")],
            ..ArgsSchema::default()
        };
        assert!(schema.parse("--force").expect("parses").has_flag("force"));
        assert!(schema.parse("-f").expect("parses").has_flag("force"));
        assert!(!schema.parse("").expect("parses").has_flag("force"));
    }

    #[test]
    fn an_unknown_short_flag_is_refused() {
        let schema = ArgsSchema {
            flags: vec![FlagSpec::new("force").with_short("-f")],
            ..ArgsSchema::default()
        };
        assert!(
            schema
                .parse("-x")
                .expect_err("refused")
                .contains("unknown option")
        );
    }

    #[test]
    fn arguments_mix_positionals_named_and_flags() {
        let schema = ArgsSchema {
            positionals: vec![ArgSpec::required("target", ArgKind::Text)],
            named: vec![ArgSpec::optional("model", ArgKind::Text)],
            flags: vec![FlagSpec::new("dry-run")],
        };
        let parsed = schema
            .parse("main --model gpt-5 --dry-run")
            .expect("parses");
        assert_eq!(parsed.positional(0), Some("main"));
        assert_eq!(parsed.named_value("model"), Some("gpt-5"));
        assert!(parsed.has_flag("dry-run"));
    }

    // === Registry ===

    fn route(full: &str) -> CommandRoute {
        CommandRoute {
            full: full.to_owned(),
            domain: full.split(':').next().map(str::to_owned),
            description: format!("the {full} command"),
            aliases: Vec::new(),
            args: ArgsSchema::default(),
            argument_hint: None,
        }
    }

    #[test]
    fn a_bare_name_resolves_to_the_built_in_domain() {
        let mut registry = CommandRegistry::new();
        registry.register(route("core:compact"));

        let bare = CommandName::parse("compact").expect("parses");
        assert_eq!(
            registry.resolve(&bare).map(|r| r.full.as_str()),
            Some("core:compact")
        );

        let explicit = CommandName::parse("core:compact").expect("parses");
        assert_eq!(
            registry.resolve(&explicit).map(|r| r.full.as_str()),
            Some("core:compact")
        );
    }

    #[test]
    fn a_bare_name_prefers_an_unqualified_entry() {
        // Both spellings mean the same thing to the user, so whichever the
        // author wrote, the unqualified one they can see wins.
        let mut registry = CommandRegistry::new();
        registry.register(route("compact"));
        registry.register(route("core:compact"));

        let bare = CommandName::parse("compact").expect("parses");
        assert_eq!(
            registry.resolve(&bare).map(|r| r.full.as_str()),
            Some("compact")
        );
    }

    #[test]
    fn an_explicit_name_does_not_fall_back_to_a_bare_one() {
        let mut registry = CommandRegistry::new();
        registry.register(route("compact"));

        let explicit = CommandName::parse("plugin:acme:compact").expect("parses");
        assert!(registry.resolve(&explicit).is_none());
    }

    #[test]
    fn registering_the_same_name_replaces_in_place() {
        let mut registry = CommandRegistry::new();
        registry.register(route("core:a"));
        registry.register(route("core:b"));
        let mut replacement = route("core:a");
        replacement.description = "replaced".to_owned();
        registry.register(replacement);

        assert_eq!(registry.len(), 2);
        assert_eq!(registry.all()[0].full, "core:a");
        assert_eq!(registry.all()[0].description, "replaced");
    }

    #[test]
    fn an_alias_resolves_to_its_command() {
        let mut registry = CommandRegistry::new();
        let mut compact = route("core:compact");
        compact.aliases = vec!["c".to_owned()];
        registry.register(compact);

        assert_eq!(
            registry.resolve_alias("c").map(|r| r.full.as_str()),
            Some("core:compact")
        );
        assert!(registry.resolve_alias("nope").is_none());
    }

    // === Parsing input ===

    #[test]
    fn plain_text_is_a_prompt() {
        assert_eq!(
            parse_input("fix the login bug", &CommandRegistry::new()),
            ParsedInput::Prompt("fix the login bug".to_owned())
        );
    }

    #[test]
    fn a_path_is_not_mistaken_for_a_command() {
        // The slash is mid-sentence, and `src/lib.rs` must reach the model.
        assert_eq!(
            parse_input("read src/lib.rs", &CommandRegistry::new()),
            ParsedInput::Prompt("read src/lib.rs".to_owned())
        );
    }

    #[test]
    fn a_lone_slash_is_a_prompt() {
        assert_eq!(
            parse_input("/", &CommandRegistry::new()),
            ParsedInput::Prompt("/".to_owned())
        );
        assert_eq!(
            parse_input("//comment", &CommandRegistry::new()),
            ParsedInput::Prompt("//comment".to_owned())
        );
    }

    #[test]
    fn a_command_parses_with_its_arguments() {
        let mut registry = CommandRegistry::new();
        registry.register(route("core:compact"));

        let ParsedInput::Command(invocation) = parse_input("/compact now", &registry) else {
            panic!("expected a command");
        };
        assert_eq!(invocation.name.full(), "compact");
        assert_eq!(invocation.raw_args, "now");
        assert_eq!(invocation.args.positionals, vec!["now"]);
    }

    #[test]
    fn leading_whitespace_before_the_slash_is_tolerated() {
        let ParsedInput::Command(invocation) = parse_input("  /compact", &CommandRegistry::new())
        else {
            panic!("expected a command");
        };
        assert_eq!(invocation.name.full(), "compact");
        assert!(invocation.raw_args.is_empty());
    }

    #[test]
    fn a_bad_name_is_reported_as_invalid_not_as_a_prompt() {
        // Sending `a:b:c:d` to the model as prose would be baffling; the user
        // typed a command and it should be reported as one.
        let result = parse_input("/a:b:c:d", &CommandRegistry::new());
        assert!(matches!(result, ParsedInput::Invalid { .. }));
    }

    #[test]
    fn a_bad_argument_is_reported_with_the_command_name() {
        let mut registry = CommandRegistry::new();
        let mut counted = route("core:repeat");
        counted.args = ArgsSchema {
            positionals: vec![ArgSpec::required("count", ArgKind::Integer)],
            ..ArgsSchema::default()
        };
        registry.register(counted);

        let ParsedInput::Invalid { message } = parse_input("/repeat many", &registry) else {
            panic!("expected invalid");
        };
        // The message echoes the name as the user typed it, not the resolved
        // full name: correcting `/repeat` when they never wrote `core:` would
        // read as a different command.
        assert!(message.starts_with("/repeat:"), "got: {message}");
        assert!(message.contains("whole number"));
    }

    #[test]
    fn an_unknown_command_still_parses_so_the_caller_can_name_it() {
        // The registry is the authority on existence, and the loop reports
        // "no such command" against a real name rather than a parse error.
        let ParsedInput::Command(invocation) =
            parse_input("/nosuchthing with args", &CommandRegistry::new())
        else {
            panic!("expected a command");
        };
        assert_eq!(invocation.name.full(), "nosuchthing");
        assert_eq!(invocation.raw_args, "with args");
    }

    #[test]
    fn an_alias_typed_by_the_user_resolves_through_the_registry() {
        let mut registry = CommandRegistry::new();
        let mut compact = route("core:compact");
        compact.aliases = vec!["c".to_owned()];
        registry.register(compact);

        let ParsedInput::Command(invocation) = parse_input("/c", &registry) else {
            panic!("expected a command");
        };
        assert!(registry.resolve_alias(&invocation.name.full()).is_some());
    }

    #[test]
    fn feedback_defaults_to_the_user_and_can_join_the_session() {
        assert_eq!(CommandFeedback::ui("done").channel, FeedbackChannel::UiOnly);
        assert_eq!(
            CommandFeedback::session("done").channel,
            FeedbackChannel::Session
        );
        assert!(CommandFeedback::error("nope").is_error);
        assert!(!CommandFeedback::ui("ok").is_error);
    }
}
