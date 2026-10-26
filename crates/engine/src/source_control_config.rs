#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct CheckoutRemoteConfig {
    pub branch_remote: Option<String>,
    pub push_default: Option<String>,
}

impl CheckoutRemoteConfig {
    pub fn parse(output: &str, branch: &str) -> Option<Self> {
        if !output.is_empty() && !output.ends_with('\0') {
            return None;
        }
        let branch_key = format!("branch.{branch}.remote");
        let mut config = Self::default();
        for entry in output.split_terminator('\0') {
            let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
            if key == branch_key {
                config.branch_remote = Some(value.trim().to_owned());
            } else if key == "remote.pushdefault" {
                config.push_default = Some(value.trim().to_owned());
            }
        }
        Some(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_exact_case_sensitive_branch_and_last_values() {
        let config = CheckoutRemoteConfig::parse(
            "branch.feature/One.remote\nwrong\0branch.feature/one.remote\nfirst\0remote.pushdefault\nold\0branch.feature/one.remote\nfork\0remote.pushdefault\norigin\0",
            "feature/one",
        )
        .unwrap();
        assert_eq!(config.branch_remote.as_deref(), Some("fork"));
        assert_eq!(config.push_default.as_deref(), Some("origin"));
    }

    #[test]
    fn preserves_empty_values_and_local_remote() {
        let config =
            CheckoutRemoteConfig::parse("branch.main.remote\n.\0remote.pushdefault\0", "main")
                .unwrap();
        assert_eq!(config.branch_remote.as_deref(), Some("."));
        assert_eq!(config.push_default.as_deref(), Some(""));
    }

    #[test]
    fn ignores_unrelated_multiline_values_and_trims_selected_values() {
        let config = CheckoutRemoteConfig::parse(
            "alias.example\nfirst\nremote.pushdefault\nignored\0branch.dot.name.remote\n fork \n\0",
            "dot.name",
        )
        .unwrap();
        assert_eq!(config.branch_remote.as_deref(), Some("fork"));
        assert_eq!(config.push_default, None);
    }

    #[test]
    fn rejects_incomplete_records_without_inventing_missing_values() {
        assert_eq!(
            CheckoutRemoteConfig::parse("remote.pushdefault\norigin", "main"),
            None
        );
        assert_eq!(
            CheckoutRemoteConfig::parse("", "main"),
            Some(CheckoutRemoteConfig::default())
        );
    }
}
