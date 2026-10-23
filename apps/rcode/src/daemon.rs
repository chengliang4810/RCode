//! `rcode daemon …` — install/manage `rcode headless` as a background service:
//! a systemd **user** unit on Linux (the VPS deployment target), a launchd
//! LaunchAgent on macOS. The unit runs the current executable with the
//! `RCODE_*` environment captured at install time, so
//! `RCODE_EDGE_URL=… rcode daemon install` bakes that override in.
//!
//! Where the daemon's **data** lives is not part of that capture: the
//! `location` pointer is read from the config root at every boot, by this
//! service and by the headed app alike, so relocating the data needs no
//! reinstall and cannot leave the two on different directories.
//! `RCODE_DATA_DIR` is the other case — it names an *installation's* config
//! root, so it is captured, and changing it for an installed service means
//! running `rcode daemon install` again.
//!
//! Everything is local: there is no session to save, and the service runs the
//! device's own identity.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

const LAUNCHD_LABEL: &str = "com.rcode";
/// Same unit name the curl|sh installer (`dist/install.sh`) writes, so
/// `rcode daemon …` manages that installation rather than a competing copy.
const SYSTEMD_UNIT: &str = "rcode.service";

/// Environment captured into the unit file. `PATH` is always included (the
/// engine spawns `git`, `gh` and terminals, which service managers' minimal
/// default PATH won't find); the `RCODE_*`/logging vars only when set.
///
/// `RCODE_DATA_DIR` is here on purpose: it names one installation's config
/// root, and the service must open the same one the app that installed it
/// does. The *data* root is not captured — it comes from the `location`
/// pointer each boot, so moving the data needs no reinstall.
const CAPTURED_ENV: &[&str] = &[
    "PATH",
    "RCODE_DATA_DIR",
    "RCODE_EDGE_URL",
    "RCODE_EDGE_TOKEN",
    "RCODE_ORG_ID",
    "RCODE_IPC_PORT",
    "RCODE_HARNESS",
    "RCODE_DEVICE_NAME",
    "RUST_LOG",
];

pub fn install(data_dir: &Path) -> anyhow::Result<()> {
    if !cfg!(any(target_os = "linux", target_os = "macos")) {
        bail!("rcode daemon is only supported on macOS (launchd) and Linux (systemd)");
    }
    let access = rcode_engine::remote_access::RemoteAccess::open(data_dir)?;
    let api_key = std::env::var("RCODE_API_KEY")
        .ok()
        .or_else(|| access.api_key())
        .context("set RCODE_API_KEY before installing the headless service")?;
    rcode_rpc::validate_api_key(&api_key)?;
    let port = std::env::var("RCODE_REMOTE_PORT")
        .ok()
        .map(|value| value.parse::<u16>())
        .transpose()
        .context("invalid RCODE_REMOTE_PORT")?
        .unwrap_or(access.view().port);
    if port == 0 {
        bail!("port must be between 1 and 65535");
    }
    write_service_credentials(data_dir, &api_key, port)?;
    let exe = std::env::current_exe().context("resolving the rcode executable path")?;
    let mut env = captured_env();
    if cfg!(target_os = "macos") {
        env.push(("RCODE_API_KEY".into(), api_key));
        env.push(("RCODE_REMOTE_PORT".into(), port.to_string()));
    }
    if cfg!(target_os = "macos") {
        let plist = launchd_plist_path()?;
        std::fs::create_dir_all(plist.parent().expect("LaunchAgents parent"))?;
        std::fs::create_dir_all(data_dir)?;
        // Reinstall-friendly: unload any previous incarnation before rewriting.
        let _ = run_quiet("launchctl", &["bootout", &launchd_service_target()?]);
        std::fs::write(
            &plist,
            render_launchd_plist(&exe, &env, &data_dir.join("daemon.log")),
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&plist, std::fs::Permissions::from_mode(0o600))?;
        }
        run(
            "launchctl",
            &["bootstrap", &launchd_domain()?, &plist.to_string_lossy()],
        )?;
        println!(
            "Installed and started {LAUNCHD_LABEL} ({}).",
            plist.display()
        );
    } else if cfg!(target_os = "linux") {
        let unit = systemd_unit_path()?;
        std::fs::create_dir_all(unit.parent().expect("systemd user dir"))?;
        let contents = render_systemd_unit(&exe, &env, Some(&data_dir.join("env")))?;
        std::fs::write(&unit, contents)?;
        run("systemctl", &["--user", "daemon-reload"])?;
        run("systemctl", &["--user", "enable", "--now", SYSTEMD_UNIT])?;
        println!("Installed and started {SYSTEMD_UNIT} ({}).", unit.display());
        println!(
            "For start-at-boot without an active login session (VPS): loginctl enable-linger $USER"
        );
    } else {
        bail!("rcode daemon is only supported on macOS (launchd) and Linux (systemd)");
    }
    println!("External WebSocket port: {port}");
    println!(
        "Logs: {}",
        if cfg!(target_os = "macos") {
            format!("{}", data_dir.join("daemon.log").display())
        } else {
            format!("journalctl --user -u {SYSTEMD_UNIT}")
        }
    );
    Ok(())
}

fn write_service_credentials(root: &Path, key: &str, port: u16) -> anyhow::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(root)?;
    let path = root.join("env");
    let existing = match std::fs::symlink_metadata(&path) {
        Ok(meta) if !meta.file_type().is_file() => bail!("service env must be a regular file"),
        Ok(_) => std::fs::read_to_string(&path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    for line in existing.lines().filter(|line| {
        !line.starts_with("RCODE_API_KEY=") && !line.starts_with("RCODE_REMOTE_PORT=")
    }) {
        writeln!(file, "{line}")?;
    }
    writeln!(file, "RCODE_API_KEY={key}\nRCODE_REMOTE_PORT={port}")?;
    file.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

pub fn uninstall() -> anyhow::Result<()> {
    if cfg!(target_os = "macos") {
        let _ = run_quiet("launchctl", &["bootout", &launchd_service_target()?]);
        let plist = launchd_plist_path()?;
        match std::fs::remove_file(&plist) {
            Ok(()) => println!("Removed {}.", plist.display()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                println!("Not installed.")
            }
            Err(err) => return Err(err.into()),
        }
    } else if cfg!(target_os = "linux") {
        let _ = run_quiet("systemctl", &["--user", "disable", "--now", SYSTEMD_UNIT]);
        let unit = systemd_unit_path()?;
        match std::fs::remove_file(&unit) {
            Ok(()) => {
                run("systemctl", &["--user", "daemon-reload"])?;
                println!("Removed {}.", unit.display());
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                println!("Not installed.")
            }
            Err(err) => return Err(err.into()),
        }
    } else {
        bail!("rcode daemon is only supported on macOS (launchd) and Linux (systemd)");
    }
    Ok(())
}

pub fn start() -> anyhow::Result<()> {
    if cfg!(target_os = "macos") {
        let plist = launchd_plist_path()?;
        if !plist.exists() {
            bail!("not installed — run `rcode daemon install` first");
        }
        // `stop` boots the job out of the domain, so start = bootstrap; already
        // loaded is fine, then kickstart guarantees a running process either way.
        let _ = run_quiet(
            "launchctl",
            &["bootstrap", &launchd_domain()?, &plist.to_string_lossy()],
        );
        run("launchctl", &["kickstart", &launchd_service_target()?])?;
    } else if cfg!(target_os = "linux") {
        run("systemctl", &["--user", "start", SYSTEMD_UNIT])?;
    } else {
        bail!("rcode daemon is only supported on macOS (launchd) and Linux (systemd)");
    }
    println!("Started.");
    Ok(())
}

pub fn stop() -> anyhow::Result<()> {
    if cfg!(target_os = "macos") {
        // bootout (not `kill`): with KeepAlive the job would otherwise respawn.
        run("launchctl", &["bootout", &launchd_service_target()?])?;
    } else if cfg!(target_os = "linux") {
        run("systemctl", &["--user", "stop", SYSTEMD_UNIT])?;
    } else {
        bail!("rcode daemon is only supported on macOS (launchd) and Linux (systemd)");
    }
    println!("Stopped.");
    Ok(())
}

pub fn restart() -> anyhow::Result<()> {
    if cfg!(target_os = "macos") {
        if run_quiet(
            "launchctl",
            &["kickstart", "-k", &launchd_service_target()?],
        )
        .is_err()
        {
            // Not loaded (e.g. after `stop`) — fall through to a plain start.
            return start();
        }
        println!("Restarted.");
        Ok(())
    } else if cfg!(target_os = "linux") {
        run("systemctl", &["--user", "restart", SYSTEMD_UNIT])?;
        println!("Restarted.");
        Ok(())
    } else {
        bail!("rcode daemon is only supported on macOS (launchd) and Linux (systemd)");
    }
}

pub fn status() -> anyhow::Result<()> {
    if cfg!(target_os = "macos") {
        let output = Command::new("launchctl")
            .args(["print", &launchd_service_target()?])
            .output()
            .context("running launchctl")?;
        if !output.status.success() {
            println!(
                "{LAUNCHD_LABEL}: not loaded{}",
                if launchd_plist_path()?.exists() {
                    " (installed — `rcode daemon start`)"
                } else {
                    " (not installed — `rcode daemon install`)"
                }
            );
            return Ok(());
        }
        // `launchctl print` is pages long; surface just the liveness lines.
        let text = String::from_utf8_lossy(&output.stdout);
        println!("{LAUNCHD_LABEL}: loaded");
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("state = ")
                || trimmed.starts_with("pid = ")
                || trimmed.starts_with("last exit code = ")
            {
                println!("  {trimmed}");
            }
        }
        Ok(())
    } else if cfg!(target_os = "linux") {
        // Passthrough; `status` exits nonzero for inactive units, which is not an
        // error for us to report — the output already says it.
        let _ = Command::new("systemctl")
            .args(["--user", "--no-pager", "status", SYSTEMD_UNIT])
            .status()
            .context("running systemctl")?;
        Ok(())
    } else {
        bail!("rcode daemon is only supported on macOS (launchd) and Linux (systemd)");
    }
}

// ---------------------------------------------------------------------------
// Unit rendering (pure — unit-tested below)
// ---------------------------------------------------------------------------

fn captured_env() -> Vec<(String, String)> {
    CAPTURED_ENV
        .iter()
        .filter_map(|key| std::env::var(key).ok().map(|v| (key.to_string(), v)))
        .collect()
}

fn render_systemd_unit(
    exe: &Path,
    env: &[(String, String)],
    environment_file: Option<&Path>,
) -> anyhow::Result<String> {
    let environment_file = match environment_file {
        Some(path) => systemd_environment_file_path(path)?,
        // This is an intentional specifier, unlike '%' in an actual path.
        None => "-%h/.rcode/env".into(),
    };
    let mut unit = String::from(
        "[Unit]\nDescription=RCode headless engine\nAfter=network-online.target\nStartLimitIntervalSec=60\nStartLimitBurst=5\n\n[Service]\n",
    );
    for (key, value) in env {
        // systemd unquotes the value; escape the characters it treats specially.
        let value = value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%");
        unit.push_str(&format!("Environment=\"{key}={value}\"\n"));
    }
    unit.push_str(&format!(
        "ExecStart={} headless\nRestart=on-failure\nRestartSec=5\nEnvironmentFile={environment_file}\n\n[Install]\nWantedBy=default.target\n",
        systemd_exec_path(exe)
    ));
    Ok(unit)
}

/// EnvironmentFile= takes one raw path, not a quoted/C-unescaped word.
/// systemd expands unit specifiers, then glob() resolves the file name.
fn systemd_environment_file_path(path: &Path) -> anyhow::Result<String> {
    let mut escaped = String::new();
    for character in path.to_string_lossy().chars() {
        match character {
            '\r' | '\n' => bail!("service environment file path must not contain a line break"),
            '%' => escaped.push_str("%%"),
            '\\' | '*' | '?' | '[' | ']' => {
                escaped.push('\\');
                escaped.push(character);
            }
            _ => escaped.push(character),
        }
    }
    Ok(escaped)
}

/// The ExecStart binary path. An exe under `~/.rcode/app/` came from the
/// curl|sh installer, whose upgrades relink `app/current` — point the unit at
/// the symlink (as the installer's own unit does) so it never pins one version.
/// (`current_exe` resolves symlinks, so the versioned dir is what we see here.)
fn systemd_exec_path(exe: &Path) -> String {
    exec_path_for(exe, std::env::var_os("HOME").map(PathBuf::from).as_deref())
}

fn exec_path_for(exe: &Path, home: Option<&Path>) -> String {
    let installed = home
        .map(|home| home.join(".rcode/app"))
        .is_some_and(|app_root| exe.starts_with(app_root));
    if installed {
        "%h/.rcode/app/current/rcode".to_string()
    } else {
        format!("{}", exe.display())
    }
}

fn render_launchd_plist(exe: &Path, env: &[(String, String)], log: &Path) -> String {
    let mut env_dict = String::new();
    for (key, value) in env {
        env_dict.push_str(&format!(
            "      <key>{}</key><string>{}</string>\n",
            xml_escape(key),
            xml_escape(value)
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>Label</key><string>{label}</string>
    <key>ProgramArguments</key>
    <array>
      <string>{exe}</string>
      <string>headless</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
{env_dict}    </dict>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key>
    <dict>
      <key>SuccessfulExit</key><false/>
    </dict>
    <key>ThrottleInterval</key><integer>30</integer>
    <key>StandardOutPath</key><string>{log}</string>
    <key>StandardErrorPath</key><string>{log}</string>
  </dict>
</plist>
"#,
        label = LAUNCHD_LABEL,
        exe = xml_escape(&exe.to_string_lossy()),
        env_dict = env_dict,
        log = xml_escape(&log.to_string_lossy()),
    )
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ---------------------------------------------------------------------------
// Paths + process helpers
// ---------------------------------------------------------------------------

fn home_dir() -> anyhow::Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME not set")
}

fn launchd_plist_path() -> anyhow::Result<PathBuf> {
    Ok(home_dir()?
        .join("Library/LaunchAgents")
        .join(format!("{LAUNCHD_LABEL}.plist")))
}

fn systemd_unit_path() -> anyhow::Result<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or(home_dir()?.join(".config"));
    Ok(config.join("systemd/user").join(SYSTEMD_UNIT))
}

fn launchd_domain() -> anyhow::Result<String> {
    let output = Command::new("id").arg("-u").output().context("id -u")?;
    let uid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if uid.is_empty() {
        bail!("could not determine the current uid");
    }
    Ok(format!("gui/{uid}"))
}

fn launchd_service_target() -> anyhow::Result<String> {
    Ok(format!("{}/{LAUNCHD_LABEL}", launchd_domain()?))
}

/// Run a command echoing it first; error (with stderr) on nonzero exit.
fn run(program: &str, args: &[&str]) -> anyhow::Result<()> {
    println!("$ {program} {}", args.join(" "));
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("running {program}"))?;
    if !output.status.success() {
        bail!(
            "{program} {} failed ({}): {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Run without echoing; used where failure is an expected branch.
fn run_quiet(program: &str, args: &[&str]) -> anyhow::Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("running {program}"))?;
    if !output.status.success() {
        bail!("{program} failed ({})", output.status);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_unit_shape() {
        let unit = render_systemd_unit(
            Path::new("/usr/local/bin/rcode"),
            &[
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("RCODE_EDGE_URL".into(), "https://edge.example".into()),
                ("RUST_LOG".into(), "info,rcode=\"debug\"".into()),
            ],
            None,
        )
        .unwrap();
        assert!(unit.contains("ExecStart=/usr/local/bin/rcode headless\n"));
        assert!(unit.contains("Environment=\"PATH=/usr/bin:/bin\"\n"));
        assert!(unit.contains("Environment=\"RCODE_EDGE_URL=https://edge.example\"\n"));
        // Inner quotes escaped so systemd re-parses the value verbatim.
        assert!(unit.contains("Environment=\"RUST_LOG=info,rcode=\\\"debug\\\"\"\n"));
        assert!(unit.contains("StartLimitIntervalSec=60\n"));
        assert!(unit.contains("StartLimitBurst=5\n"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(!unit.contains("session.json"));
        assert!(!unit.contains("ConditionPathExists"));
        assert!(unit.contains("EnvironmentFile=-%h/.rcode/env"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn systemd_environment_file_uses_the_actual_config_path_without_quotes() {
        let unit = render_systemd_unit(
            Path::new("/usr/local/bin/rcode"),
            &[],
            Some(Path::new("/opt/rcode-debug/profile/env")),
        )
        .unwrap();
        let lines: Vec<_> = unit
            .lines()
            .filter(|line| line.starts_with("EnvironmentFile="))
            .collect();
        assert_eq!(lines, ["EnvironmentFile=/opt/rcode-debug/profile/env"]);
        assert!(!unit.contains("EnvironmentFile=-%h/.rcode/env"));
    }

    #[test]
    fn systemd_environment_file_preserves_literal_path_characters() {
        // EnvironmentFile first expands % specifiers, then uses glob() with
        // backslash escaping enabled. Spaces/quotes are not word delimiters.
        for (path, expected) in [
            ("/srv/RCode Profiles/env", "/srv/RCode Profiles/env"),
            ("/srv/literal%h%u/env", "/srv/literal%%h%%u/env"),
            (r#"/srv/inner"quote/env"#, r#"/srv/inner"quote/env"#),
            (r"/srv/back\slash/env", r"/srv/back\\slash/env"),
            ("/srv/glob[*?]/env", r"/srv/glob\[\*\?\]/env"),
            (
                r#"/srv/RCode Profiles/%h\slash"quote[*?]/env"#,
                r#"/srv/RCode Profiles/%%h\\slash"quote\[\*\?\]/env"#,
            ),
        ] {
            let unit = render_systemd_unit(
                Path::new("/usr/local/bin/rcode"),
                &[],
                Some(Path::new(path)),
            )
            .unwrap();
            assert_eq!(
                unit.lines()
                    .find_map(|line| line.strip_prefix("EnvironmentFile=")),
                Some(expected),
                "path: {path:?}"
            );
        }
    }

    #[test]
    fn systemd_captured_config_root_keeps_literal_percent_specifiers() {
        let unit = render_systemd_unit(
            Path::new("/usr/local/bin/rcode"),
            &[("RCODE_DATA_DIR".into(), r#"/srv/%h\slash"quote"#.into())],
            Some(Path::new(r#"/srv/%h\slash"quote/env"#)),
        )
        .unwrap();
        assert!(unit.contains("Environment=\"RCODE_DATA_DIR=/srv/%%h\\\\slash\\\"quote\"\n"));
        assert!(unit.contains(r#"EnvironmentFile=/srv/%%h\\slash"quote/env"#));
    }

    #[test]
    fn systemd_environment_file_rejects_line_break_injection() {
        for path in [
            "/srv/profile\nExecStart=/bin/false/env",
            "/srv/profile\renv",
        ] {
            let error = render_systemd_unit(
                Path::new("/usr/local/bin/rcode"),
                &[],
                Some(Path::new(path)),
            )
            .unwrap_err();
            assert!(error.to_string().contains("line break"));
        }
    }

    #[test]
    fn curl_installer_always_starts_the_local_capable_service() {
        // Git for Windows can check out this source fixture with CRLF. These
        // assertions cover the installer directives, not checkout line endings.
        let installer = include_str!("../../../dist/install.sh").replace("\r\n", "\n");
        assert!(!installer.contains("session.json"));
        assert!(installer.contains("StartLimitIntervalSec=60\n"));
        assert!(installer.contains("StartLimitBurst=5\n"));
        assert!(installer.contains("systemctl --user enable rcode"));
        assert!(installer.contains("systemctl --user restart rcode"));
    }

    #[test]
    fn installed_exe_uses_the_current_symlink() {
        // Installer-managed binary (current_exe resolves the `current` symlink to
        // the versioned dir): the unit must point back at the symlink.
        assert_eq!(
            exec_path_for(
                Path::new("/home/u/.rcode/app/0.3.0/rcode"),
                Some(Path::new("/home/u")),
            ),
            "%h/.rcode/app/current/rcode"
        );
        // Source build: literal path.
        assert_eq!(
            exec_path_for(
                Path::new("/src/target/debug/rcode"),
                Some(Path::new("/home/u"))
            ),
            "/src/target/debug/rcode"
        );
    }

    #[test]
    fn launchd_plist_shape() {
        let plist = render_launchd_plist(
            Path::new("/Users/x/rcode & co/rcode"),
            &[("RCODE_EDGE_URL".into(), "https://e?a=1&b=2".into())],
            Path::new("/Users/x/.rcode/daemon.log"),
        );
        assert!(plist.contains("<key>Label</key><string>com.rcode</string>"));
        // XML-escaped exe path and env value.
        assert!(plist.contains("<string>/Users/x/rcode &amp; co/rcode</string>"));
        assert!(plist.contains("<string>https://e?a=1&amp;b=2</string>"));
        assert!(plist.contains("<string>headless</string>"));
        assert!(plist.contains("<key>SuccessfulExit</key><false/>"));
        assert!(
            plist.contains("<key>StandardOutPath</key><string>/Users/x/.rcode/daemon.log</string>")
        );
    }
}
