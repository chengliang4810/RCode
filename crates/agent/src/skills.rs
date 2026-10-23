//! Skills: reusable instructions the model can pull in on demand.
//!
//! A skill is a directory containing `SKILL.md`, whose YAML frontmatter names
//! it and describes when it applies:
//!
//! ```text
//! ---
//! name: review
//! description: Review a diff for correctness and style.
//! ---
//! The body is the instruction the model reads.
//! ```
//!
//! Two roots, project first so a project can shadow a personal skill:
//!
//! - `{cwd}/.agents/skills`
//! - `{home}/.rcode/skills`
//!
//! The scan keeps the rules and drops the machinery: no MCP resource layer, no
//! digests, no activation protocol. What is kept is what makes a scan safe and
//! predictable — `SKILL.md` is the sole source of identity, the frontmatter
//! `name` is the identifier, symlinks are refused, and the walk is bounded.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// How deep below a root a skill directory may sit. A skill nested deeper is
/// usually a stray copy inside another project, not something the user meant to
/// install.
const MAX_DEPTH: usize = 4;
/// Directories visited per root. Bounds a root that turns out to be someone's
/// home directory.
const MAX_DIRS_PER_ROOT: usize = 2_000;
/// `SKILL.md` larger than this is not an instruction, and reading it would
/// cost more than it can be worth.
const MAX_SKILL_BYTES: u64 = 512 * 1024;

/// Where a skill came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillSource {
    /// `{cwd}/.agents/skills` — belongs to the project.
    Project,
    /// `{home}/.rcode/skills` — belongs to the person.
    User,
    /// A directory an installed plugin contributes.
    ///
    /// Read-only: the plugin owns those files, and a settings page that wrote
    /// into one would be editing a directory a re-install replaces.
    Plugin,
}

impl SkillSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::User => "user",
            Self::Plugin => "plugin",
        }
    }
}

/// The wire's name for a scope, and back.
///
/// Kept beside the definition so the harness cannot grow a second mapping:
/// the listing reports the scope a write used, and the write side resolves
/// that scope back to a root. The two directions must be exact inverses or a
/// skill created as `user` would come back listed as something else.
impl From<SkillSource> for rcode_proto::invocation::SkillScope {
    fn from(source: SkillSource) -> Self {
        match source {
            SkillSource::Project => Self::Project,
            SkillSource::User => Self::User,
            SkillSource::Plugin => Self::Plugin,
        }
    }
}

impl From<rcode_proto::invocation::SkillScope> for SkillSource {
    fn from(scope: rcode_proto::invocation::SkillScope) -> Self {
        match scope {
            rcode_proto::invocation::SkillScope::Project => Self::Project,
            rcode_proto::invocation::SkillScope::User => Self::User,
            rcode_proto::invocation::SkillScope::Plugin => Self::Plugin,
        }
    }
}

/// A skill found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// The frontmatter `name`, which is the identifier the model uses. Not the
    /// directory name — the two may differ, and the frontmatter is what the
    /// author declared.
    pub name: String,
    pub description: String,
    /// The directory holding `SKILL.md`.
    pub dir: PathBuf,
    pub source: SkillSource,
}

impl Skill {
    /// The path to read for the skill's body.
    pub fn manifest(&self) -> PathBuf {
        self.dir.join("SKILL.md")
    }
}

/// The directory a project keeps its skills in, relative to its root.
pub const PROJECT_SKILLS_DIR: &str = ".agents/skills";

/// The roots to scan, in precedence order.
///
/// Project before user so a project's skill wins a name collision: the project
/// is the more specific context, and a personal skill with the same name is
/// almost always the generic version of it.
///
/// [`skill_dir`] builds write targets from the same two shapes. A writer that
/// invented its own layout would create skills the scan never finds, which is
/// exactly the bug this pair exists to prevent.
pub fn roots(cwd: &Path, home: Option<&Path>) -> Vec<(PathBuf, SkillSource)> {
    let mut roots = vec![(cwd.join(PROJECT_SKILLS_DIR), SkillSource::Project)];
    if let Some(home) = home {
        roots.push((user_skills_dir(home), SkillSource::User));
    }
    roots
}

/// The user's own skills, under the data directory this build uses.
pub fn user_skills_dir(home: &Path) -> PathBuf {
    home.join(".rcode").join("skills")
}

/// The roots a scan reads, plus the plugin roots a caller supplies.
///
/// Kept separate from [`roots`] because a plugin root is not a *scope*: it is
/// never written to, and [`scope_root`] deliberately cannot name one.
pub fn all_roots(cwd: &Path, extra: &[PathBuf]) -> Vec<(PathBuf, SkillSource)> {
    let mut all = roots(cwd, home_dir().as_deref());
    all.extend(extra.iter().map(|root| (root.clone(), SkillSource::Plugin)));
    all
}

/// The one skills directory a scope writes to.
///
/// The write side of [`roots`] for a caller that has been *told* which scope
/// to touch (an RPC parameter, not a path): project resolves against the
/// working directory, user against the home. The root is derived here rather
/// than accepted from the caller, so no request can aim a write outside the
/// two directories this module manages.
///
/// `None` is the user scope with no readable home: there is nowhere a user
/// skill could live, and inventing a location would be worse than refusing.
pub fn scope_root(cwd: &Path, scope: SkillSource, home: Option<&Path>) -> Option<PathBuf> {
    match scope {
        SkillSource::Project => Some(project_skills_dir(cwd)),
        SkillSource::User => home.map(user_skills_dir),
        // A plugin's directory is not a scope a caller can address: it belongs
        // to the plugin, and a write would be undone by the next install.
        SkillSource::Plugin => None,
    }
}

/// Find every skill under one root, tagged `source`.
///
/// The scope-addressed half of [`discover`]: same scan, one root. A caller
/// that already knows which scope it wants (delete the user's copy, update the
/// project's) scans only that root, so a same-named skill in the other root —
/// the shadowing situation [`discover`] resolves by precedence — stays out of
/// the result instead of winning it.
pub fn discover_in(root: &Path, source: SkillSource) -> Vec<Skill> {
    scan_root(root, source)
}

/// A project's own skills directory, for a caller that has the project root.
///
/// The write side of [`roots`]: the pair means "where does a project keep its
/// skills" has one answer, so a created skill is one the scan finds.
pub fn project_skills_dir(cwd: &Path) -> PathBuf {
    cwd.join(PROJECT_SKILLS_DIR)
}

/// Where a skill of `name` lives, for the root a caller resolved.
///
/// `root` is the skills directory itself (`<project>/.agents/skills`,
/// `~/.rcode/skills`), not the project or the home — the caller gets it from
/// [`roots`] or from [`user_skills_dir`], so the scan and the writers cannot
/// disagree about the layout.
pub fn skill_dir(root: &Path, name: &str) -> Result<PathBuf, SkillError> {
    validate_name(name)?;
    Ok(root.join(name.trim()))
}

/// The user's home directory, from the environment.
///
/// `USERPROFILE` first on Windows (`HOME` is often unset for a GUI launch), and
/// the reverse elsewhere — the same ordering `rcode-harness` uses to find a
/// login shell.
pub fn home_dir() -> Option<PathBuf> {
    let primary = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let secondary = if cfg!(windows) { "HOME" } else { "USERPROFILE" };
    std::env::var_os(primary)
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os(secondary).filter(|value| !value.is_empty()))
        .map(PathBuf::from)
}

/// Find every skill under `cwd`, project roots first.
///
/// A missing root is not an error — most projects have no skills. A skill that
/// cannot be read is skipped with a log line rather than failing the scan:
/// one malformed `SKILL.md` should not hide the others.
pub fn discover(cwd: &Path) -> Vec<Skill> {
    discover_with(cwd, &[])
}

/// Every skill under `cwd` and `extra` roots, project first.
///
/// `extra` is where installed plugins contribute. They come last so a project's
/// own skill still wins a name collision — the project is the more specific
/// context, and a plugin is the most general thing in the list.
pub fn discover_with(cwd: &Path, extra: &[PathBuf]) -> Vec<Skill> {
    let home = home_dir();
    discover_roots(roots(cwd, home.as_deref()), extra)
}

/// Personal settings omit project roots before resolving name collisions.
pub fn discover_personal(home: Option<&Path>, extra: &[PathBuf]) -> Vec<Skill> {
    let roots = home
        .map(|home| vec![(user_skills_dir(home), SkillSource::User)])
        .unwrap_or_default();
    discover_roots(roots, extra)
}

fn discover_roots(mut all: Vec<(PathBuf, SkillSource)>, extra: &[PathBuf]) -> Vec<Skill> {
    let mut found: Vec<Skill> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    all.extend(extra.iter().map(|root| (root.clone(), SkillSource::Plugin)));
    for (root, source) in all {
        let namespace = if source == SkillSource::Plugin { plugin_namespace(&root) } else { None };
        for mut skill in scan_root(&root, source) {
            if let Some(namespace) = &namespace { skill.name = format!("{namespace}:{}", skill.name); }
            // First root wins a name collision, so a project skill shadows the
            // user's rather than appearing twice.
            if seen.insert(skill.name.clone()) {
                found.push(skill);
            } else {
                tracing::debug!(
                    skill = %skill.name,
                    root = %root.display(),
                    "a skill with this name was already found; ignoring this one"
                );
            }
        }
    }
    found
}

pub(crate) fn plugin_namespace(root: &Path) -> Option<String> {
    for ancestor in root.ancestors().take(8) {
        let path = ancestor.join(".claude-plugin/plugin.json");
        if let Ok(bytes) = std::fs::read(path) {
            let manifest: Value = serde_json::from_slice(&bytes).ok()?;
            return manifest.get("name")?.as_str().map(str::to_owned);
        }
    }
    None
}

/// Read a skill's body, without its frontmatter.
pub fn read_body(skill: &Skill) -> std::io::Result<String> {
    let text = std::fs::read_to_string(skill.manifest())?;
    let mut body = split_frontmatter(&text).map(|(_, body)| body.to_owned()).unwrap_or(text);
    body = body.replace("${CLAUDE_SKILL_DIR}", &skill.dir.to_string_lossy().replace('\\', "/"));
    if skill.source == SkillSource::Plugin {
        if let Some(root) = skill.dir.ancestors().find(|root| root.join(".claude-plugin/plugin.json").is_file()) {
            body = body.replace("${CLAUDE_PLUGIN_ROOT}", &root.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(body)
}

fn scan_root(root: &Path, source: SkillSource) -> Vec<Skill> {
    if !root.is_dir() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut visited = 0usize;
    walk(root, root, source, 0, &mut visited, &mut found);
    found
}

fn walk(
    root: &Path,
    dir: &Path,
    source: SkillSource,
    depth: usize,
    visited: &mut usize,
    found: &mut Vec<Skill>,
) {
    if depth > MAX_DEPTH || *visited >= MAX_DIRS_PER_ROOT {
        return;
    }
    *visited += 1;

    // A directory holding a readable `SKILL.md` is a skill, and the walk does
    // not descend further: a skill's own subdirectories are its attachments,
    // not more skills.
    if let Some(skill) = read_skill(dir, root, source) {
        found.push(skill);
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        // Symlinks are refused outright: a link can point outside the root, and
        // a scan that follows them can walk into anywhere on the machine.
        .filter(|entry| {
            !entry
                .file_type()
                .map(|kind| kind.is_symlink())
                .unwrap_or(true)
        })
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    children.sort();
    for child in children {
        walk(root, &child, source, depth + 1, visited, found);
    }
}

fn read_skill(dir: &Path, root: &Path, source: SkillSource) -> Option<Skill> {
    let manifest = dir.join("SKILL.md");
    let metadata = std::fs::symlink_metadata(&manifest).ok()?;
    if metadata.file_type().is_symlink() || metadata.len() > MAX_SKILL_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(&manifest).ok()?;
    let (name, description) = summary(&text).or_else(|| {
        if source != SkillSource::Plugin { return None; }
        let metadata = if let Some((frontmatter, _)) = split_frontmatter(&text) {
            yaml_serde::from_str::<yaml_serde::Value>(frontmatter).ok()?
        } else { yaml_serde::Value::Null };
        let name = metadata.get("name").and_then(|value| value.as_str()).unwrap_or(dir.file_name()?.to_str()?).to_owned();
        if name.is_empty() || name.contains(['/', '\\']) || name.chars().any(char::is_whitespace) { return None; }
        let description = metadata.get("description").and_then(|value| value.as_str()).unwrap_or_default().to_owned();
        Some((name, description))
    })?;
    let _ = root;
    Some(Skill {
        name,
        description,
        dir: dir.to_path_buf(),
        source,
    })
}

/// `(name, description)` from the frontmatter. Both are required; a skill
/// without them has nothing to tell the model about when it applies.
fn summary(text: &str) -> Option<(String, String)> {
    let (frontmatter, _) = split_frontmatter(text)?;
    let parsed: yaml_serde::Value = yaml_serde::from_str(frontmatter).ok()?;
    let map = parsed.as_mapping()?;

    let get = |key: &str| -> Option<String> {
        map.get(yaml_serde::Value::String(key.to_owned()))
            .and_then(|value| value.as_str())
            .map(str::to_owned)
    };
    let name = get("name")?;
    if name.trim().is_empty() {
        return None;
    }
    // A skill name is an identifier the model writes; a path separator or a
    // space in it would make the name unusable wherever it is passed back.
    if name.contains('/') || name.contains('\\') || name.chars().any(char::is_whitespace) {
        tracing::warn!(skill = %name, "a skill name must not contain whitespace or path separators; skipping");
        return None;
    }
    let description = get("description")?;
    Some((name, description.trim().to_owned()))
}

/// Split a `---`-delimited frontmatter block from its body.
///
/// A leading BOM is ignored (editors add one); a first line that is not `---`
/// means there is no frontmatter; an unclosed block means the same. The
/// separator is the exact three-dash form — `+++` and other conventions are not
/// recognised, matching the original project.
pub fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut lines = content.split_inclusive('\n');
    let first = lines.next()?;
    if first.trim_end() != "---" {
        return None;
    }
    let start = first.len();
    let mut cursor = start;
    for line in lines {
        if line.trim_end() == "---" {
            return Some((&content[start..cursor], &content[cursor + line.len()..]));
        }
        cursor += line.len();
    }
    None
}

/// The frontmatter as JSON, for a caller that wants more than the summary.
pub fn frontmatter_json(content: &str) -> Option<serde_json::Map<String, Value>> {
    let (frontmatter, _) = split_frontmatter(content)?;
    let parsed: yaml_serde::Value = yaml_serde::from_str(frontmatter).ok()?;
    let json = serde_json::to_value(yaml_to_json(parsed)).ok()?;
    match json {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// YAML values as JSON values.
///
/// Serde renders YAML mappings with non-string keys as *sequences*, so a plain
/// `serde_json::to_value` mangles them. Skills use string keys only, but a
/// malformed file should degrade to "no frontmatter" rather than to a JSON
/// array the caller then has to defend against.
fn yaml_to_json(value: yaml_serde::Value) -> Value {
    match value {
        yaml_serde::Value::Null => Value::Null,
        yaml_serde::Value::Bool(value) => Value::Bool(value),
        yaml_serde::Value::Number(value) => serde_json::to_value(value).unwrap_or(Value::Null),
        yaml_serde::Value::String(value) => Value::String(value),
        yaml_serde::Value::Sequence(items) => {
            Value::Array(items.into_iter().map(yaml_to_json).collect())
        }
        yaml_serde::Value::Mapping(map) => {
            let mut object = serde_json::Map::new();
            for (key, value) in map {
                let key = match key {
                    yaml_serde::Value::String(key) => key,
                    other => format!("{other:?}"),
                };
                object.insert(key, yaml_to_json(value));
            }
            Value::Object(object)
        }
        yaml_serde::Value::Tagged(tagged) => yaml_to_json(tagged.value),
    }
}

// ─── Creating, deleting and enabling ────────────────────────────────────────
//
// The scan above is read-only, and the model's view of a project should not
// depend on what the settings UI happens to be doing. These are the mutations,
// kept in the same module because they must agree with the scan about what a
// skill *is*: a directory with a `SKILL.md` whose frontmatter names it.

/// Why a skill could not be created or deleted.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("a skill needs a name of letters, digits, dashes or underscores")]
    BadName,
    #[error("a skill named `{0}` already exists here")]
    Exists(String),
    #[error("no skill named `{0}` was found to change")]
    NotFound(String),
    #[error("only a skill this application can write may be changed: {0}")]
    NotWritable(String),
    #[error("{0}")]
    Io(String),
    #[error("the skill file is not valid: {0}")]
    Malformed(String),
}

impl From<std::io::Error> for SkillError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// A skill name as a directory name.
///
/// Letters, digits, dash and underscore — the same set the frontmatter names
/// use, and deliberately narrow: the name becomes a path segment, and anything
/// a filesystem treats specially there is a way to write outside the root.
pub fn validate_name(name: &str) -> Result<(), SkillError> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || !trimmed
            .chars()
            .all(|character| character.is_alphanumeric() || character == '-' || character == '_')
    {
        return Err(SkillError::BadName);
    }
    // A name that is only dots is a traversal, not a name.
    if trimmed.chars().all(|character| character == '.') {
        return Err(SkillError::BadName);
    }
    Ok(())
}

/// Create a skill, returning the file it wrote.
///
/// The body is the instruction the model reads; `description` is what the model
/// sees when deciding whether to load it, so both are required — a skill with
/// no description is one the model can never find a reason to open.
///
/// Fails rather than overwriting when the name is taken: silently replacing a
/// skill the user wrote is the kind of loss that has no undo.
pub fn create(
    root: &Path,
    name: &str,
    description: &str,
    body: &str,
) -> Result<PathBuf, SkillError> {
    let dir = skill_dir(root, name)?;
    if dir.exists() {
        return Err(SkillError::Exists(name.trim().to_owned()));
    }
    if description.trim().is_empty() {
        return Err(SkillError::Malformed(
            "a description is required: it is what the model reads when deciding whether to use \
             this skill"
                .to_owned(),
        ));
    }

    std::fs::create_dir_all(&dir)?;
    let manifest = dir.join("SKILL.md");
    // The frontmatter is written by hand rather than through a serializer: the
    // file is something the user will open and edit, and a minimal, predictable
    // shape is worth more there than the generality of a YAML writer.
    let contents = format!(
        "---\nname: {}\ndescription: {}\n---\n\n{}\n",
        name.trim(),
        description.trim(),
        body.trim()
    );
    std::fs::write(&manifest, contents)?;
    Ok(manifest)
}

/// The skills directory a scanned skill belongs to.
///
/// The scan returns the skill; this says which of the two roots it came from,
/// so a caller that wants to write or delete can name the same one. Home is
/// read the same way the scan reads it, and resolution goes through
/// [`scope_root`], so the scope-addressed writers and this cannot disagree.
pub fn root_for(skill: &Skill, cwd: &Path) -> PathBuf {
    scope_root(cwd, skill.source, home_dir().as_deref())
        // A home that cannot be read cannot have produced a user skill; the
        // project root is the harmless answer and the containment check in
        // `delete` will refuse if it is wrong.
        .unwrap_or_else(|| project_skills_dir(cwd))
}

/// Delete a skill, if it lives under a directory this application manages.
///
/// A skill under a root we manage is ours to remove. One somewhere else — a
/// project's own tree the user assembled, or a path a caller passed in — is
/// refused: the scan will happily *read* it, but deleting a directory we did
/// not create is not a settings-page action.
///
/// `root` is the **skills directory** ([`project_skills_dir`] or
/// [`user_skills_dir`]), and the check is containment rather than an exact
/// name match: a skill's directory keeps the name it was created with, so a
/// renamed one would otherwise be undeletable.
pub fn delete(skill: &Skill, root: &Path) -> Result<(), SkillError> {
    let root = std::fs::canonicalize(root)
        .map_err(|_| SkillError::NotWritable(format!("`{}` no longer exists", root.display())))?;
    let actual = std::fs::canonicalize(&skill.dir).map_err(|_| {
        SkillError::NotWritable(format!("`{}` no longer exists", skill.dir.display()))
    })?;
    if actual == root || !actual.starts_with(&root) {
        return Err(SkillError::NotWritable(format!(
            "`{}` is not under the skills directory this application manages",
            skill.dir.display()
        )));
    }
    std::fs::remove_dir_all(&actual)?;
    Ok(())
}

/// Rewrite a skill's frontmatter, and optionally its body, in place.
///
/// Used to rename or re-describe a skill, or to replace its instruction. A
/// `body` of `None` keeps the instruction the file already carries — the
/// common edit, and the reason the old body is parsed out rather than
/// regenerated. A file whose frontmatter cannot be split is refused rather
/// than rewritten with `Some` body: caller-supplied text would silently
/// replace an instruction we never read. With `None` the refusal is the same,
/// because there is no body to keep.
pub fn update_metadata(
    skill: &Skill,
    name: &str,
    description: &str,
    body: Option<&str>,
) -> Result<(), SkillError> {
    validate_name(name)?;
    if description.trim().is_empty() {
        return Err(SkillError::Malformed(
            "a description is required".to_owned(),
        ));
    }
    let text = std::fs::read_to_string(skill.manifest())?;
    let (_, current) = split_frontmatter(&text).ok_or_else(|| {
        SkillError::Malformed(format!(
            "`{}` has no frontmatter block to rewrite",
            skill.manifest().display()
        ))
    })?;
    let body = body.map(str::trim).unwrap_or(current);
    let contents = format!(
        "---\nname: {}\ndescription: {}\n---\n{body}",
        name.trim(),
        description.trim()
    );
    std::fs::write(skill.manifest(), contents)?;
    Ok(())
}

/// Which skills are switched off, keyed by their directory path.
///
/// Off-by-path rather than by name: two roots can hold a skill of the same name
/// (one shadowing the other), and switching one off must not switch off the
/// other.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SkillEnableMap {
    disabled: std::collections::BTreeSet<String>,
}

impl SkillEnableMap {
    /// Whether a skill at `dir` is enabled.
    pub fn is_enabled(&self, dir: &Path) -> bool {
        !self.disabled.contains(&key(dir))
    }

    /// Switch a skill on or off, returning whether anything changed.
    pub fn set(&mut self, dir: &Path, enabled: bool) -> bool {
        let key = key(dir);
        if enabled {
            self.disabled.remove(&key)
        } else {
            self.disabled.insert(key)
        }
    }

    /// How many skills are switched off.
    pub fn disabled_count(&self) -> usize {
        self.disabled.len()
    }

    /// Drop entries for directories that no longer exist, so the file does not
    /// grow without bound as skills come and go.
    pub fn prune(&mut self) -> usize {
        let before = self.disabled.len();
        self.disabled.retain(|path| Path::new(path).exists());
        before - self.disabled.len()
    }
}

/// The key a skill is remembered under.
///
/// Canonical when the directory exists, so the same skill reached by two
/// spellings is one entry. A path that cannot be canonicalised falls back to
/// its own text rather than being dropped.
fn key(dir: &Path) -> String {
    std::fs::canonicalize(dir)
        .unwrap_or_else(|_| dir.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(root: &Path, name: &str, frontmatter_name: &str, description: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!(
                "---\nname: {frontmatter_name}\ndescription: {description}\n---\n\nThe body.\n"
            ),
        )
        .unwrap();
        dir
    }

    #[test]
    fn frontmatter_splits_from_the_body() {
        let text = "---\nname: a\ndescription: b\n---\nBody here";
        let (frontmatter, body) = split_frontmatter(text).expect("splits");
        assert!(frontmatter.contains("name: a"));
        assert_eq!(body, "Body here");
    }

    #[test]
    fn a_leading_bom_is_ignored() {
        let text = "\u{feff}---\nname: a\ndescription: b\n---\nBody";
        assert!(split_frontmatter(text).is_some());
    }

    #[test]
    fn no_frontmatter_yields_none() {
        assert!(split_frontmatter("# Just a heading\n").is_none());
        assert!(split_frontmatter("---\nname: a\n").is_none(), "unclosed");
    }

    #[test]
    fn plus_fences_are_not_frontmatter() {
        // The original project recognises only `---`; `+++` is treated as no frontmatter.
        assert!(split_frontmatter("+++\nname = \"a\"\n+++\n").is_none());
    }

    #[test]
    fn a_skill_is_found_at_the_root() {
        let root = tempfile::tempdir().unwrap();
        write_skill(root.path(), "review", "review", "Review a diff.");

        let found = scan_root(root.path(), SkillSource::Project);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "review");
        assert_eq!(found[0].description, "Review a diff.");
    }

    #[test]
    fn a_skill_is_found_below_the_root() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("team").join("backend");
        std::fs::create_dir_all(&nested).unwrap();
        write_skill(&nested, "deploy", "deploy", "Deploy the service.");

        let found = scan_root(root.path(), SkillSource::Project);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "deploy");
    }

    #[test]
    fn the_walk_does_not_descend_into_a_skill() {
        let root = tempfile::tempdir().unwrap();
        let dir = write_skill(root.path(), "outer", "outer", "Outer.");
        // A skill-looking directory inside a skill is an attachment, not a
        // second skill.
        write_skill(&dir, "inner", "inner", "Inner.");

        let found = scan_root(root.path(), SkillSource::Project);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "outer");
    }

    #[test]
    fn the_frontmatter_name_wins_over_the_directory_name() {
        let root = tempfile::tempdir().unwrap();
        write_skill(root.path(), "directory-name", "actual-name", "Desc.");

        let found = scan_root(root.path(), SkillSource::Project);
        assert_eq!(found[0].name, "actual-name");
    }

    #[test]
    fn a_skill_without_a_description_is_skipped() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("bare");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: bare\n---\nBody").unwrap();

        // Nothing to tell the model about when it applies.
        assert!(scan_root(root.path(), SkillSource::Project).is_empty());
    }

    #[test]
    fn a_name_with_whitespace_is_skipped() {
        let root = tempfile::tempdir().unwrap();
        write_skill(root.path(), "dir", "two words", "Desc.");
        assert!(scan_root(root.path(), SkillSource::Project).is_empty());
    }

    #[test]
    fn a_name_with_a_path_separator_is_skipped() {
        let root = tempfile::tempdir().unwrap();
        write_skill(root.path(), "dir", "a/b", "Desc.");
        assert!(scan_root(root.path(), SkillSource::Project).is_empty());
    }

    #[test]
    fn a_directory_without_a_manifest_is_not_a_skill() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("not-a-skill")).unwrap();
        assert!(scan_root(root.path(), SkillSource::Project).is_empty());
    }

    #[test]
    fn a_missing_root_is_not_an_error() {
        let missing = std::env::temp_dir().join("rcode-no-such-skills-root");
        assert!(scan_root(&missing, SkillSource::Project).is_empty());
    }

    #[test]
    fn the_body_reads_without_its_frontmatter() {
        let root = tempfile::tempdir().unwrap();
        let dir = write_skill(root.path(), "review", "review", "Review a diff.");
        let found = scan_root(root.path(), SkillSource::Project);

        let body = read_body(&found[0]).expect("reads");
        assert_eq!(body.trim(), "The body.");
        assert!(!body.contains("name:"));
        assert_eq!(found[0].dir, dir);
    }

    #[test]
    fn project_roots_come_before_user_roots() {
        let cwd = PathBuf::from("/work/project");
        let home = PathBuf::from("/home/someone");
        let roots = roots(&cwd, Some(&home));

        assert_eq!(roots[0].1, SkillSource::Project);
        assert!(roots[0].0.ends_with(".agents/skills"));
        assert_eq!(roots[1].1, SkillSource::User);
        assert!(roots[1].0.ends_with(".rcode/skills"));
    }

    #[test]
    fn a_project_skill_shadows_a_user_skill_of_the_same_name() {
        let project = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        let project_root = project.path().join(".agents").join("skills");
        let user_root = user.path().join(".rcode").join("skills");
        write_skill(&project_root, "x", "shared", "The project's version.");
        write_skill(&user_root, "x", "shared", "The user's version.");

        // The same precedence `discover` applies: project roots first, so the
        // first occurrence of a name wins.
        let mut seen = std::collections::HashSet::new();
        let mut found = Vec::new();
        for (root, source) in [
            (project_root, SkillSource::Project),
            (user_root, SkillSource::User),
        ] {
            for skill in scan_root(&root, source) {
                if seen.insert(skill.name.clone()) {
                    found.push(skill);
                }
            }
        }

        let shared: Vec<&Skill> = found.iter().filter(|s| s.name == "shared").collect();
        assert_eq!(shared.len(), 1, "the name should appear once");
        assert_eq!(shared[0].source, SkillSource::Project);
        assert_eq!(shared[0].description, "The project's version.");
    }

    #[test]
    fn frontmatter_json_renders_the_scalars() {
        let text = "---\nname: a\ndescription: b\nversion: 2\nflag: true\n---\nBody";
        let json = frontmatter_json(text).expect("parses");
        assert_eq!(json.get("name").and_then(Value::as_str), Some("a"));
        assert_eq!(json.get("version").and_then(Value::as_u64), Some(2));
        assert_eq!(json.get("flag").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn frontmatter_preserves_yaml_aliases_tags_and_scalar_types() {
        let text = r#"---
name: &name !skill yaml-example
description: *name
plain: [yes, on, 012, 0b11, 0o10, 0x10, true, null, .inf, .nan]
quoted: ['true', "123"]
tagged: !metadata {nested: !items [one, two]}
large: 18446744073709551615
? [x, y]
: complex-key
---
Body
"#;
        assert_eq!(
            summary(text),
            Some(("yaml-example".into(), "yaml-example".into()))
        );
        let json = frontmatter_json(text).expect("valid YAML frontmatter");
        assert_eq!(
            json["plain"],
            serde_json::json!(["yes", "on", "012", 3, 8, 16, true, null, null, null])
        );
        assert_eq!(json["quoted"], serde_json::json!(["true", "123"]));
        assert_eq!(json["tagged"], serde_json::json!({"nested": ["one", "two"]}));
        assert_eq!(json["large"].as_u64(), Some(u64::MAX));
        assert_eq!(
            json["Sequence [String(\"x\"), String(\"y\")]"],
            "complex-key"
        );
        assert!(summary("---\nname: 123\ndescription: text\n---\nBody").is_none());
        assert!(summary("---\nname: example\ndescription: true\n---\nBody").is_none());
        assert!(frontmatter_json("---\nname: one\nname: two\n---\nBody").is_none());
    }

    #[test]
    fn scopes_round_trip_through_the_wire_names() {
        for (source, scope) in [
            (
                SkillSource::Project,
                rcode_proto::invocation::SkillScope::Project,
            ),
            (SkillSource::User, rcode_proto::invocation::SkillScope::User),
        ] {
            assert_eq!(rcode_proto::invocation::SkillScope::from(source), scope);
            assert_eq!(SkillSource::from(scope), source);
        }
        assert_eq!(
            serde_json::from_str::<rcode_proto::invocation::SkillScope>("\"project\"").unwrap(),
            rcode_proto::invocation::SkillScope::Project
        );
        assert_eq!(
            serde_json::from_str::<rcode_proto::invocation::SkillScope>("\"user\"").unwrap(),
            rcode_proto::invocation::SkillScope::User
        );
    }

    #[test]
    fn scope_roots_are_the_two_managed_directories() {
        let cwd = PathBuf::from("/work/project");
        let home = PathBuf::from("/home/someone");

        let project = scope_root(&cwd, SkillSource::Project, Some(&home)).expect("project");
        assert_eq!(project, project_skills_dir(&cwd));
        assert!(project.ends_with(".agents/skills"));

        let user = scope_root(&cwd, SkillSource::User, Some(&home)).expect("user");
        assert_eq!(user, user_skills_dir(&home));
        assert!(user.ends_with(".rcode/skills"));

        // A user scope with no home has nowhere to go, and the caller must
        // hear that rather than be handed a project path by surprise.
        assert!(scope_root(&cwd, SkillSource::User, None).is_none());
        assert!(scope_root(&cwd, SkillSource::Project, None).is_some());
    }

    #[test]
    fn a_scoped_scan_sees_only_its_own_root() {
        let project = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        let project_root = project.path().join(".agents").join("skills");
        let user_root = user.path().join(".rcode").join("skills");
        write_skill(&project_root, "shared", "shared", "The project's version.");
        write_skill(
            &project_root,
            "only-project",
            "only-project",
            "Project only.",
        );
        write_skill(&user_root, "shared", "shared", "The user's version.");
        write_skill(&user_root, "only-user", "only-user", "User only.");

        let project_skills = discover_in(&project_root, SkillSource::Project);
        let user_skills = discover_in(&user_root, SkillSource::User);
        fn names(found: &[Skill]) -> Vec<&str> {
            let mut names: Vec<_> = found.iter().map(|s| s.name.as_str()).collect();
            names.sort_unstable();
            names
        }

        assert_eq!(names(&project_skills), ["only-project", "shared"]);
        assert!(
            project_skills
                .iter()
                .all(|s| s.source == SkillSource::Project)
        );
        assert_eq!(names(&user_skills), ["only-user", "shared"]);
        assert!(user_skills.iter().all(|s| s.source == SkillSource::User));
        // Same name, both scopes: each root's copy is addressable in its own
        // scan, which is what a scope-addressed delete/update needs.
        assert_eq!(
            project_skills
                .iter()
                .find(|s| s.name == "shared")
                .unwrap()
                .description,
            "The project's version."
        );
        assert_eq!(
            user_skills
                .iter()
                .find(|s| s.name == "shared")
                .unwrap()
                .description,
            "The user's version."
        );
    }
}

// ─── CRUD tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod crud_tests {
    use super::*;

    fn root() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    /// A skill directory the way `discover` finds one.
    fn existing(root: &Path, name: &str, description: &str) -> Skill {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\nDo the thing.\n"),
        )
        .expect("write");
        Skill {
            name: name.to_owned(),
            description: description.to_owned(),
            dir,
            source: SkillSource::User,
        }
    }

    #[test]
    fn a_name_must_be_a_single_safe_segment() {
        assert!(validate_name("review").is_ok());
        assert!(validate_name("code-review_2").is_ok());
        assert!(validate_name("修复登录").is_ok(), "unicode names are fine");

        // Anything that is not a plain name is refused: it becomes a path
        // segment, and a separator would let it escape the root.
        for bad in [
            "",
            "  ",
            "../escape",
            "a/b",
            "a\\b",
            "a b",
            ".",
            "..",
            "a.b",
        ] {
            assert!(
                validate_name(bad).is_err(),
                "`{bad}` should have been refused"
            );
        }
    }

    #[test]
    fn a_name_cannot_traverse_out_of_the_root() {
        // The whole point of validating: the name is joined to a root.
        let dir = root();
        for bad in ["../outside", "a/../../b", ".."] {
            let result = skill_dir(dir.path(), bad);
            assert!(
                result.is_err(),
                "`{bad}` produced {}",
                result.map(|p| p.display().to_string()).unwrap_or_default()
            );
        }
    }

    #[test]
    fn creating_writes_a_file_discover_can_read() {
        let dir = root();
        let manifest =
            create(dir.path(), "review", "Review a diff.", "Check the diff.").expect("create");

        assert!(manifest.exists());
        // The round trip that matters: what we wrote, the scan must find.
        let found = scan_root(dir.path(), SkillSource::User);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "review");
        assert_eq!(found[0].description, "Review a diff.");
        assert!(
            read_body(&found[0])
                .expect("body")
                .contains("Check the diff.")
        );
    }

    #[test]
    fn creating_twice_is_refused_rather_than_overwriting() {
        // Silently replacing a skill the user wrote is a loss with no undo.
        let dir = root();
        create(dir.path(), "review", "First.", "Body one.").expect("first");
        let error = create(dir.path(), "review", "Second.", "Body two.")
            .expect_err("the second must be refused");
        assert!(matches!(error, SkillError::Exists(name) if name == "review"));

        // And the original is intact.
        let found = scan_root(dir.path(), SkillSource::User);
        assert!(read_body(&found[0]).expect("body").contains("Body one."));
    }

    #[test]
    fn a_description_is_required() {
        // Without one the model has no reason to ever open the skill.
        let dir = root();
        assert!(matches!(
            create(dir.path(), "review", "   ", "Body.").expect_err("refused"),
            SkillError::Malformed(_)
        ));
    }

    /// A created project skill must be one the project scan finds.
    ///
    /// The two halves are separate functions (`project_skills_dir` to write,
    /// `roots` to read) and they must describe one layout: a skill written
    /// beside the project instead of under `.agents/skills` is invisible, and
    /// nothing else would report it.
    #[test]
    fn a_created_project_skill_is_one_the_scan_finds() {
        let project = tempfile::tempdir().expect("project");
        let skills = project_skills_dir(project.path());
        std::fs::create_dir_all(&skills).expect("mkdir");
        create(&skills, "review", "Review a diff.", "Check the diff.").expect("create");

        let found = discover(project.path());
        assert_eq!(found.len(), 1, "the scan must find it: {found:#?}");
        assert_eq!(found[0].name, "review");
        assert_eq!(found[0].source, SkillSource::Project);

        // And the root a caller needs to delete it is the one it was written to.
        assert_eq!(root_for(&found[0], project.path()), skills);
        delete(&found[0], &root_for(&found[0], project.path())).expect("delete");
        assert!(discover(project.path()).is_empty());
    }

    #[test]
    fn deleting_removes_only_a_skill_under_the_root() {
        let dir = root();
        create(dir.path(), "review", "Review.", "Body.").expect("create");
        let found = scan_root(dir.path(), SkillSource::User);
        assert_eq!(found.len(), 1);

        delete(&found[0], dir.path()).expect("delete");
        assert!(scan_root(dir.path(), SkillSource::User).is_empty());
    }

    #[test]
    fn deleting_refuses_a_skill_that_lives_elsewhere() {
        // The scan reads any root; the delete must not follow it there.
        let managed = root();
        let elsewhere = root();
        let skill = existing(elsewhere.path(), "review", "Review.");

        let error = delete(&skill, managed.path()).expect_err("refused");
        assert!(matches!(error, SkillError::NotWritable(_)));
        assert!(
            skill.dir.exists(),
            "a skill outside the managed root was deleted"
        );
    }

    #[test]
    fn deleting_a_missing_skill_reports_rather_than_panicking() {
        let dir = root();
        let skill = Skill {
            name: "gone".to_owned(),
            description: String::new(),
            dir: dir.path().join("gone"),
            source: SkillSource::User,
        };
        assert!(delete(&skill, dir.path()).is_err());
    }

    #[test]
    fn updating_metadata_keeps_the_body() {
        // The body is the instruction; only the framing changes.
        let dir = root();
        create(
            dir.path(),
            "review",
            "Old description.",
            "The precious instruction.",
        )
        .expect("create");
        let skill = scan_root(dir.path(), SkillSource::User).remove(0);

        update_metadata(&skill, "review", "New description.", None).expect("update");

        let found = scan_root(dir.path(), SkillSource::User);
        assert_eq!(found[0].description, "New description.");
        assert!(
            read_body(&found[0])
                .expect("body")
                .contains("The precious instruction."),
            "the body was lost"
        );
    }

    #[test]
    fn updating_metadata_can_replace_the_body() {
        let dir = root();
        create(
            dir.path(),
            "review",
            "Review a diff.",
            "The old instruction.",
        )
        .expect("create");
        let skill = scan_root(dir.path(), SkillSource::User).remove(0);

        update_metadata(
            &skill,
            "review",
            "Review a diff.",
            Some("The new instruction."),
        )
        .expect("update");

        let found = scan_root(dir.path(), SkillSource::User);
        assert_eq!(found[0].description, "Review a diff.");
        let body = read_body(&found[0]).expect("body");
        assert!(body.contains("The new instruction."), "{body}");
        assert!(!body.contains("The old instruction."), "{body}");
    }

    #[test]
    fn updating_a_file_without_frontmatter_is_refused() {
        // We cannot reconstruct a body we never parsed, so rewriting would
        // destroy the instruction.
        let dir = root();
        let skill_dir = dir.path().join("bare");
        std::fs::create_dir_all(&skill_dir).expect("mkdir");
        std::fs::write(skill_dir.join("SKILL.md"), "no frontmatter here").expect("write");
        let skill = Skill {
            name: "bare".to_owned(),
            description: String::new(),
            dir: skill_dir,
            source: SkillSource::User,
        };

        assert!(matches!(
            update_metadata(&skill, "bare", "d", None).expect_err("refused"),
            SkillError::Malformed(_)
        ));
    }

    // === Enable map ===

    #[test]
    fn skills_are_enabled_until_switched_off() {
        let dir = root();
        let skill = existing(dir.path(), "review", "Review.");
        let mut map = SkillEnableMap::default();

        assert!(map.is_enabled(&skill.dir));
        assert!(map.set(&skill.dir, false));
        assert!(!map.is_enabled(&skill.dir));
        assert_eq!(map.disabled_count(), 1);
    }

    #[test]
    fn switching_off_twice_reports_no_change() {
        // So a toggle that writes state can avoid a pointless save.
        let dir = root();
        let skill = existing(dir.path(), "review", "Review.");
        let mut map = SkillEnableMap::default();

        assert!(map.set(&skill.dir, false));
        assert!(!map.set(&skill.dir, false), "the second switch is a no-op");
        assert!(map.set(&skill.dir, true));
        assert!(!map.set(&skill.dir, true));
    }

    #[test]
    fn switching_off_is_per_directory_not_per_name() {
        // Two roots can hold a skill of the same name; one shadowing the other
        // is not the same skill.
        let first = root();
        let second = root();
        let a = existing(first.path(), "review", "Review.");
        let b = existing(second.path(), "review", "Review.");
        let mut map = SkillEnableMap::default();

        map.set(&a.dir, false);
        assert!(!map.is_enabled(&a.dir));
        assert!(
            map.is_enabled(&b.dir),
            "the other root's skill is unaffected"
        );
    }

    #[test]
    fn the_enable_map_round_trips_through_json() {
        let dir = root();
        let skill = existing(dir.path(), "review", "Review.");
        let mut map = SkillEnableMap::default();
        map.set(&skill.dir, false);

        let json = serde_json::to_string(&map).expect("serialize");
        let parsed: SkillEnableMap = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, map);
        assert!(!parsed.is_enabled(&skill.dir));
    }

    #[test]
    fn pruning_drops_entries_for_vanished_skills() {
        let dir = root();
        let skill = existing(dir.path(), "review", "Review.");
        let mut map = SkillEnableMap::default();
        map.set(&skill.dir, false);
        assert_eq!(map.disabled_count(), 1);

        std::fs::remove_dir_all(&skill.dir).expect("remove");
        assert_eq!(map.prune(), 1, "the stale entry should be dropped");
        assert_eq!(map.disabled_count(), 0);
    }
}

#[cfg(test)]
mod marketplace_compat_tests {
    use super::*;
    #[test]
    fn plugin_skill_names_can_be_inferred_and_resource_variables_resolve_without_editing_files() {
        let temp = tempfile::tempdir().unwrap();
        let plugin = temp.path().join("kit");
        let skills = plugin.join("skills");
        let dir = skills.join("review");
        std::fs::create_dir_all(plugin.join(".claude-plugin")).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(plugin.join(".claude-plugin/plugin.json"), r#"{"name":"kit"}"#).unwrap();
        let original = "---\ndescription: Review code\n---\nRead ${CLAUDE_SKILL_DIR}/guide.md and ${CLAUDE_PLUGIN_ROOT}/data.json\n";
        std::fs::write(dir.join("SKILL.md"), original).unwrap();
        let found = discover_with(temp.path(), &[skills]);
        let skill = found.iter().find(|skill| skill.name == "kit:review").unwrap();
        let body = read_body(skill).unwrap();
        assert!(body.contains(&format!("{}/guide.md", dir.to_string_lossy().replace('\\', "/"))));
        assert!(body.contains(&format!("{}/data.json", plugin.to_string_lossy().replace('\\', "/"))));
        assert_eq!(std::fs::read_to_string(dir.join("SKILL.md")).unwrap(), original);
    }
}
