use codex_apply_patch::ApplyPatchAction;
use codex_apply_patch::ApplyPatchFileChange;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_sandboxing::get_platform_sandbox;
use codex_utils_path_uri::PathUri;

const PATCH_REJECTED_OUTSIDE_PROJECT_REASON: &str =
    "writing outside of the project; rejected by user approval settings";
const PATCH_REJECTED_READ_ONLY_REASON: &str =
    "writing is blocked by read-only sandbox; rejected by user approval settings";

#[derive(Debug, PartialEq)]
pub enum SafetyCheck {
    AutoApprove,
    AskUser,
    Reject { reason: String },
}

pub fn assess_patch_safety(
    action: &ApplyPatchAction,
    policy: AskForApproval,
    permission_profile: &PermissionProfile,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    cwd: &PathUri,
    windows_sandbox_level: WindowsSandboxLevel,
) -> SafetyCheck {
    if action.is_empty() {
        return SafetyCheck::Reject {
            reason: "empty patch".to_string(),
        };
    }

    match policy {
        AskForApproval::Never | AskForApproval::OnRequest | AskForApproval::Granular(_) => {
            // Continue to see if this can be auto-approved.
        }
        // TODO(ragona): I'm not sure this is actually correct? I believe in this case
        // we want to continue to the writable paths check before asking the user.
        AskForApproval::UnlessTrusted => {
            return SafetyCheck::AskUser;
        }
    }

    let rejects_sandbox_approval = matches!(policy, AskForApproval::Never)
        || matches!(
            policy,
            AskForApproval::Granular(granular_config) if !granular_config.sandbox_approval
        );

    // Filesystem sandbox policies now operate on PathUri directly via
    // `can_write_path_uri_with_cwd` / `get_writable_roots_for_path_uri`, which
    // perform host projection internally and fail closed (deny) for
    // foreign-host or foreign-convention URIs.
    if is_write_patch_constrained_to_writable_paths(action, file_system_sandbox_policy, cwd) {
        if matches!(
            permission_profile,
            PermissionProfile::Disabled | PermissionProfile::External { .. }
        ) {
            // Disabled and External profiles intentionally do not apply an
            // outer Codex filesystem sandbox.
            SafetyCheck::AutoApprove
        } else {
            // Only auto‑approve when we can actually enforce a sandbox. Otherwise
            // fall back to asking the user because the patch may touch arbitrary
            // paths outside the project.
            match get_platform_sandbox(windows_sandbox_level != WindowsSandboxLevel::Disabled) {
                Some(_) => SafetyCheck::AutoApprove,
                None => {
                    if rejects_sandbox_approval {
                        SafetyCheck::Reject {
                            reason: patch_rejection_reason(
                                permission_profile,
                                file_system_sandbox_policy,
                                cwd,
                            )
                            .to_string(),
                        }
                    } else {
                        SafetyCheck::AskUser
                    }
                }
            }
        }
    } else if rejects_sandbox_approval {
        SafetyCheck::Reject {
            reason: patch_rejection_reason(permission_profile, file_system_sandbox_policy, cwd)
                .to_string(),
        }
    } else {
        SafetyCheck::AskUser
    }
}

fn patch_rejection_reason(
    permission_profile: &PermissionProfile,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    cwd: &PathUri,
) -> &'static str {
    // A foreign cwd cannot be projected to localhost; we can't inspect writable
    // roots, so fall back to the safer "outside project" rejection reason.
    let has_no_writable_roots = cwd
        .project_to_localhost()
        .ok()
        .is_some_and(|native_cwd| {
            file_system_sandbox_policy
                .get_writable_roots_with_cwd(native_cwd.as_path())
                .is_empty()
        });
    match permission_profile {
        PermissionProfile::Managed { .. }
            if !file_system_sandbox_policy.has_full_disk_write_access()
                && has_no_writable_roots =>
        {
            PATCH_REJECTED_READ_ONLY_REASON
        }
        PermissionProfile::Managed { .. }
        | PermissionProfile::Disabled
        | PermissionProfile::External { .. } => PATCH_REJECTED_OUTSIDE_PROJECT_REASON,
    }
}

fn is_write_patch_constrained_to_writable_paths(
    action: &ApplyPatchAction,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    cwd: &PathUri,
) -> bool {
    // A full-disk policy permits every patch target, so no per-path writable-root check can
    // further constrain the result.
    if file_system_sandbox_policy.has_full_disk_write_access() {
        return true;
    }
    // Sandbox policy path checks accept PathUri directly; host projection
    // happens in the policy (fail closed for foreign-host/convention URIs).
    for (path, change) in action.changes() {
        match change {
            ApplyPatchFileChange::Add { .. } | ApplyPatchFileChange::Delete { .. } => {
                if !file_system_sandbox_policy.can_write_path_uri_with_cwd(path, cwd) {
                    return false;
                }
            }
            ApplyPatchFileChange::Update { move_path, .. } => {
                if !file_system_sandbox_policy.can_write_path_uri_with_cwd(path, cwd) {
                    return false;
                }
                if let Some(dest) = move_path
                    && !file_system_sandbox_policy.can_write_path_uri_with_cwd(dest, cwd)
                {
                    return false;
                }
            }
        }
    }

    true
}

#[cfg(test)]
#[path = "safety_tests.rs"]
mod tests;
