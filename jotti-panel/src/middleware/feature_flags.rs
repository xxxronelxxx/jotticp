//! Feature-flag middleware helper.
//!
//! Reseller packages carry a `feature_flags` JSONB column that controls which
//! panel features are available to each client. This module provides a reusable
//! guard that any API handler can call before performing a gated operation.
//!
//! Usage:
//! ```rust
//! require_feature(&state, user_id, "db_manager").await?;
//! ```

use uuid::Uuid;
use crate::{AppState, ApiError, ApiResult};

/// Check whether `flag` is enabled in the user's assigned package.
///
/// Resolution order:
/// 1. Query `user_packages` JOIN `reseller_packages` for the user.
/// 2. Parse `feature_flags` as a JSON object.
/// 3. Return `Ok(())` if `flags[flag] == true`.
/// 4. Return `Err(ApiError::Forbidden)` if the user has **no package** assigned.
///    Previously this defaulted to allow, which would grant Community/unmanaged
///    users access to all gated features.  The correct default is **deny** —
///    unmanaged users must be assigned a package explicitly by an admin.
/// 5. Return `Err(ApiError::Forbidden)` if the flag is explicitly `false`.
/// 6. Return `Err(ApiError::Forbidden)` if the flag key is absent (conservative).
///
/// SECURITY NOTE: Admins bypass feature flag checks at the route level via the
/// `require_admin` guards present in each admin-only handler; `require_feature`
/// is only called in user/reseller-facing handlers where package limits apply.
pub async fn require_feature(
    state: &AppState,
    user_id: Uuid,
    flag: &str,
) -> ApiResult<()> {
    let row = sqlx::query!(
        r#"SELECT rp.feature_flags
           FROM user_packages up
           JOIN reseller_packages rp ON rp.id = up.package_id
           WHERE up.user_id = $1"#,
        user_id
    )
    .fetch_optional(&state.db)
    .await?;

    // No package assigned → DENY.
    //
    // Previous behaviour was "allow" on the assumption that un-packaged users were
    // admins.  Admins do not call `require_feature` (they are blocked at the
    // `require_admin` guard layer), so any user reaching this point with no package
    // is a Community or orphaned account and should receive the most restrictive
    // defaults until an admin explicitly assigns them a plan.
    let Some(row) = row else {
        return Err(ApiError::Forbidden(format!(
            "Feature '{}' requires an active hosting package. \
             Contact your administrator to assign a plan.",
            flag
        )));
    };

    let flags = &row.feature_flags;

    // flags is serde_json::Value (JSONB from sqlx)
    match flags.get(flag) {
        Some(serde_json::Value::Bool(true)) => Ok(()),
        Some(serde_json::Value::Bool(false)) => Err(ApiError::Forbidden(format!(
            "Your hosting package does not include the '{}' feature. \
             Contact your reseller to upgrade.",
            flag
        ))),
        _ => {
            // Key absent or unexpected type — default deny for unknown flags.
            // This prevents newly-introduced feature flags from being implicitly
            // granted to all existing packages before resellers have opted in.
            Err(ApiError::Forbidden(format!(
                "Feature '{}' is not enabled in your hosting package.",
                flag
            )))
        }
    }
}
