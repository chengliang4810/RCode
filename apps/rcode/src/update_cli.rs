//! `rcode update` — check for and apply a newer release, natively (the same
//! flow `dist/install.sh` performs: download → verify → symlink swap →
//! service restart). macOS app bundles swap the bundle instead; source builds
//! are report-only.

use anyhow::bail;
use rcode_update::{InstallKind, current_version, version_newer};

/// `--check` prints the verdict and exits (nonzero when an update is available,
/// so scripts can gate on it).
pub async fn update(release_base: &str, check_only: bool) -> anyhow::Result<()> {
    let manifest = rcode_update::fetch_latest(release_base).await?;
    let current = current_version();
    if !version_newer(&manifest.version, current) {
        println!(
            "rcode {current} is up to date (latest: {}).",
            manifest.version
        );
        return Ok(());
    }
    println!("rcode {current} → {} available", manifest.version);
    if check_only {
        std::process::exit(1);
    }

    let install = rcode_update::detect_install();
    if let Some(blocker) = install.desktop_update_blocker() {
        bail!("{blocker}");
    }
    match install {
        InstallKind::Managed { app_root } => {
            println!(
                "downloading {}…",
                rcode_update::headless_artifact(&manifest.version)
            );
            rcode_update::stage_headless(release_base, &manifest, &app_root).await?;
            rcode_update::apply_headless(&app_root, &manifest.version)?;
            println!(
                "installed {} (current → {})",
                app_root.join(&manifest.version).display(),
                manifest.version
            );
            match rcode_update::restart_service() {
                Ok(()) => println!("engine service restarted."),
                Err(err) => println!(
                    "note: service restart failed ({err:#}) — restart the engine manually to finish."
                ),
            }
            Ok(())
        }
        InstallKind::MacApp { bundle } => {
            println!(
                "downloading {}…",
                rcode_update::mac_app_artifact(&manifest.version)
            );
            let data_dir = super::paths::config_dir();
            let staged = rcode_update::stage_mac_app(release_base, &manifest, &data_dir).await?;
            rcode_update::apply_mac_app(&staged, &bundle)?;
            println!("updated {} — relaunch RCode to finish.", bundle.display());
            Ok(())
        }
        #[cfg(windows)]
        InstallKind::WindowsPortable { directory } => {
            let staged = rcode_update::windows::stage(release_base, &manifest, &directory).await?;
            rcode_update::windows::apply(&staged, &directory, false)?;
            println!(
                "updated to {} — relaunch RCode to finish.",
                manifest.version
            );
            Ok(())
        }
        InstallKind::Unmanaged => {
            bail!(
                "this binary is not update-managed (source build or hand-copied).\n\
                 Linux: curl -fsSL https://github.com/chengliang4810/RCode/releases/latest/download/install.sh | sh, or run install.sh from the release tarball\n\
                 macOS: download the new RCode.app dmg, or rebuild from source.\n\
                 Windows: install with the RCode setup .exe from {}, or rebuild from source.",
                rcode_update::LATEST_RELEASE_PAGE
            )
        }
    }
}
