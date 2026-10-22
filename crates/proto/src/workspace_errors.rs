//! Structured workspace failures carried by the existing RPC error string.
use serde::{Deserialize, Serialize};

const PREFIX: &str = "rcode.workspace:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum WorkspaceFailure {
    Validation {
        rule: WorkspaceRule,
    },
    QueryTooLong {
        maximum: usize,
    },
    WriteTooLarge {
        maximum: u64,
    },
    WorkspaceChanged,
    WorkspaceChangedSaving,
    CheckoutUnavailable,
    DirectoryChanged,
    DirectoryCursorRepeated,
    ImageChanged,
    ImageChunkInvalid,
    ImageChunkLimit,
    RequestObjectRequired,
    MemoryUnavailable,
    MemoryDirectoryUnavailable {
        reason: String,
    },
    InvalidTarget,
    CheckoutPathRequiresSpace,
    ChatMissing,
    SpaceMissing,
    DifferentDevice,
    FolderAbsoluteRequired,
    FolderUnavailable {
        reason: String,
    },
    FolderNotDirectory,
    FolderMissing,
    NotCheckout,
    ServiceStopping,
    SearchCancelled,
    ReadCancelled,
    WriteCancelled,
    FileMissing,
    FileChanged,
    WorkerFailed {
        reason: String,
    },
    Native {
        reason: String,
    },
    RecoveryRequired {
        path: String,
        reason: String,
        rollback: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceRule {
    MutationChanged,
    OperationIdentity,
    DeleteRecursive,
    MutationCancelled,
    CanonicalSpelling,
    MutationUnsupported,
    MutationSourceChanged,
    DestinationSpelling,
    MoveIntoSelf,
    PathEmpty,
    PathTooLong,
    PathInvalidCharacter,
    PathRelative,
    PathTraversal,
    PathComponents,
    PathUtf8,
    GitPath,
    ImageHash,
    ImageOffset,
    FileComponent,
    DirectoryComponent,
    RelativePath,
    DirectoryCursor,
    DirectoryCursorMismatch,
    DirectoryCursorRange,
    QueryEmpty,
    UnsupportedImage,
    ImageTooLarge,
    Symlink,
    SymlinkTraversal,
    SymlinkDirectory,
    NotFile,
    ParentNotDirectory,
    OutsideWorkspace,
    WriteNul,
    WriteLineEndings,
    WriteHash,
    BinaryWrite,
    EncodingWrite,
    MixedWrite,
    NoParent,
    VerifyWrite,
    FileChangedPreparing,
    ImageChangedLimit,
    DirectoryCancelled,
    DirectoryOutside,
    EntryOutside,
    DirectoryNotDirectory,
}

impl WorkspaceRule {
    pub fn encode(self) -> String {
        WorkspaceFailure::Validation { rule: self }.encode()
    }
}

impl WorkspaceFailure {
    pub fn encode(&self) -> String {
        format!(
            "{PREFIX}{}",
            serde_json::to_string(self).expect("workspace failure serializes")
        )
    }

    pub fn decode(message: &str) -> Option<Self> {
        serde_json::from_str(message.strip_prefix(PREFIX)?).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_failures_keep_recovery_parameters_and_native_diagnostics() {
        let failure = WorkspaceFailure::RecoveryRequired {
            path: "src/恢复.tmp".into(),
            reason: "EIO".into(),
            rollback: "EACCES".into(),
        };
        assert_eq!(WorkspaceFailure::decode(&failure.encode()), Some(failure));
        assert!(WorkspaceFailure::decode("legacy peer error").is_none());
        assert!(WorkspaceFailure::decode("rcode.workspace:{\"code\":\"future\"}").is_none());
    }
}
