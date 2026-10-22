//! Display copy for workspace operations; protocol values stay language-neutral.
use crate::{Localizer, TextKey};
use rcode_proto::WorkspaceMutationRejection;

pub fn selection_label(localizer: &Localizer, count: usize) -> String {
    localizer.format_count(TextKey::FilesSelectionCount, "count", count as i64)
}

pub fn delete_subject(localizer: &Localizer, paths: &[String]) -> String {
    let names = paths
        .iter()
        .map(|path| path.rsplit('/').next().unwrap_or(path))
        .collect::<Vec<_>>();
    match names.as_slice() {
        [] => String::new(),
        [name] => (*name).to_owned(),
        [first, second] => localizer.format_with(
            TextKey::FilesDeleteSubjectTwo,
            &[("first", first), ("second", second)],
        ),
        [first, second, rest @ ..] => localizer.format_with(
            TextKey::FilesDeleteSubjectMany,
            &[
                ("count", &names.len().to_string()),
                ("first", first),
                ("second", second),
                ("remaining", &rest.len().to_string()),
            ],
        ),
    }
}

pub fn mutation_rejection(localizer: &Localizer, reason: WorkspaceMutationRejection) -> String {
    let key = match reason {
        WorkspaceMutationRejection::InvalidPath => TextKey::FilesMutationInvalidPath,
        WorkspaceMutationRejection::WorkspaceChanged => TextKey::FilesMutationWorkspaceChanged,
        WorkspaceMutationRejection::SourceMissing => TextKey::FilesMutationSourceMissing,
        WorkspaceMutationRejection::SourceChanged => TextKey::FilesMutationSourceChanged,
        WorkspaceMutationRejection::DestinationExists => TextKey::FilesMutationDestinationExists,
        WorkspaceMutationRejection::InvalidDestination => TextKey::FilesMutationInvalidDestination,
        WorkspaceMutationRejection::PermissionDenied => TextKey::FilesReadonlyPermission,
        WorkspaceMutationRejection::Unsupported => TextKey::FilesMutationUnsupported,
        WorkspaceMutationRejection::Busy => TextKey::FilesMutationBusy,
        WorkspaceMutationRejection::PartialFailure => TextKey::FilesMutationPartialFailure,
    };
    localizer.text(key)
}

pub fn mutation_failure(
    localizer: &Localizer,
    reason: WorkspaceMutationRejection,
    message: &str,
) -> String {
    use rcode_proto::workspace_errors::WorkspaceFailure;
    if let Some(failure) = WorkspaceFailure::decode(message) {
        return match failure {
            WorkspaceFailure::Native { reason: detail } => localizer.format_with(
                TextKey::FilesErrorDetail,
                &[
                    ("summary", &mutation_rejection(localizer, reason)),
                    ("reason", &detail),
                ],
            ),
            other => workspace_failure(localizer, &other),
        };
    }
    let summary = mutation_rejection(localizer, reason);
    if message.is_empty() {
        summary
    } else {
        localizer.format_with(
            TextKey::FilesErrorDetail,
            &[("summary", &summary), ("reason", message)],
        )
    }
}

pub fn workspace_failure(
    localizer: &Localizer,
    failure: &rcode_proto::workspace_errors::WorkspaceFailure,
) -> String {
    use rcode_proto::workspace_errors::WorkspaceFailure::*;
    let key = match failure {
        Validation { rule } => return localizer.text(validation_key(*rule)),
        QueryTooLong { maximum } => {
            return localizer.format_with(
                TextKey::FilesQueryTooLong,
                &[("maximum", &maximum.to_string())],
            );
        }
        WriteTooLarge { maximum } => {
            return localizer.format_with(
                TextKey::FilesWriteTooLarge,
                &[("maximum", &maximum.to_string())],
            );
        }
        WorkspaceChanged => TextKey::FilesChangedBanner,
        WorkspaceChangedSaving => TextKey::FilesOperationChangedSaving,
        CheckoutUnavailable => TextKey::FilesCheckoutUnavailable,
        DirectoryChanged => TextKey::FilesDirectoryChanged,
        DirectoryCursorRepeated => TextKey::FilesDirectoryCursorRepeated,
        ImageChanged => TextKey::FilesImageChanged,
        ImageChunkInvalid => TextKey::FilesImageChunkInvalid,
        ImageChunkLimit => TextKey::FilesImageChunkLimit,
        RequestObjectRequired => TextKey::FilesRequestObjectRequired,
        MemoryUnavailable => TextKey::FilesMemoryUnavailable,
        MemoryDirectoryUnavailable { .. } => TextKey::FilesMemoryDirectoryUnavailable,
        InvalidTarget => TextKey::FilesInvalidTarget,
        CheckoutPathRequiresSpace => TextKey::FilesCheckoutPathRequiresSpace,
        ChatMissing => TextKey::FilesChatMissing,
        SpaceMissing => TextKey::FilesSpaceMissing,
        DifferentDevice => TextKey::FilesDifferentDevice,
        FolderAbsoluteRequired => TextKey::FilesFolderAbsoluteRequired,
        FolderUnavailable { .. } => TextKey::FilesFolderUnavailable,
        FolderNotDirectory => TextKey::FilesFolderNotDirectory,
        FolderMissing => TextKey::FilesFolderMissing,
        NotCheckout => TextKey::FilesNotCheckout,
        ServiceStopping => TextKey::FilesServiceStopping,
        SearchCancelled => TextKey::FilesSearchCancelled,
        ReadCancelled => TextKey::FilesReadCancelled,
        WriteCancelled => TextKey::FilesWriteCancelled,
        FileMissing => TextKey::FilesFileNotFound,
        FileChanged => TextKey::FilesChangedOutside,
        WorkerFailed { .. } => TextKey::FilesWorkerFailed,
        Native { .. } => TextKey::FilesNativeFailed,
        RecoveryRequired { .. } => TextKey::FilesRecoveryRequired,
    };
    match failure {
        MemoryDirectoryUnavailable { reason }
        | FolderUnavailable { reason }
        | WorkerFailed { reason }
        | Native { reason } => localizer.format_with(key, &[("reason", reason)]),
        RecoveryRequired {
            path,
            reason,
            rollback,
        } => localizer.format_with(
            key,
            &[("path", path), ("reason", reason), ("rollback", rollback)],
        ),
        _ => localizer.text(key),
    }
}

fn validation_key(rule: rcode_proto::workspace_errors::WorkspaceRule) -> TextKey {
    use rcode_proto::workspace_errors::WorkspaceRule;
    match rule {
        WorkspaceRule::MutationChanged => TextKey::FilesRuleMutationChanged,
        WorkspaceRule::OperationIdentity => TextKey::FilesRuleOperationIdentity,
        WorkspaceRule::DeleteRecursive => TextKey::FilesRuleDeleteRecursive,
        WorkspaceRule::MutationCancelled => TextKey::FilesRuleMutationCancelled,
        WorkspaceRule::CanonicalSpelling => TextKey::FilesRuleCanonicalSpelling,
        WorkspaceRule::MutationUnsupported => TextKey::FilesRuleMutationUnsupported,
        WorkspaceRule::MutationSourceChanged => TextKey::FilesRuleMutationSourceChanged,
        WorkspaceRule::DestinationSpelling => TextKey::FilesRuleDestinationSpelling,
        WorkspaceRule::MoveIntoSelf => TextKey::FilesRuleMoveIntoSelf,
        WorkspaceRule::PathEmpty => TextKey::FilesRulePathEmpty,
        WorkspaceRule::PathTooLong => TextKey::FilesRulePathTooLong,
        WorkspaceRule::PathInvalidCharacter => TextKey::FilesRulePathInvalidCharacter,
        WorkspaceRule::PathRelative => TextKey::FilesRulePathRelative,
        WorkspaceRule::PathTraversal => TextKey::FilesRulePathTraversal,
        WorkspaceRule::PathComponents => TextKey::FilesRulePathComponents,
        WorkspaceRule::PathUtf8 => TextKey::FilesRulePathUtf8,
        WorkspaceRule::GitPath => TextKey::FilesRuleGitPath,
        WorkspaceRule::ImageHash => TextKey::FilesRuleImageHash,
        WorkspaceRule::ImageOffset => TextKey::FilesRuleImageOffset,
        WorkspaceRule::FileComponent => TextKey::FilesRuleFileComponent,
        WorkspaceRule::DirectoryComponent => TextKey::FilesRuleDirectoryComponent,
        WorkspaceRule::RelativePath => TextKey::FilesRuleRelativePath,
        WorkspaceRule::DirectoryCursor => TextKey::FilesRuleDirectoryCursor,
        WorkspaceRule::DirectoryCursorMismatch => TextKey::FilesRuleDirectoryCursorMismatch,
        WorkspaceRule::DirectoryCursorRange => TextKey::FilesRuleDirectoryCursorRange,
        WorkspaceRule::QueryEmpty => TextKey::FilesRuleQueryEmpty,
        WorkspaceRule::UnsupportedImage => TextKey::FilesRuleUnsupportedImage,
        WorkspaceRule::ImageTooLarge => TextKey::FilesRuleImageTooLarge,
        WorkspaceRule::Symlink => TextKey::FilesRuleSymlink,
        WorkspaceRule::SymlinkTraversal => TextKey::FilesRuleSymlinkTraversal,
        WorkspaceRule::SymlinkDirectory => TextKey::FilesRuleSymlinkDirectory,
        WorkspaceRule::NotFile => TextKey::FilesRuleNotFile,
        WorkspaceRule::ParentNotDirectory => TextKey::FilesRuleParentNotDirectory,
        WorkspaceRule::OutsideWorkspace => TextKey::FilesRuleOutsideWorkspace,
        WorkspaceRule::WriteNul => TextKey::FilesRuleWriteNul,
        WorkspaceRule::WriteLineEndings => TextKey::FilesRuleWriteLineEndings,
        WorkspaceRule::WriteHash => TextKey::FilesRuleWriteHash,
        WorkspaceRule::BinaryWrite => TextKey::FilesRuleBinaryWrite,
        WorkspaceRule::EncodingWrite => TextKey::FilesRuleEncodingWrite,
        WorkspaceRule::MixedWrite => TextKey::FilesRuleMixedWrite,
        WorkspaceRule::NoParent => TextKey::FilesRuleNoParent,
        WorkspaceRule::VerifyWrite => TextKey::FilesRuleVerifyWrite,
        WorkspaceRule::FileChangedPreparing => TextKey::FilesRuleFileChangedPreparing,
        WorkspaceRule::ImageChangedLimit => TextKey::FilesRuleImageChangedLimit,
        WorkspaceRule::DirectoryCancelled => TextKey::FilesRuleDirectoryCancelled,
        WorkspaceRule::DirectoryOutside => TextKey::FilesRuleDirectoryOutside,
        WorkspaceRule::EntryOutside => TextKey::FilesRuleEntryOutside,
        WorkspaceRule::DirectoryNotDirectory => TextKey::FilesRuleDirectoryNotDirectory,
    }
}

pub fn batch_failure(
    localizer: &Localizer,
    moving: bool,
    completed: usize,
    total: usize,
    name: &str,
    message: &str,
) -> String {
    localizer.format_with(
        if moving {
            TextKey::FilesBatchMoveFailed
        } else {
            TextKey::FilesBatchDeleteFailed
        },
        &[
            ("completed", &completed.to_string()),
            ("total", &total.to_string()),
            ("name", name),
            ("reason", message),
        ],
    )
}

/// Current hosts send a code plus native diagnostics; old hosts get readable
/// generic copy rather than leaking an untranslated application error sentence.
pub fn terminal_open_failure(localizer: &Localizer, message: &str) -> String {
    use rcode_proto::terminal_errors::TerminalOpenFailure;
    match TerminalOpenFailure::decode(message) {
        Some(TerminalOpenFailure::Limit { maximum }) => localizer.format_with(
            TextKey::TerminalLimitReached,
            &[("maximum", &maximum.to_string())],
        ),
        Some(TerminalOpenFailure::WorkingDirectoryUnavailable) => {
            localizer.text(TextKey::TerminalWorkingDirectoryUnavailable)
        }
        Some(TerminalOpenFailure::Startup { reason }) => {
            localizer.format_with(TextKey::TerminalStartupFailed, &[("reason", &reason)])
        }
        Some(TerminalOpenFailure::Launch { shell, reason }) => localizer.format_with(
            TextKey::TerminalLaunchFailed,
            &[("shell", &shell), ("reason", &reason)],
        ),
        None => localizer.format_with(TextKey::TerminalOpenFailed, &[("reason", message)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LanguagePreference;

    #[test]
    fn selected_names_and_counts_follow_the_display_language() {
        let en = Localizer::new(LanguagePreference::EnUs);
        let zh = Localizer::new(LanguagePreference::ZhCn);
        let paths = vec!["src/a.txt".into(), "b.txt".into(), "c.txt".into()];
        assert_eq!(selection_label(&en, 1), "1 item");
        assert_eq!(selection_label(&en, 3), "3 items");
        assert_eq!(selection_label(&zh, 3), "3 个条目");
        assert_eq!(
            delete_subject(&en, &paths[..2]),
            "2 items (a.txt and b.txt)"
        );
        assert_eq!(
            delete_subject(&zh, &paths),
            "3 个条目（a.txt、b.txt 等，另有 1 个）"
        );
        assert_eq!(delete_subject(&zh, &paths[..1]), "a.txt");
    }

    #[test]
    fn every_mutation_reason_has_a_localized_recovery_message() {
        use WorkspaceMutationRejection::*;
        let en = Localizer::new(LanguagePreference::EnUs);
        let zh = Localizer::new(LanguagePreference::ZhCn);
        for reason in [
            InvalidPath,
            WorkspaceChanged,
            SourceMissing,
            SourceChanged,
            DestinationExists,
            InvalidDestination,
            PermissionDenied,
            Unsupported,
            Busy,
            PartialFailure,
        ] {
            let english = mutation_rejection(&en, reason);
            let chinese = mutation_rejection(&zh, reason);
            assert!(!english.is_empty());
            assert_ne!(english, chinese, "{reason:?}");
            assert!(!english.starts_with("files-"));
            assert!(!chinese.starts_with("files-"));
        }
        let message = batch_failure(
            &zh,
            true,
            2,
            5,
            "a.txt",
            &mutation_rejection(&zh, DestinationExists),
        );
        assert!(message.contains("2/5"));
        assert!(message.contains("a.txt"));
        assert!(!message.contains("Moved"));
    }

    #[test]
    fn terminal_open_codes_localize_and_keep_only_native_diagnostics() {
        use rcode_proto::terminal_errors::TerminalOpenFailure;
        let en = Localizer::new(LanguagePreference::EnUs);
        let zh = Localizer::new(LanguagePreference::ZhCn);
        for error in [
            TerminalOpenFailure::Limit { maximum: 32 },
            TerminalOpenFailure::WorkingDirectoryUnavailable,
            TerminalOpenFailure::Startup {
                reason: "EIO".into(),
            },
            TerminalOpenFailure::Launch {
                shell: "pwsh".into(),
                reason: "ENOENT".into(),
            },
        ] {
            assert_ne!(
                terminal_open_failure(&en, &error.encode()),
                terminal_open_failure(&zh, &error.encode())
            );
        }
        let error = TerminalOpenFailure::Launch {
            shell: "pwsh".into(),
            reason: "ENOENT".into(),
        };
        let message = terminal_open_failure(&zh, &error.encode());
        assert!(message.contains("pwsh") && message.contains("ENOENT"));
        assert!(!message.contains("could not spawn"));
        assert!(terminal_open_failure(&zh, "legacy host error").contains("legacy host error"));
    }

    #[test]
    fn recovery_failures_preserve_the_recovery_location_and_system_errors() {
        use rcode_proto::workspace_errors::WorkspaceFailure;
        let zh = Localizer::new(LanguagePreference::ZhCn);
        let failure = WorkspaceFailure::RecoveryRequired {
            path: "src/recovery.tmp".into(),
            reason: "EIO".into(),
            rollback: "EACCES".into(),
        };
        let message = mutation_failure(
            &zh,
            WorkspaceMutationRejection::PartialFailure,
            &failure.encode(),
        );
        for detail in ["src/recovery.tmp", "EIO", "EACCES"] {
            assert!(message.contains(detail));
        }
        assert!(!message.contains("Rename could not"));
        let legacy = "Recover the entry at src/legacy-recovery.tmp";
        assert!(
            mutation_failure(&zh, WorkspaceMutationRejection::PartialFailure, legacy)
                .contains(legacy)
        );
    }
}
