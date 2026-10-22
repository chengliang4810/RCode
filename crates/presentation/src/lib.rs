//! Text and formatting that depend on the display language.
//!
//! One crate owns every string the user reads, so the answer to "what language
//! is this in" has a single home. That matters more than it sounds: before
//! this, display text was produced in `rcode-proto` (tool card labels,
//! relative times) and in `rcode-agent` (permission mode names), two layers
//! that execute a run and have no business knowing what language the window is
//! in.
//!
//! # What is here
//!
//! [`Localizer`] resolves a [`LanguagePreference`] against the system locale
//! and looks messages up in the compiled-in resources. [`TextKey`] is
//! generated from the English resources at build time, so a key that does not
//! exist fails to compile.
//!
//! On top of the lookup sit the pure display functions that were reaching for
//! raw English literals before: [`tools::tool_chip_content`],
//! [`tools::tool_group_summary`], [`time::format_time_ago`]. Each takes a
//! `&Localizer` and returns an owned `String`, so a translated message is built
//! rather than borrowed from a table.
//!
//! # What is not
//!
//! No GPUI, no engine, no agent, no global mutable state. A `Localizer` is
//! passed explicitly by whoever needs it, which is what lets a test ask what a
//! message looks like in Chinese without a window — and what stops one
//! window's language from becoming every window's language.
//!
//! The resources are parsed once, when the first `Localizer` is built, and the
//! bundle is the concurrent one: several windows and the token-folding path
//! all read from it, and re-parsing per read would show up in a profile.

use std::sync::Arc;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
use unic_langid::LanguageIdentifier;

pub mod assessment;
pub mod time;
pub mod tools;
pub mod workspace;

/// What the user chose, before it meets the system's own preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LanguagePreference {
    /// Follow the operating system. The default, because a user who has told
    /// their computer which language they read should not have to say it twice.
    #[default]
    System,
    EnUs,
    ZhCn,
}

impl LanguagePreference {
    /// Every value, for a picker.
    pub const ALL: [LanguagePreference; 3] = [Self::System, Self::EnUs, Self::ZhCn];

    /// Parse a stored value, falling back to [`Self::System`].
    ///
    /// An unknown value is not an error: a settings file written by a newer
    /// build, or edited by hand, should cost the user their language choice
    /// and nothing else. Refusing to parse the whole file would take the
    /// memory and skill switches with it.
    pub fn parse(value: &str) -> Self {
        match value {
            "enUs" => Self::EnUs,
            "zhCn" => Self::ZhCn,
            _ => Self::System,
        }
    }

    /// The stable name a settings file stores.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::EnUs => "enUs",
            Self::ZhCn => "zhCn",
        }
    }
}

/// A language the application actually has resources for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Locale {
    EnUs,
    ZhCn,
}

impl Locale {
    /// The language tag the resources are filed under.
    pub fn tag(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::ZhCn => "zh-CN",
        }
    }

    /// The identifier Fluent matches against.
    pub fn language_identifier(self) -> LanguageIdentifier {
        self.tag()
            .parse()
            .expect("the tags here are constants and parse")
    }
}

/// Why a message could not be produced.
///
/// Carries the key and the failure kind, never the arguments: an argument may
/// be a path, a provider's message or a piece of the user's own text, and this
/// error goes to a log. The caller decides what to show — see the fallback in
/// [`Localizer::text`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    #[error("the resource for `{key}` is missing")]
    Missing { key: String },
    #[error("the resource for `{key}` could not be formatted")]
    Format { key: String },
}

include!(concat!(env!("OUT_DIR"), "/text_keys.rs"));

/// The resources, parsed once and shared.
struct Resources {
    /// One bundle per language. Fluent's concurrent bundle is `Send + Sync`,
    /// so a window, a background fold and a test can all read it at once.
    bundles: std::collections::HashMap<Locale, FluentBundle<FluentResource>>,
}

/// The messages for the language this session is in.
///
/// Cheap to clone: the resources are behind an [`Arc`] and shared. Hold one
/// per window or per test, and pass it explicitly — there is deliberately no
/// global, because a global would make two windows unable to differ and a test
/// unable to ask for a specific language.
#[derive(Clone)]
pub struct Localizer {
    locale: Locale,
    resources: Arc<Resources>,
}

impl Localizer {
    /// Build a localizer for `preference`, reading the system locale when the
    /// preference asks for it.
    pub fn new(preference: LanguagePreference) -> Self {
        Self::with_system_locale(preference, sys_locale::get_locale().as_deref())
    }

    /// Build a localizer with the system locale supplied.
    ///
    /// The seam [`Self::new`] uses, and the only way to test the system
    /// preference: a test cannot change what the operating system reports, so
    /// the value it would have read is passed in.
    pub fn with_system_locale(preference: LanguagePreference, system_locale: Option<&str>) -> Self {
        Self {
            locale: resolve(preference, system_locale),
            resources: resources(),
        }
    }

    /// The language in use.
    pub fn locale(&self) -> Locale {
        self.locale
    }

    /// A message with no arguments.
    pub fn text(&self, key: TextKey) -> String {
        self.format(key, &FluentArgs::new()).unwrap_or_else(|_| {
            // The build checks that both languages define every key, so a
            // missing one means a language was added without its resources, or
            // a resource failed to parse at startup. Falling back to English
            // keeps the UI readable; showing the key would not.
            self.fallback(key, &FluentArgs::new())
        })
    }

    /// A message with arguments, which must match the resource's placeables.
    ///
    /// Returning a `String` rather than a borrowed one is the point: the old
    /// signatures returned `&'static str`, which forced callers to leak or to
    /// keep a table of literals. A translated message is built, not looked up,
    /// so it is owned.
    pub fn format_with(&self, key: TextKey, args: &[(&str, &str)]) -> String {
        let mut fluent = FluentArgs::new();
        for (name, value) in args {
            fluent.set(*name, (*value).to_owned());
        }
        self.format(key, &fluent)
            .unwrap_or_else(|_| self.fallback(key, &fluent))
    }

    /// A message with a number, for the plural rules.
    ///
    /// Separate from [`Self::format_with`] because Fluent's plural selection
    /// needs a number, not a string: passing `"2"` would match no variant and
    /// the message would render as its fallback.
    pub fn format_count(&self, key: TextKey, name: &str, count: i64) -> String {
        let mut fluent = FluentArgs::new();
        fluent.set(name, count);
        self.format(key, &fluent)
            .unwrap_or_else(|_| self.fallback(key, &fluent))
    }

    /// Format strictly: the error, if any, rather than a fallback.
    ///
    /// For tests and for the build check, where a silent fallback would hide
    /// exactly the failure being looked for.
    pub fn try_text(&self, key: TextKey) -> Result<String, TextError> {
        self.format(key, &FluentArgs::new())
    }

    /// Format strictly, with one named value, for the coverage check.
    ///
    /// The argument-less [`Self::try_text`] cannot format a message that needs
    /// one, so a check over every key needs a way to supply a plausible value
    /// per name. This is that seam; it is not a convenience for callers, who
    /// know their own arguments.
    pub fn try_text_with(
        &self,
        key: TextKey,
        name: &str,
        value: &str,
    ) -> Result<String, TextError> {
        let mut args = FluentArgs::new();
        args.set(name, value.to_owned());
        self.format(key, &args)
    }

    /// Look the message up without a fallback.
    fn format(&self, key: TextKey, args: &FluentArgs) -> Result<String, TextError> {
        let Some(bundle) = self.resources.bundles.get(&self.locale) else {
            return Err(TextError::Missing {
                key: key.id().to_owned(),
            });
        };
        let message = bundle
            .get_message(key.id())
            .ok_or_else(|| TextError::Missing {
                key: key.id().to_owned(),
            })?;
        let Some(pattern) = message.value() else {
            return Err(TextError::Missing {
                key: key.id().to_owned(),
            });
        };
        let mut errors = Vec::new();
        let formatted = bundle.format_pattern(pattern, Some(args), &mut errors);
        if !errors.is_empty() {
            return Err(TextError::Format {
                key: key.id().to_owned(),
            });
        }
        Ok(formatted.into_owned())
    }

    /// The English message, for a lookup that failed in the current language.
    ///
    /// Returns the key itself if English is missing too — visibly wrong, which
    /// is what a developer needs to see, and better than an empty string that
    /// looks like a message with no text.
    fn fallback(&self, key: TextKey, args: &FluentArgs) -> String {
        let english = Locale::EnUs;
        if self.locale == english {
            tracing_missing(key);
            return key.id().to_owned();
        }
        let Some(bundle) = self.resources.bundles.get(&english) else {
            return key.id().to_owned();
        };
        let Some(message) = bundle.get_message(key.id()) else {
            return key.id().to_owned();
        };
        let Some(pattern) = message.value() else {
            return key.id().to_owned();
        };
        let mut errors = Vec::new();
        let formatted = bundle.format_pattern(pattern, Some(args), &mut errors);
        if !errors.is_empty() {
            return key.id().to_owned();
        }
        // Only the key and the kind of failure: the arguments may hold a path,
        // a provider's message or the user's own text.
        tracing::warn!(
            key = key.id(),
            locale = self.locale.tag(),
            "the message is missing or unformattable; served English"
        );
        formatted.into_owned()
    }
}

/// Note a lookup that failed in the current language, without the arguments.
fn tracing_missing(key: TextKey) {
    tracing::warn!(key = key.id(), "the message is missing from the resources");
}

/// Resolve a preference against the system locale.
///
/// The rules, and the reason for each:
///
/// - A manual choice always wins. A user who set English on a Chinese system
///   meant it.
/// - `System` with a `zh` primary subtag is Chinese — `zh-Hans`, `zh-Hant` and
///   `zh-TW` all resolve here, because the application ships one Chinese
///   translation and offering none of them would be worse than offering
///   simplified script to a traditional-script reader.
/// - Everything else, **including a locale that could not be read**, is
///   English. Guessing is worse than a default: an unreadable locale is not a
///   reason to show nothing.
pub fn resolve(preference: LanguagePreference, system_locale: Option<&str>) -> Locale {
    match preference {
        LanguagePreference::EnUs => Locale::EnUs,
        LanguagePreference::ZhCn => Locale::ZhCn,
        LanguagePreference::System => match system_locale {
            Some(locale) if primary_subtag(locale) == Some("zh") => Locale::ZhCn,
            _ => Locale::EnUs,
        },
    }
}

/// The language part of a locale tag, lowercased: `zh-Hans-CN` → `zh`.
fn primary_subtag(locale: &str) -> Option<&str> {
    let primary = locale.split(['-', '_']).next()?;
    (!primary.is_empty()).then_some(primary)
}

/// The shared resources, parsed on first use.
fn resources() -> Arc<Resources> {
    static RESOURCES: std::sync::OnceLock<Arc<Resources>> = std::sync::OnceLock::new();
    Arc::clone(RESOURCES.get_or_init(|| {
        let mut bundles = std::collections::HashMap::new();
        for locale in [Locale::EnUs, Locale::ZhCn] {
            bundles.insert(locale, load_bundle(locale));
        }
        Arc::new(Resources { bundles })
    }))
}

/// Every resource file, compiled in.
///
/// `include_str!` rather than a runtime directory read: the strings ship inside
/// the binary, so there is no path to get wrong and no file a user can delete
/// to make the UI unreadable.
const SOURCES: [(Locale, &str); 20] = [
    (Locale::EnUs, include_str!("../locales/en-US/common.ftl")),
    (Locale::EnUs, include_str!("../locales/en-US/shell.ftl")),
    (
        Locale::EnUs,
        include_str!("../locales/en-US/conversation.ftl"),
    ),
    (Locale::EnUs, include_str!("../locales/en-US/providers.ftl")),
    (Locale::EnUs, include_str!("../locales/en-US/settings.ftl")),
    (Locale::EnUs, include_str!("../locales/en-US/files.ftl")),
    (Locale::EnUs, include_str!("../locales/en-US/git.ftl")),
    (Locale::EnUs, include_str!("../locales/en-US/browser.ftl")),
    (Locale::EnUs, include_str!("../locales/en-US/terminal.ftl")),
    (Locale::EnUs, include_str!("../locales/en-US/errors.ftl")),
    (Locale::ZhCn, include_str!("../locales/zh-CN/common.ftl")),
    (Locale::ZhCn, include_str!("../locales/zh-CN/shell.ftl")),
    (
        Locale::ZhCn,
        include_str!("../locales/zh-CN/conversation.ftl"),
    ),
    (Locale::ZhCn, include_str!("../locales/zh-CN/providers.ftl")),
    (Locale::ZhCn, include_str!("../locales/zh-CN/settings.ftl")),
    (Locale::ZhCn, include_str!("../locales/zh-CN/files.ftl")),
    (Locale::ZhCn, include_str!("../locales/zh-CN/git.ftl")),
    (Locale::ZhCn, include_str!("../locales/zh-CN/browser.ftl")),
    (Locale::ZhCn, include_str!("../locales/zh-CN/terminal.ftl")),
    (Locale::ZhCn, include_str!("../locales/zh-CN/errors.ftl")),
];

fn load_bundle(locale: Locale) -> FluentBundle<FluentResource> {
    let identifier = locale.language_identifier();
    let mut bundle = FluentBundle::new_concurrent(vec![identifier]);
    // A formatting error should not silently drop the message: the caller
    // decides, and `format` reports it. Fluent's default is to swallow them
    // into the output as placeholders.
    bundle.set_use_isolating(false);
    for (source_locale, source) in SOURCES {
        if source_locale != locale {
            continue;
        }
        let resource = FluentResource::try_new(source.to_owned()).unwrap_or_else(|(_, errors)| {
            panic!("the {} resource is invalid: {errors:?}", locale.tag())
        });
        bundle
            .add_resource(resource)
            .unwrap_or_else(|errors| panic!("{} has a duplicate key: {errors:?}", locale.tag()));
    }
    bundle
}

#[cfg(test)]
mod tests;
