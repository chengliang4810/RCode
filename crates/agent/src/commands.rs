//! The commands the agent ships with.
//!
//! One: `core:compact`.
//!
//! The test a command has to pass is whether the model could do the same thing
//! if asked. Nearly everything passes that test — "remember this", "forget
//! that", "start over" are all things a model can be told, and expressing them
//! as prompts leaves the model free to interpret them in context. `/compact`
//! fails it: the model cannot summarize a conversation it can no longer see,
//! and the moment the context is full is exactly when it stops being able to
//! act at all. So compaction is the one thing that must be a command.
//!
//! Two candidates were deliberately dropped. `/clear` ("the model stops seeing
//! the history, but it stays on screen") duplicated the desktop's New session
//! button while leaving the transcript in a state a user cannot read — the
//! messages are there but the model is pretending they are not. `/rewind`
//! duplicated editing a message. A command that repeats a visible control is
//! one more thing to learn for nothing.
//!
//! # Why the handler is not here
//!
//! A command's *definition* (name, arguments, description) is data the registry
//! and the composer's completion list both read, so it lives here. Its *effect*
//! needs the conversation document, which only the engine holds. The two are
//! separate so this module stays pure data — no async, no state — and the
//! engine matches on [`BuiltinCommand`] to run the effect it knows how to run.

use crate::command::{ArgsSchema, CommandName, CommandRegistry, CommandRoute};

/// The built-in commands, as a closed set.
///
/// An enum rather than a trait object because the engine must *recognise* a
/// command to act on it, and matching a variant is how that recognition stays
/// exhaustive: adding a command here makes the engine's match fail to compile
/// until it handles the new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinCommand {
    /// Summarize the older part of the conversation and keep the recent turns.
    Compact,
}

impl BuiltinCommand {
    /// The full name, matching the route registered in [`registry`].
    pub fn full_name(self) -> &'static str {
        match self {
            Self::Compact => "core:compact",
        }
    }

    /// Every built-in, for iteration.
    pub const ALL: [BuiltinCommand; 1] = [Self::Compact];

    /// Recognise a resolved command name.
    ///
    /// Accepts the bare form as well as the qualified one: the user types
    /// `/compact`, and a bare name resolves against the built-in domain, so
    /// comparing only against `core:compact` would never match what they sent.
    pub fn from_name(name: &CommandName) -> Option<Self> {
        let full = name.full();
        Self::ALL.into_iter().find(|command| {
            let registered = command.full_name();
            registered == full
                || registered
                    .strip_prefix("core:")
                    .is_some_and(|bare| bare == full)
        })
    }
}

/// The route for one built-in.
fn route(command: BuiltinCommand) -> CommandRoute {
    let (description, aliases, args, hint) = match command {
        BuiltinCommand::Compact => (
            "Summarize the earlier part of this conversation and keep the recent \
             turns, freeing context. Use it when the conversation is getting long.",
            vec!["c".to_owned()],
            // Free-form on purpose: `/compact keep the database decisions` is
            // one instruction, not five arguments. A schema that counted words
            // would reject the sentence a user actually types, and the handler
            // wants the whole tail as written.
            ArgsSchema::default(),
            Some("what to keep in mind"),
        ),
    };

    CommandRoute {
        full: command.full_name().to_owned(),
        domain: Some("core".to_owned()),
        description: description.to_owned(),
        aliases,
        args,
        argument_hint: hint.map(str::to_owned),
    }
}

/// A registry holding every built-in.
pub fn registry() -> CommandRegistry {
    let mut registry = CommandRegistry::new();
    for command in BuiltinCommand::ALL {
        registry.register(route(command));
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{ParsedInput, parse_input};

    #[test]
    fn the_only_built_in_is_compact() {
        // Pinned deliberately: every command is a name the user must learn, so
        // the set is something to argue about, not something to grow quietly.
        assert_eq!(BuiltinCommand::ALL, [BuiltinCommand::Compact]);
        assert_eq!(registry().len(), 1);
    }

    #[test]
    fn every_built_in_has_a_route() {
        let registry = registry();
        for command in BuiltinCommand::ALL {
            let name = CommandName::parse(command.full_name()).expect("parses");
            assert!(
                registry.resolve(&name).is_some(),
                "{} is not registered",
                command.full_name()
            );
        }
    }

    #[test]
    fn a_bare_name_and_its_domain_reach_the_same_command() {
        for spelling in ["compact", "core:compact"] {
            let ParsedInput::Command(invocation) =
                parse_input(&format!("/{spelling}"), &registry())
            else {
                panic!("/{spelling} did not parse");
            };
            assert_eq!(
                BuiltinCommand::from_name(&invocation.name),
                Some(BuiltinCommand::Compact),
                "/{spelling} did not reach Compact"
            );
        }
    }

    #[test]
    fn the_alias_resolves_to_compact() {
        assert_eq!(
            registry()
                .resolve_alias("c")
                .map(|route| route.full.as_str()),
            Some("core:compact")
        );
    }

    #[test]
    fn the_dropped_commands_are_not_registered() {
        // `/clear` and `/rewind` were removed on purpose (they duplicated the
        // New session button and message editing). A leftover registration
        // would keep them alive in the composer's list.
        let registry = registry();
        for gone in ["clear", "rewind", "reset", "undo"] {
            assert!(
                registry.resolve_alias(gone).is_none(),
                "`{gone}` is still registered"
            );
            let name = CommandName::parse(gone).expect("parses");
            assert!(registry.resolve(&name).is_none(), "`{gone}` still resolves");
            assert_eq!(BuiltinCommand::from_name(&name), None);
        }
    }

    #[test]
    fn compact_takes_free_text() {
        let registry = registry();
        let ParsedInput::Command(invocation) =
            parse_input("/compact keep the database decisions", &registry)
        else {
            panic!("expected a command");
        };
        assert_eq!(invocation.args.joined(), "keep the database decisions");
    }

    #[test]
    fn compact_works_with_no_arguments() {
        let ParsedInput::Command(invocation) = parse_input("/compact", &registry()) else {
            panic!("expected a command");
        };
        assert!(invocation.raw_args.is_empty());
        assert!(invocation.args.positionals.is_empty());
    }

    #[test]
    fn the_route_describes_itself_for_the_composer() {
        let registry = registry();
        let route = registry.resolve_alias("c").expect("registered");
        assert!(!route.description.is_empty());
        assert_eq!(route.argument_hint.as_deref(), Some("what to keep in mind"));
        assert_eq!(route.domain.as_deref(), Some("core"));
    }
}
