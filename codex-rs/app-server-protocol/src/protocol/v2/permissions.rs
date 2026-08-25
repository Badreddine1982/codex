use super::shared::v2_enum_from_core;
use crate::JsonSchema;
use crate::TS;
use codex_protocol::approvals::ExecPolicyAmendment as CoreExecPolicyAmendment;
use codex_protocol::approvals::NetworkApprovalContext as CoreNetworkApprovalContext;
use codex_protocol::approvals::NetworkApprovalProtocol as CoreNetworkApprovalProtocol;
use codex_protocol::approvals::NetworkPolicyAmendment as CoreNetworkPolicyAmendment;
use codex_protocol::approvals::NetworkPolicyRuleAction as CoreNetworkPolicyRuleAction;
use codex_protocol::models::ActivePermissionProfile as CoreActivePermissionProfile;
use codex_protocol::models::AdditionalPermissionProfile as CoreAdditionalPermissionProfile;
use codex_protocol::models::FileSystemPermissions as CoreFileSystemPermissions;
use codex_protocol::models::LegacyReadWriteRoots;
use codex_protocol::models::NetworkPermissions as CoreNetworkPermissions;
use codex_protocol::permissions::FileSystemAccessMode as CoreFileSystemAccessMode;
use codex_protocol::permissions::FileSystemPath as CoreFileSystemPath;
use codex_protocol::permissions::FileSystemSandboxEntry as CoreFileSystemSandboxEntry;
use codex_protocol::permissions::FileSystemSpecialPath as CoreFileSystemSpecialPath;
use codex_protocol::protocol::NetworkAccess as CoreNetworkAccess;
use codex_protocol::request_permissions::PermissionGrantScope as CorePermissionGrantScope;
use codex_protocol::request_permissions::RequestPermissionProfile as CoreRequestPermissionProfile;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_path_uri::LegacyAppPathString;
use codex_utils_path_uri::PathConvention;
use codex_utils_path_uri::PathUri;
use serde::Deserialize;
use serde::Serialize;
use std::io;
use std::num::NonZeroUsize;
use std::path::Path;

v2_enum_from_core! {
    pub enum NetworkApprovalProtocol from CoreNetworkApprovalProtocol {
        Http,
        Https,
        Socks5Tcp,
        Socks5Udp,
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct NetworkApprovalContext {
    pub host: String,
    pub protocol: NetworkApprovalProtocol,
}

impl From<CoreNetworkApprovalContext> for NetworkApprovalContext {
    fn from(value: CoreNetworkApprovalContext) -> Self {
        Self {
            host: value.host,
            protocol: value.protocol.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy <-> core path conversion helpers
//
// These shared helpers replace seven previously-duplicated conversion blocks
// for translating between the v2 API's legacy string-based path representation
// (`LegacyAppPathString`) and the core types (`AbsolutePathBuf`/`PathUri`).
//
// They remain necessary because the v2 wire format still carries paths as
// native strings (matching today's clients); once core permission types store
// PathUri end-to-end (Phase 0a completion in protocol), these helpers will
// collapse into a simple `From`/`Into` pair.
// ---------------------------------------------------------------------------

/// Convert a v2 legacy path string to a core [`PathUri`], enforcing the local
/// native path convention. Returns an `io::Error` for foreign or
/// non-representable paths (fail-closed at the API boundary).
fn legacy_path_to_uri(path: LegacyAppPathString) -> io::Result<PathUri> {
    path.to_path_uri(PathConvention::native())
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))
}

/// Convert a core [`PathUri`] to a v2 legacy path string under the local
/// native convention. Foreign URIs return an `io::Error` (fail-closed)
/// because the v2 wire only carries native paths.
fn uri_to_legacy_path(path: &PathUri) -> io::Result<LegacyAppPathString> {
    LegacyAppPathString::from_path_uri(path, PathConvention::native())
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))
}

fn core_path_to_v2(path: CoreFileSystemPath) -> io::Result<FileSystemPath> {
    Ok(match path {
        CoreFileSystemPath::Path { path } => FileSystemPath::Path {
            path: uri_to_legacy_path(&path)?,
        },
        CoreFileSystemPath::GlobPattern { pattern } => FileSystemPath::GlobPattern { pattern },
        CoreFileSystemPath::Special { value } => FileSystemPath::Special {
            value: value.into(),
        },
    })
}

fn v2_path_to_core(path: FileSystemPath) -> io::Result<CoreFileSystemPath> {
    Ok(match path {
        FileSystemPath::Path { path } => CoreFileSystemPath::Path {
            path: legacy_path_to_uri(path)?,
        },
        FileSystemPath::GlobPattern { pattern } => CoreFileSystemPath::GlobPattern { pattern },
        FileSystemPath::Special { value } => CoreFileSystemPath::Special {
            value: value.into(),
        },
    })
}

fn core_entry_to_v2(entry: CoreFileSystemSandboxEntry) -> io::Result<FileSystemSandboxEntry> {
    Ok(FileSystemSandboxEntry {
        path: core_path_to_v2(entry.path)?,
        access: entry.access.into(),
    })
}

fn v2_entry_to_core(entry: FileSystemSandboxEntry) -> io::Result<CoreFileSystemSandboxEntry> {
    Ok(CoreFileSystemSandboxEntry {
        path: v2_path_to_core(entry.path)?,
        access: entry.access.to_core(),
        missing_path_behavior: None,
    })
}

// ---------------------------------------------------------------------------
// AdditionalFileSystemPermissions
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AdditionalFileSystemPermissions {
    /// This will be removed in favor of `entries`.
    pub read: Option<Vec<LegacyAppPathString>>,
    /// This will be removed in favor of `entries`.
    pub write: Option<Vec<LegacyAppPathString>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub glob_scan_max_depth: Option<NonZeroUsize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub entries: Option<Vec<FileSystemSandboxEntry>>,
}

impl From<CoreFileSystemPermissions> for AdditionalFileSystemPermissions {
    fn from(value: CoreFileSystemPermissions) -> Self {
        if let Some(LegacyReadWriteRoots { read, write }) = value.legacy_read_write_roots() {
            let mut entries = Vec::with_capacity(
                read.as_ref().map_or(0, Vec::len) + write.as_ref().map_or(0, Vec::len),
            );
            if let Some(paths) = read.as_ref() {
                entries.extend(paths.iter().map(|path| FileSystemSandboxEntry {
                    path: FileSystemPath::Path {
                        path: uri_to_legacy_path(path).expect(
                            "core permission path should be representable on the local host",
                        ),
                    },
                    access: FileSystemAccessMode::Read,
                }));
            }
            if let Some(paths) = write.as_ref() {
                entries.extend(paths.iter().map(|path| FileSystemSandboxEntry {
                    path: FileSystemPath::Path {
                        path: uri_to_legacy_path(path).expect(
                            "core permission path should be representable on the local host",
                        ),
                    },
                    access: FileSystemAccessMode::Write,
                }));
            }
            Self {
                read: read.map(|paths| {
                    paths
                        .iter()
                        .map(|p| {
                            uri_to_legacy_path(p).expect(
                                "core permission path should be representable on the local host",
                            )
                        })
                        .collect()
                }),
                write: write.map(|paths| {
                    paths
                        .iter()
                        .map(|p| {
                            uri_to_legacy_path(p).expect(
                                "core permission path should be representable on the local host",
                            )
                        })
                        .collect()
                }),
                glob_scan_max_depth: None,
                entries: Some(entries),
            }
        } else {
            Self {
                read: None,
                write: None,
                glob_scan_max_depth: value.glob_scan_max_depth,
                entries: Some(
                    value
                        .entries
                        .into_iter()
                        .map(|e| {
                            core_entry_to_v2(e).expect(
                                "core permission path should be representable on the local host",
                            )
                        })
                        .collect(),
                ),
            }
        }
    }
}

impl TryFrom<AdditionalFileSystemPermissions> for CoreFileSystemPermissions {
    type Error = io::Error;

    fn try_from(value: AdditionalFileSystemPermissions) -> Result<Self, Self::Error> {
        let mut permissions = if let Some(entries) = value.entries {
            Self {
                entries: entries
                    .into_iter()
                    .map(v2_entry_to_core)
                    .collect::<io::Result<_>>()?,
                glob_scan_max_depth: None,
            }
        } else {
            let read = value
                .read
                .map(|paths| {
                    paths
                        .into_iter()
                        .map(legacy_path_to_uri)
                        .collect::<io::Result<Vec<_>>>()
                })
                .transpose()?;
            let write = value
                .write
                .map(|paths| {
                    paths
                        .into_iter()
                        .map(legacy_path_to_uri)
                        .collect::<io::Result<Vec<_>>>()
                })
                .transpose()?;
            CoreFileSystemPermissions::from_read_write_roots(read, write)
        };
        permissions.glob_scan_max_depth = value.glob_scan_max_depth;
        Ok(permissions)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AdditionalNetworkPermissions {
    pub enabled: Option<bool>,
}

impl From<CoreNetworkPermissions> for AdditionalNetworkPermissions {
    fn from(value: CoreNetworkPermissions) -> Self {
        Self {
            enabled: value.enabled,
        }
    }
}

impl From<AdditionalNetworkPermissions> for CoreNetworkPermissions {
    fn from(value: AdditionalNetworkPermissions) -> Self {
        Self {
            enabled: value.enabled,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct RequestPermissionProfile {
    pub network: Option<AdditionalNetworkPermissions>,
    pub file_system: Option<AdditionalFileSystemPermissions>,
}

impl From<CoreRequestPermissionProfile> for RequestPermissionProfile {
    fn from(value: CoreRequestPermissionProfile) -> Self {
        Self {
            network: value.network.map(AdditionalNetworkPermissions::from),
            file_system: value.file_system.map(AdditionalFileSystemPermissions::from),
        }
    }
}

impl TryFrom<RequestPermissionProfile> for CoreRequestPermissionProfile {
    type Error = io::Error;

    fn try_from(value: RequestPermissionProfile) -> Result<Self, Self::Error> {
        Ok(Self {
            network: value.network.map(CoreNetworkPermissions::from),
            file_system: value
                .file_system
                .map(CoreFileSystemPermissions::try_from)
                .transpose()?,
        })
    }
}

v2_enum_from_core!(
    pub enum FileSystemAccessMode from CoreFileSystemAccessMode {
        Read,
        Write,
        Deny
    }
);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(tag = "kind")]
#[ts(export_to = "v2/")]
pub enum FileSystemSpecialPath {
    Root,
    Minimal,
    #[serde(alias = "current_working_directory")]
    ProjectRoots {
        subpath: Option<LegacyAppPathString>,
    },
    Tmpdir,
    SlashTmp,
    Unknown {
        path: String,
        subpath: Option<LegacyAppPathString>,
    },
}

impl From<CoreFileSystemSpecialPath> for FileSystemSpecialPath {
    fn from(value: CoreFileSystemSpecialPath) -> Self {
        match value {
            CoreFileSystemSpecialPath::Root => Self::Root,
            CoreFileSystemSpecialPath::Minimal => Self::Minimal,
            CoreFileSystemSpecialPath::ProjectRoots { subpath } => Self::ProjectRoots {
                subpath: subpath
                    .as_deref()
                    .map(Path::new)
                    .map(LegacyAppPathString::from_path),
            },
            CoreFileSystemSpecialPath::Tmpdir => Self::Tmpdir,
            CoreFileSystemSpecialPath::SlashTmp => Self::SlashTmp,
            CoreFileSystemSpecialPath::Unknown { path, subpath } => Self::Unknown {
                path,
                subpath: subpath
                    .as_deref()
                    .map(Path::new)
                    .map(LegacyAppPathString::from_path),
            },
        }
    }
}

impl From<FileSystemSpecialPath> for CoreFileSystemSpecialPath {
    fn from(value: FileSystemSpecialPath) -> Self {
        match value {
            FileSystemSpecialPath::Root => Self::Root,
            FileSystemSpecialPath::Minimal => Self::Minimal,
            FileSystemSpecialPath::ProjectRoots { subpath } => Self::ProjectRoots {
                subpath: subpath.map(LegacyAppPathString::into_string),
            },
            FileSystemSpecialPath::Tmpdir => Self::Tmpdir,
            FileSystemSpecialPath::SlashTmp => Self::SlashTmp,
            FileSystemSpecialPath::Unknown { path, subpath } => Self::Unknown {
                path,
                subpath: subpath.map(LegacyAppPathString::into_string),
            },
        }
    }
}

/// v2 wire-format filesystem path.
///
/// NOTE: This type shares a name with `codex_protocol::permissions::FileSystemPath`
/// (the core type). Files that import both should rename one at the use site,
/// e.g. `use codex_protocol::permissions::FileSystemPath as CoreFileSystemPath;`,
/// which is already the convention in this file. A rename to a distinct v2
/// name (e.g. `V2FileSystemPath`) is deferred to a follow-up PR so the
/// generated TypeScript filename can be updated in lockstep with downstream
/// consumers.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(tag = "type")]
#[ts(export_to = "v2/")]
pub enum FileSystemPath {
    Path { path: LegacyAppPathString },
    GlobPattern { pattern: String },
    Special { value: FileSystemSpecialPath },
}

impl From<CoreFileSystemPath> for FileSystemPath {
    fn from(value: CoreFileSystemPath) -> Self {
        core_path_to_v2(value).expect(
            "core permission path should be representable on the local host",
        )
    }
}

impl TryFrom<FileSystemPath> for CoreFileSystemPath {
    type Error = io::Error;

    fn try_from(value: FileSystemPath) -> Result<Self, Self::Error> {
        v2_path_to_core(value)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FileSystemSandboxEntry {
    pub path: FileSystemPath,
    pub access: FileSystemAccessMode,
}

impl From<CoreFileSystemSandboxEntry> for FileSystemSandboxEntry {
    fn from(value: CoreFileSystemSandboxEntry) -> Self {
        core_entry_to_v2(value).expect(
            "core permission path should be representable on the local host",
        )
    }
}

impl TryFrom<FileSystemSandboxEntry> for CoreFileSystemSandboxEntry {
    type Error = io::Error;

    fn try_from(value: FileSystemSandboxEntry) -> Result<Self, Self::Error> {
        v2_entry_to_core(value)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct PermissionProfileListParams {
    /// Opaque pagination cursor returned by a previous call.
    #[ts(optional = nullable)]
    pub cursor: Option<String>,
    /// Optional page size; defaults to the full result set.
    #[ts(optional = nullable)]
    pub limit: Option<u32>,
    /// Optional working directory to resolve project config layers.
    #[ts(optional = nullable)]
    pub cwd: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct PermissionProfileSummary {
    /// Available permission profile identifier.
    pub id: String,
    /// Optional user-facing description for display in clients.
    pub description: Option<String>,
    /// Whether the effective requirements allow selecting this profile.
    pub allowed: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct PermissionProfileListResponse {
    pub data: Vec<PermissionProfileSummary>,
    /// Opaque cursor to pass to the next call to continue after the last item.
    /// If None, there are no more items to return.
    pub next_cursor: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ActivePermissionProfile {
    /// Identifier from `default_permissions` or the implicit built-in default,
    /// such as `:workspace` or a user-defined `[permissions.<id>]` profile.
    pub id: String,
    /// Parent profile identifier from the selected permissions profile's
    /// `extends` setting, when present.
    #[serde(default)]
    pub extends: Option<String>,
}

impl ActivePermissionProfile {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            extends: None,
        }
    }

    pub fn read_only() -> Self {
        CoreActivePermissionProfile::read_only().into()
    }
}

impl From<CoreActivePermissionProfile> for ActivePermissionProfile {
    fn from(value: CoreActivePermissionProfile) -> Self {
        Self {
            id: value.id,
            extends: value.extends,
        }
    }
}

impl From<ActivePermissionProfile> for CoreActivePermissionProfile {
    fn from(value: ActivePermissionProfile) -> Self {
        Self {
            id: value.id,
            extends: value.extends,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AdditionalPermissionProfile {
    /// Partial overlay used for per-command permission requests.
    pub network: Option<AdditionalNetworkPermissions>,
    pub file_system: Option<AdditionalFileSystemPermissions>,
}

impl From<CoreAdditionalPermissionProfile> for AdditionalPermissionProfile {
    fn from(value: CoreAdditionalPermissionProfile) -> Self {
        Self {
            network: value.network.map(AdditionalNetworkPermissions::from),
            file_system: value.file_system.map(AdditionalFileSystemPermissions::from),
        }
    }
}

impl TryFrom<AdditionalPermissionProfile> for CoreAdditionalPermissionProfile {
    type Error = io::Error;

    fn try_from(value: AdditionalPermissionProfile) -> Result<Self, Self::Error> {
        Ok(Self {
            network: value.network.map(CoreNetworkPermissions::from),
            file_system: value
                .file_system
                .map(CoreFileSystemPermissions::try_from)
                .transpose()?,
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct GrantedPermissionProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub network: Option<AdditionalNetworkPermissions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub file_system: Option<AdditionalFileSystemPermissions>,
}

impl TryFrom<GrantedPermissionProfile> for CoreAdditionalPermissionProfile {
    type Error = io::Error;

    fn try_from(value: GrantedPermissionProfile) -> Result<Self, Self::Error> {
        Ok(Self {
            network: value.network.map(CoreNetworkPermissions::from),
            file_system: value
                .file_system
                .map(CoreFileSystemPermissions::try_from)
                .transpose()?,
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum NetworkAccess {
    #[default]
    Restricted,
    Enabled,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(tag = "type")]
#[ts(export_to = "v2/")]
pub enum SandboxPolicy {
    DangerFullAccess,
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    ReadOnly {
        #[serde(default)]
        network_access: bool,
    },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    ExternalSandbox {
        #[serde(default)]
        network_access: NetworkAccess,
    },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    // TODO(anp): [v3-pathuri] Move `writable_roots` from `AbsolutePathBuf` to
    // `FileSystemPath::Path` (LegacyAppPathString) once v3 settles the wire
    // representation for foreign paths; kept as AbsolutePathBuf in this Epic
    // to avoid a breaking wire-format change.
    WorkspaceWrite {
        #[serde(default)]
        writable_roots: Vec<AbsolutePathBuf>,
        #[serde(default)]
        network_access: bool,
        #[serde(default)]
        exclude_tmpdir_env_var: bool,
        #[serde(default)]
        exclude_slash_tmp: bool,
    },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum SandboxPolicyDeserialize {
    DangerFullAccess,
    #[serde(rename_all = "camelCase")]
    ReadOnly {
        #[serde(default)]
        network_access: bool,
        #[serde(default)]
        access: Option<LegacyReadOnlyAccess>,
    },
    #[serde(rename_all = "camelCase")]
    ExternalSandbox {
        #[serde(default)]
        network_access: NetworkAccess,
    },
    #[serde(rename_all = "camelCase")]
    WorkspaceWrite {
        #[serde(default)]
        writable_roots: Vec<AbsolutePathBuf>,
        #[serde(default)]
        read_only_access: Option<LegacyReadOnlyAccess>,
        #[serde(default)]
        network_access: bool,
        #[serde(default)]
        exclude_tmpdir_env_var: bool,
        #[serde(default)]
        exclude_slash_tmp: bool,
    },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum LegacyReadOnlyAccess {
    FullAccess,
    Restricted,
}

impl<'de> Deserialize<'de> for SandboxPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match SandboxPolicyDeserialize::deserialize(deserializer)? {
            SandboxPolicyDeserialize::DangerFullAccess => Ok(SandboxPolicy::DangerFullAccess),
            SandboxPolicyDeserialize::ReadOnly {
                network_access,
                access,
            } => {
                if matches!(access, Some(LegacyReadOnlyAccess::Restricted)) {
                    return Err(serde::de::Error::custom(
                        "readOnly.access is no longer supported; use permissionProfile for restricted reads",
                    ));
                }
                Ok(SandboxPolicy::ReadOnly { network_access })
            }
            SandboxPolicyDeserialize::ExternalSandbox { network_access } => {
                Ok(SandboxPolicy::ExternalSandbox { network_access })
            }
            SandboxPolicyDeserialize::WorkspaceWrite {
                writable_roots,
                read_only_access,
                network_access,
                exclude_tmpdir_env_var,
                exclude_slash_tmp,
            } => {
                if matches!(read_only_access, Some(LegacyReadOnlyAccess::Restricted)) {
                    return Err(serde::de::Error::custom(
                        "workspaceWrite.readOnlyAccess is no longer supported; use permissionProfile for restricted reads",
                    ));
                }
                Ok(SandboxPolicy::WorkspaceWrite {
                    writable_roots,
                    network_access,
                    exclude_tmpdir_env_var,
                    exclude_slash_tmp,
                })
            }
        }
    }
}

impl SandboxPolicy {
    pub fn to_core(&self) -> codex_protocol::protocol::SandboxPolicy {
        match self {
            SandboxPolicy::DangerFullAccess => {
                codex_protocol::protocol::SandboxPolicy::DangerFullAccess
            }
            SandboxPolicy::ReadOnly { network_access } => {
                codex_protocol::protocol::SandboxPolicy::ReadOnly {
                    network_access: *network_access,
                }
            }
            SandboxPolicy::ExternalSandbox { network_access } => {
                codex_protocol::protocol::SandboxPolicy::ExternalSandbox {
                    network_access: match network_access {
                        NetworkAccess::Restricted => CoreNetworkAccess::Restricted,
                        NetworkAccess::Enabled => CoreNetworkAccess::Enabled,
                    },
                }
            }
            SandboxPolicy::WorkspaceWrite {
                writable_roots,
                network_access,
                exclude_tmpdir_env_var,
                exclude_slash_tmp,
            } => codex_protocol::protocol::SandboxPolicy::WorkspaceWrite {
                writable_roots: writable_roots.clone(),
                network_access: *network_access,
                exclude_tmpdir_env_var: *exclude_tmpdir_env_var,
                exclude_slash_tmp: *exclude_slash_tmp,
            },
        }
    }
}

impl From<codex_protocol::protocol::SandboxPolicy> for SandboxPolicy {
    fn from(value: codex_protocol::protocol::SandboxPolicy) -> Self {
        match value {
            codex_protocol::protocol::SandboxPolicy::DangerFullAccess => {
                SandboxPolicy::DangerFullAccess
            }
            codex_protocol::protocol::SandboxPolicy::ReadOnly { network_access } => {
                SandboxPolicy::ReadOnly { network_access }
            }
            codex_protocol::protocol::SandboxPolicy::ExternalSandbox { network_access } => {
                SandboxPolicy::ExternalSandbox {
                    network_access: match network_access {
                        CoreNetworkAccess::Restricted => NetworkAccess::Restricted,
                        CoreNetworkAccess::Enabled => NetworkAccess::Enabled,
                    },
                }
            }
            codex_protocol::protocol::SandboxPolicy::WorkspaceWrite {
                writable_roots,
                network_access,
                exclude_tmpdir_env_var,
                exclude_slash_tmp,
            } => SandboxPolicy::WorkspaceWrite {
                writable_roots,
                network_access,
                exclude_tmpdir_env_var,
                exclude_slash_tmp,
            },
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(transparent)]
#[ts(type = "Array<string>", export_to = "v2/")]
pub struct ExecPolicyAmendment {
    pub command: Vec<String>,
}

impl ExecPolicyAmendment {
    pub fn into_core(self) -> CoreExecPolicyAmendment {
        CoreExecPolicyAmendment::new(self.command)
    }
}

impl From<CoreExecPolicyAmendment> for ExecPolicyAmendment {
    fn from(value: CoreExecPolicyAmendment) -> Self {
        Self {
            command: value.command().to_vec(),
        }
    }
}

v2_enum_from_core!(
    pub enum NetworkPolicyRuleAction from CoreNetworkPolicyRuleAction {
        Allow, Deny
    }
);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct NetworkPolicyAmendment {
    pub host: String,
    pub action: NetworkPolicyRuleAction,
}

impl NetworkPolicyAmendment {
    pub fn into_core(self) -> CoreNetworkPolicyAmendment {
        CoreNetworkPolicyAmendment {
            host: self.host,
            action: self.action.to_core(),
        }
    }
}

impl From<CoreNetworkPolicyAmendment> for NetworkPolicyAmendment {
    fn from(value: CoreNetworkPolicyAmendment) -> Self {
        Self {
            host: value.host,
            action: NetworkPolicyRuleAction::from(value.action),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct PermissionsRequestApprovalParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    #[serde(default)]
    pub environment_id: Option<String>,
    /// Unix timestamp (in milliseconds) when this approval request started.
    #[ts(type = "number")]
    pub started_at_ms: i64,
    // TODO(anp): [v3-pathuri] Switch `cwd` to `FileSystemPath::Path` (a
    // LegacyAppPathString) once v3 settles the wire representation; kept as
    // AbsolutePathBuf in this Epic to avoid a breaking client change.
    pub cwd: AbsolutePathBuf,
    pub reason: Option<String>,
    pub permissions: RequestPermissionProfile,
}

v2_enum_from_core!(
    #[derive(Default)]
    pub enum PermissionGrantScope from CorePermissionGrantScope {
        #[default]
        Turn,
        Session
    }
);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct PermissionsRequestApprovalResponse {
    pub permissions: GrantedPermissionProfile,
    #[serde(default)]
    pub scope: PermissionGrantScope,
    /// Review every subsequent command in this turn before normal sandboxed execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub strict_auto_review: Option<bool>,
}
