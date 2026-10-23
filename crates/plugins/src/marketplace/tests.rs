use super::*;
use std::io::Write;

fn entry(value: Value) -> Entry {
    serde_json::from_value(value).unwrap()
}
fn archive(path: &Path, files: &[(&str, &str)]) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, content) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn resolves_all_builtin_markets_and_source_shapes() {
    assert_eq!(SOURCES.len(), 3);
    for market in &SOURCES {
        let e = entry(json!({"name":"kit", "source":"./"}));
        assert_eq!(
            source(&e, market).unwrap(),
            Source {
                repo: market.repository.into(),
                revision: "main".into(),
                path: PathBuf::new()
            }
        );
    }
    let e = entry(
        json!({"name":"kit", "source":{"source":"git-subdir", "url":"https://github.com/owner/repo.git", "path":"plugins/kit", "ref":"main", "sha":"abc123"}}),
    );
    assert_eq!(
        source(&e, &SOURCES[0]).unwrap(),
        Source {
            repo: "owner/repo".into(),
            revision: "abc123".into(),
            path: PathBuf::from("plugins/kit")
        }
    );
    assert!(market_source("custom-url").is_err());
    for path in [
        "../outside",
        "/absolute",
        "D:/outside",
        "..\\outside",
        "skills/../../outside",
        ".. /outside",
        "folder./outside",
    ] {
        assert!(relative_path(path).is_err(), "accepted {path}");
    }
}

#[tokio::test]
async fn immutable_revisions_are_preserved_and_commit_responses_are_validated() {
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let source = Source {
        repo: "owner/repo".into(),
        revision: sha.into(),
        path: PathBuf::new(),
    };
    // An already pinned commit needs no mutable-ref API lookup.
    assert_eq!(
        resolve_revision(&rcode_github::client().unwrap(), &source)
            .await
            .unwrap(),
        sha
    );
    assert_eq!(
        parse_commit_sha(json!({"sha":sha}).to_string().as_bytes()).unwrap(),
        sha
    );
    for response in [
        br#"{"sha":"main"}"#.as_slice(),
        br#"{"message":"not found"}"#.as_slice(),
        b"<html>mirror error</html>",
    ] {
        assert!(parse_commit_sha(response).is_err());
    }
    assert!(
        cache_path(Path::new("config"), &SOURCES[0])
            .to_string_lossy()
            .contains("marketplace-origin-official")
    );
}

#[tokio::test]
async fn branch_and_short_revisions_resolve_to_immutable_commits() {
    let sha = "0123456789abcdef0123456789abcdef01234567";
    for revision in ["main", "HEAD", "feature/kit", "abc123"] {
        let source = Source {
            repo: "owner/repo".into(),
            revision: revision.into(),
            path: PathBuf::new(),
        };
        let resolved = resolve_revision_with_fetch(&source, |url| async move {
            let expected_ref = revision.replace('/', "%2F");
            assert_eq!(
                url,
                format!("https://api.github.com/repos/owner/repo/commits/{expected_ref}")
            );
            parse_commit_sha(json!({"sha":sha}).to_string().as_bytes())
        })
        .await
        .unwrap();
        assert_eq!(resolved, sha);
    }
}

#[test]
fn cache_identity_and_duplicate_names_are_checked() {
    let valid =
        json!({"name":"ponytail", "plugins":[{"name":"ponytail", "source":"./"}]}).to_string();
    assert!(parse_catalog(valid.as_bytes(), &SOURCES[1]).is_ok());
    assert!(parse_catalog(valid.as_bytes(), &SOURCES[0]).is_err());
    assert!(parse_catalog(b"<html>proxy error</html>", &SOURCES[0]).is_err());
    let duplicate = json!({"name":"ponytail", "plugins":[{"name":"same", "source":"./"}, {"name":"same", "source":"./"}]}).to_string();
    assert!(parse_catalog(duplicate.as_bytes(), &SOURCES[1]).is_err());
}

fn catalog_bytes(market: &MarketSource, name: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"name":market.catalog_name,"plugins":[{"name":name,"source":"./"}]}))
        .unwrap()
}

#[test]
fn cache_expiry_handles_boundaries_and_clock_changes() {
    let now = SystemTime::now();
    assert!(!cache_is_stale(Some(now), now));
    assert!(!cache_is_stale(
        Some(now - CACHE_TTL + Duration::from_secs(1)),
        now
    ));
    assert!(cache_is_stale(Some(now - CACHE_TTL), now));
    assert!(cache_is_stale(None, now));
    assert!(cache_is_stale(Some(now + Duration::from_secs(1)), now));
}

#[tokio::test]
async fn fresh_and_expired_cache_return_without_network_and_sources_are_isolated() {
    let config = tempfile::tempdir().unwrap();
    let market = &SOURCES[0];
    let path = cache_path(config.path(), market);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, catalog_bytes(market, "cached")).unwrap();
    let (catalog, cached, stale) = catalog_with_fetch(config.path(), market, false, || async {
        panic!("cache hit must not wait on the network")
    })
    .await
    .unwrap();
    assert_eq!(catalog.plugins[0].name, "cached");
    assert!(cached && !stale);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(SystemTime::now() - CACHE_TTL)
        .unwrap();
    let (_, cached, stale) = catalog_with_fetch(config.path(), market, false, || async {
        panic!("expired cache must still be displayed immediately")
    })
    .await
    .unwrap();
    assert!(cached && stale);
    let other = &SOURCES[1];
    let bytes = catalog_bytes(other, "other-market");
    let (catalog, cached, stale) = catalog_with_fetch(config.path(), other, false, || async {
        Ok((parse_catalog(&bytes, other)?, bytes))
    })
    .await
    .unwrap();
    assert_eq!(catalog.plugins[0].name, "other-market");
    assert!(!cached && !stale);
}

#[tokio::test]
async fn forced_refresh_replaces_cache_and_failure_preserves_it() {
    let config = tempfile::tempdir().unwrap();
    let market = &SOURCES[0];
    let path = cache_path(config.path(), market);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, catalog_bytes(market, "old")).unwrap();
    let bytes = catalog_bytes(market, "new");
    let (_, cached, stale) = catalog_with_fetch(config.path(), market, true, || async {
        Ok((parse_catalog(&bytes, market)?, bytes.clone()))
    })
    .await
    .unwrap();
    assert!(!cached && !stale);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let (catalog, cached, _) = catalog_with_fetch(config.path(), market, true, || async {
        anyhow::bail!("offline")
    })
    .await
    .unwrap();
    assert!(cached);
    assert_eq!(catalog.plugins[0].name, "new");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);

    std::fs::write(&path, "invalid cache").unwrap();
    let (catalog, cached, _) = catalog_with_fetch(config.path(), market, false, || async {
        Ok((parse_catalog(&bytes, market)?, bytes.clone()))
    })
    .await
    .unwrap();
    assert!(!cached);
    assert_eq!(catalog.plugins[0].name, "new");
}

#[tokio::test]
async fn successful_fetch_survives_cache_write_failure() {
    let config = tempfile::tempdir().unwrap();
    std::fs::write(config.path().join("cache"), "blocked directory").unwrap();
    let market = &SOURCES[0];
    let bytes = catalog_bytes(market, "available");
    let (catalog, cached, stale) = catalog_with_fetch(config.path(), market, false, || async {
        Ok((parse_catalog(&bytes, market)?, bytes))
    })
    .await
    .unwrap();
    assert!(!cached && !stale);
    assert_eq!(catalog.plugins[0].name, "available");
}

#[test]
fn publishes_only_the_selected_plugin_and_preserves_existing_installs() {
    let config = tempfile::tempdir().unwrap();
    let archive_path = config.path().join("repo.zip");
    archive(
        &archive_path,
        &[
            (
                "repo-main/plugins/kit/.claude-plugin/plugin.json",
                r#"{"name":"kit"}"#,
            ),
            (
                "repo-main/plugins/kit/skills/review/SKILL.md",
                "---\nname: review\ndescription: review\n---\nreview",
            ),
            ("repo-main/other/secret.txt", "not selected"),
        ],
    );
    let e = entry(json!({"name":"kit", "source":"./plugins/kit"}));
    publish_archive(config.path(), &e, &archive_path, Path::new("plugins/kit")).unwrap();
    let plugins = crate::discover(config.path());
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].contributions.skills.len(), 1);
    assert!(!plugins[0].root.join("other").exists());
    std::fs::write(plugins[0].root.join("personal.txt"), "preserve").unwrap();
    assert!(publish_archive(config.path(), &e, &archive_path, Path::new("plugins/kit")).is_err());
    assert_eq!(
        std::fs::read_to_string(plugins[0].root.join("personal.txt")).unwrap(),
        "preserve"
    );
}

#[test]
fn strict_false_entries_and_inline_contributions_are_projected() {
    let config = tempfile::tempdir().unwrap();
    let archive_path = config.path().join("repo.zip");
    archive(
        &archive_path,
        &[(
            "repo-main/skills/review/SKILL.md",
            "---\nname: review\ndescription: review\n---\nreview",
        )],
    );
    let e = entry(
        json!({"name":"kit", "source":"./", "strict":false, "skills":["./skills"],
        "mcpServers":{"docs":{"command":"node", "args":["${CLAUDE_PLUGIN_ROOT}/server.js"]}},
        "hooks":{"hooks":{"SessionStart":[{"matcher":"startup|compact", "hooks":[{"type":"command","command":"echo ${CLAUDE_PLUGIN_ROOT}"}]}]}}}),
    );
    publish_archive(config.path(), &e, &archive_path, Path::new("")).unwrap();
    let plugins = crate::discover(config.path());
    assert_eq!(plugins[0].contributions.skills.len(), 1);
    assert_eq!(plugins[0].contributions.mcp.len(), 1);
    assert_eq!(plugins[0].contributions.hooks.len(), 1);
    let mcp: Value =
        serde_json::from_slice(&std::fs::read(&plugins[0].contributions.mcp[0]).unwrap()).unwrap();
    assert_eq!(
        mcp["mcpServers"]["docs"]["args"][0],
        format!("{}/server.js", plugins[0].root.display())
    );
    let hook = std::fs::read_to_string(&plugins[0].contributions.hooks[0]).unwrap();
    assert!(
        hook.contains("${CLAUDE_PLUGIN_ROOT}"),
        "hooks resolve their root at runtime"
    );
}

#[test]
fn traversal_and_broken_plugins_never_publish() {
    let config = tempfile::tempdir().unwrap();
    let archive_path = config.path().join("repo.zip");
    let e = entry(json!({"name":"kit", "source":"./"}));
    archive(&archive_path, &[("repo-main/../outside", "bad")]);
    assert!(publish_archive(config.path(), &e, &archive_path, Path::new("")).is_err());
    archive(
        &archive_path,
        &[(
            "repo-main/.claude-plugin/plugin.json",
            r#"{"name":"kit","skills":"../../outside"}"#,
        )],
    );
    assert!(publish_archive(config.path(), &e, &archive_path, Path::new("")).is_err());
    assert!(crate::discover(config.path()).is_empty());
    assert!(!config.path().join("plugins/kit").exists());
}

#[tokio::test]
async fn cached_catalog_keeps_personal_installed_state() {
    let config = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(config.path().join("cache")).unwrap();
    std::fs::write(
        cache_path(config.path(), &SOURCES[1]),
        json!({"name":"ponytail", "plugins":[{"name":"ponytail", "source":"./"}]}).to_string(),
    )
    .unwrap();
    let listing = list(config.path(), "ponytail", false).await.unwrap();
    assert!(listing.cached);
    assert!(!listing.plugins[0].installed);
    let manifest = config
        .path()
        .join("plugins/ponytail/.claude-plugin/plugin.json");
    std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    std::fs::write(manifest, r#"{"name":"ponytail"}"#).unwrap();
    assert!(
        list(config.path(), "ponytail", false)
            .await
            .unwrap()
            .plugins[0]
            .installed
    );
    assert!(
        install(config.path(), "ponytail", "../outside")
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires access to public GitHub resources"]
async fn builtin_markets_and_superpowers_install_live() {
    let config = tempfile::tempdir().unwrap();
    for source in &SOURCES {
        let listing = list(config.path(), source.id, true).await.unwrap();
        println!("{}: {} plugins", source.id, listing.plugins.len());
        assert!(!listing.cached && !listing.plugins.is_empty());
    }
    install(config.path(), "ponytail", "ponytail")
        .await
        .unwrap();
    install(config.path(), "superpowers", "superpowers")
        .await
        .unwrap();
    let plugins = crate::discover(config.path());
    assert_eq!(plugins.len(), 2);
    let superpowers = plugins
        .iter()
        .find(|plugin| plugin.name == "superpowers")
        .unwrap();
    assert!(!superpowers.contributions.skills.is_empty());
    assert_eq!(superpowers.contributions.hooks.len(), 1);
    assert!(
        !plugins
            .iter()
            .find(|plugin| plugin.name == "ponytail")
            .unwrap()
            .contributions
            .skills
            .is_empty()
    );
    println!(
        "Superpowers {} installed",
        superpowers.version.as_deref().unwrap_or("unknown version")
    );
}
