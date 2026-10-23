//! Generates `TextKey` from the English resources, and refuses to build a
//! translation set that cannot be trusted.
//!
//! Two jobs, and the second is the one that earns its keep.
//!
//! **Generation** turns the English keys into an enum, so a caller writes
//! `TextKey::ActionCancel` and a typo is a compile error rather than a
//! key that resolves to nothing at runtime — where it would show as the key
//! itself, or as an empty string, depending on which failure path ran.
//!
//! **Validation** checks the two languages against each other: same keys, same
//! parameters, no duplicates, no syntax errors. Fluent parses each file
//! independently and is perfectly happy with a Chinese file that has drifted —
//! it simply has no message for the key, and the fallback quietly serves
//! English. That is the right *runtime* behaviour and exactly the wrong
//! *build-time* one: without this check a missing translation looks like a
//! working app until someone reads the Chinese UI.
//!
//! The check runs here rather than in a test so it cannot be skipped: `cargo
//! build` fails, not `cargo test`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The languages, each with the domains they must all define.
const LANGUAGES: [&str; 2] = ["en-US", "zh-CN"];
/// The reference language: keys are generated from it and the others are
/// checked against it.
const REFERENCE: &str = "en-US";
/// Every domain file that must exist in every language.
const DOMAINS: [&str; 11] = [
    "common",
    "shell",
    "conversation",
    "providers",
    "settings",
    "files",
    "git",
    "browser",
    "terminal",
    "errors",
    "visual",
];

fn main() {
    let locales = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("locales");

    // Re-run when a resource changes; without this a new key is invisible
    // until something else touches the build.
    for language in LANGUAGES {
        for domain in DOMAINS {
            println!(
                "cargo:rerun-if-changed={}",
                locales
                    .join(language)
                    .join(format!("{domain}.ftl"))
                    .display()
            );
        }
    }

    let reference = parse_language(&locales, REFERENCE);

    check_all_languages_present(&locales);
    for language in LANGUAGES {
        let parsed = parse_language(&locales, language);
        if language != REFERENCE {
            check_agreement(&reference, &parsed, language);
        }
    }

    write_keys(&reference);
}

/// One message as the build needs to see it: its id and the placeables it
/// refers to.
#[derive(Debug, Clone)]
struct Message {
    /// The `$variables` the message uses, so the two languages can be compared
    /// on more than their key set.
    variables: BTreeSet<String>,
}

/// Parse every domain file of one language into one map.
///
/// `None` for a message's id means a placeable with no name, which Fluent
/// allows inside select expressions; only named ones are compared.
fn parse_language(locales: &Path, language: &str) -> BTreeMap<String, Message> {
    let mut all = BTreeMap::new();
    let mut duplicates = Vec::new();

    for domain in DOMAINS {
        let path = locales.join(language).join(format!("{domain}.ftl"));
        let source = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "the {language} resource `{}` could not be read: {error}",
                path.display()
            )
        });
        let resource =
            fluent_syntax::parser::parse_runtime(source.clone()).unwrap_or_else(|errors| {
                panic!(
                    "the {language} resource `{}` is not valid Fluent: {errors:?}",
                    path.display()
                )
            });

        for entry in resource.body {
            let fluent_syntax::ast::Entry::Message(message) = entry else {
                continue;
            };
            let id = message.id.name.clone();
            let mut variables = BTreeSet::new();
            if let Some(value) = &message.value {
                collect_variables(value, &mut variables);
            }
            for attribute in &message.attributes {
                collect_variables(&attribute.value, &mut variables);
            }
            if all.insert(id.clone(), Message { variables }).is_some() {
                duplicates.push(id);
            }
        }
    }

    if !duplicates.is_empty() {
        panic!(
            "{language} defines the same key more than once ({}) — one of them would \
             silently win",
            duplicates.join(", ")
        );
    }
    all
}

/// Collect the `$variables` a pattern refers to.
///
/// Only `$variables` are collected, not every reference: a message that
/// mentions another message by name takes no argument from the caller, so a
/// difference there is not an argument mismatch.
fn collect_variables(pattern: &fluent_syntax::ast::Pattern<String>, into: &mut BTreeSet<String>) {
    use fluent_syntax::ast::PatternElement;
    for element in &pattern.elements {
        match element {
            PatternElement::Placeable { expression } => collect_placeable(expression, into),
            PatternElement::TextElement { .. } => {}
        }
    }
}

fn collect_placeable(
    expression: &fluent_syntax::ast::Expression<String>,
    into: &mut BTreeSet<String>,
) {
    use fluent_syntax::ast::Expression;
    match expression {
        Expression::Select { selector, variants } => {
            collect_inline(selector, into);
            for variant in variants {
                collect_variables(&variant.value, into);
            }
        }
        Expression::Inline(inline) => collect_inline(inline, into),
    }
}

fn collect_inline(
    expression: &fluent_syntax::ast::InlineExpression<String>,
    into: &mut BTreeSet<String>,
) {
    use fluent_syntax::ast::InlineExpression;
    match expression {
        InlineExpression::VariableReference { id } => {
            into.insert(id.name.clone());
        }
        InlineExpression::Placeable { expression } => collect_placeable(expression, into),
        _ => {}
    }
}

/// Every language must define every domain file, even an empty one.
///
/// A missing file is otherwise indistinguishable from a file whose keys all
/// drifted, and both fail the agreement check below for the same reason.
fn check_all_languages_present(locales: &Path) {
    for language in LANGUAGES {
        for domain in DOMAINS {
            let path = locales.join(language).join(format!("{domain}.ftl"));
            assert!(
                path.is_file(),
                "the {language} resource for `{domain}` is missing ({})",
                path.display()
            );
        }
    }
}

/// Compare a translation against the reference: same keys, same variables.
fn check_agreement(
    reference: &BTreeMap<String, Message>,
    translated: &BTreeMap<String, Message>,
    language: &str,
) {
    let missing: Vec<&str> = reference
        .keys()
        .filter(|key| !translated.contains_key(*key))
        .map(String::as_str)
        .collect();
    let extra: Vec<&str> = translated
        .keys()
        .filter(|key| !reference.contains_key(*key))
        .map(String::as_str)
        .collect();

    // Both directions matter: a missing key serves English at runtime (wrong
    // but quiet), and an extra one is dead weight nobody will notice.
    assert!(
        missing.is_empty(),
        "{language} is missing {} key(s) that {REFERENCE} defines: {}\n\
         A missing translation falls back to English silently, so this would not \
         show up until someone read the {language} UI.",
        missing.len(),
        missing.join(", ")
    );
    assert!(
        extra.is_empty(),
        "{language} defines {} key(s) that {REFERENCE} does not: {}\n\
         Either the key was removed from {REFERENCE} and left behind, or it was \
         added only here and no code can reach it.",
        extra.len(),
        extra.join(", ")
    );

    let mut mismatched = Vec::new();
    for (key, reference_message) in reference {
        let Some(translated_message) = translated.get(key) else {
            continue;
        };
        if reference_message.variables != translated_message.variables {
            mismatched.push(format!(
                "{key}: {REFERENCE} uses {:?}, {language} uses {:?}",
                reference_message.variables, translated_message.variables
            ));
        }
    }
    assert!(
        mismatched.is_empty(),
        "{language} disagrees with {REFERENCE} about the values these messages take:\n  {}\n\
         A mismatch means one language formats a value the other has no slot for, \
         so the caller cannot pass one set of arguments to both.",
        mismatched.join("\n  ")
    );
}

/// Write the `TextKey` enum and a key-to-id table.
fn write_keys(reference: &BTreeMap<String, Message>) {
    let mut out = String::new();
    out.push_str(
        "// @generated by build.rs from locales/en-US/*.ftl — do not edit.\n\
         //\n\
         // One variant per message, so a key that does not exist is a compile\n\
         // error rather than a message that resolves to nothing at runtime.\n\n",
    );
    out.push_str(
        "/// Every message the application can show.\n\
         #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n\
         pub enum TextKey {\n",
    );
    for key in reference.keys() {
        let variant = to_variant(key);
        out.push_str(&format!("    {variant},\n"));
    }
    out.push_str("}\n\n");

    out.push_str("impl TextKey {\n");
    out.push_str("    /// Every key, for the coverage checks a caller may want to run.\n");
    out.push_str("    pub const ALL: &'static [TextKey] = &[\n");
    for key in reference.keys() {
        out.push_str(&format!("        TextKey::{},\n", to_variant(key)));
    }
    out.push_str("    ];\n\n");
    out.push_str("    /// The `$variables` this message takes, from the English resource.\n");
    out.push_str("    ///\n");
    out.push_str("    /// Emitted so a coverage check can supply a value per name without\n");
    out.push_str("    /// guessing which a message needs — a message taking two values\n");
    out.push_str("    /// cannot be satisfied one name at a time.\n");
    out.push_str("    pub fn variables(self) -> &'static [&'static str] {\n");
    out.push_str("        match self {\n");
    for (key, message) in reference.iter() {
        let names: Vec<String> = message
            .variables
            .iter()
            .map(|name| format!("{name:?}"))
            .collect();
        out.push_str(&format!(
            "            TextKey::{} => &[{}],\n",
            to_variant(key),
            names.join(", ")
        ));
    }
    out.push_str("        }\n    }\n\n");
    out.push_str("    /// The message id in the resource files.\n");
    out.push_str("    pub fn id(self) -> &'static str {\n");
    out.push_str("        match self {\n");
    for key in reference.keys() {
        out.push_str(&format!(
            "            TextKey::{} => {:?},\n",
            to_variant(key),
            key
        ));
    }
    out.push_str("        }\n    }\n}\n");

    let dest = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo"))
        .join("text_keys.rs");
    std::fs::write(&dest, out)
        .unwrap_or_else(|error| panic!("could not write {}: {error}", dest.display()));
}

/// `action-cancel` becomes `ActionCancel`.
///
/// Non-alphanumeric characters separate words, so a key uses whatever
/// punctuation reads best and the variant is always a valid identifier.
fn to_variant(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for word in key.split(|character: char| !character.is_alphanumeric()) {
        if word.is_empty() {
            continue;
        }
        let mut characters = word.chars();
        if let Some(first) = characters.next() {
            out.extend(first.to_uppercase());
            out.push_str(characters.as_str());
        }
    }
    assert!(
        !out.is_empty(),
        "the key `{key}` does not produce a variant name"
    );
    // A key that starts with a digit would not be a valid identifier.
    if out.starts_with(|character: char| character.is_ascii_digit()) {
        out.insert(0, 'K');
    }
    out
}
