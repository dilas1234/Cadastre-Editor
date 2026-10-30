//! Pure workflow and session logic, decoupled from the database so it can be unit
//! tested exhaustively. The route handlers load rows, call these functions, then
//! persist the result.

use chrono::{DateTime, Duration, Utc};

/// One allowed workflow move: from a stage, an `action` leads to another stage and
/// requires a role. Mirrors a `cadastre.workflow_transition` row by stage *code*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransitionDef {
    /// The action name (e.g. `submit`, `approve`).
    pub action: String,
    /// Stage code the job must currently be in.
    pub from_stage: String,
    /// Stage code the job moves to.
    pub to_stage: String,
    /// Role the actor must hold to perform the move.
    pub required_role: String,
}

/// Why a requested transition cannot be applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransitionError {
    /// No transition exists for this `(stage, action)` pair.
    Unknown,
    /// A transition exists but the actor lacks the required role.
    Forbidden {
        /// The role the transition requires.
        required_role: String,
    },
}

/// Resolve the transition for `action` out of `current_stage`, checking the actor's roles.
///
/// Returns the matched transition on success. This is the single source of truth for
/// "can this move happen", used by `POST /jobs/{id}/transition` (FR-WF-02/03).
pub fn resolve_transition<'a>(
    transitions: &'a [TransitionDef],
    current_stage: &str,
    action: &str,
    user_roles: &[String],
) -> Result<&'a TransitionDef, TransitionError> {
    let transition = transitions
        .iter()
        .find(|t| t.from_stage == current_stage && t.action == action)
        .ok_or(TransitionError::Unknown)?;

    if user_roles.iter().any(|r| r == &transition.required_role) {
        Ok(transition)
    } else {
        Err(TransitionError::Forbidden {
            required_role: transition.required_role.clone(),
        })
    }
}

/// Whether an edit session whose last heartbeat was `last_heartbeat` is stale at `now`,
/// given a time-to-live. Stale sessions release their locks (concurrent-edit control).
pub fn session_is_stale(last_heartbeat: DateTime<Utc>, now: DateTime<Utc>, ttl: Duration) -> bool {
    now.signed_duration_since(last_heartbeat) > ttl
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_workflow() -> Vec<TransitionDef> {
        let t = |action: &str, from: &str, to: &str, role: &str| TransitionDef {
            action: action.into(),
            from_stage: from.into(),
            to_stage: to.into(),
            required_role: role.into(),
        };
        vec![
            t("submit", "draft", "submitted", "editor"),
            t("start_review", "submitted", "under_review", "reviewer"),
            t("approve", "under_review", "approved", "approver"),
            t("reject", "under_review", "rejected", "approver"),
            t("reopen", "rejected", "draft", "editor"),
        ]
    }

    fn roles(rs: &[&str]) -> Vec<String> {
        rs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn valid_transition_with_role_succeeds() {
        let wf = default_workflow();
        let t = resolve_transition(&wf, "draft", "submit", &roles(&["editor"])).unwrap();
        assert_eq!(t.to_stage, "submitted");
    }

    #[test]
    fn transition_from_wrong_stage_is_unknown() {
        let wf = default_workflow();
        // `approve` is only valid from under_review, not from draft.
        assert_eq!(
            resolve_transition(&wf, "draft", "approve", &roles(&["approver"])),
            Err(TransitionError::Unknown)
        );
    }

    #[test]
    fn unknown_action_is_unknown() {
        let wf = default_workflow();
        assert_eq!(
            resolve_transition(&wf, "draft", "teleport", &roles(&["editor"])),
            Err(TransitionError::Unknown)
        );
    }

    #[test]
    fn valid_transition_without_role_is_forbidden() {
        let wf = default_workflow();
        assert_eq!(
            resolve_transition(&wf, "under_review", "approve", &roles(&["editor"])),
            Err(TransitionError::Forbidden {
                required_role: "approver".into()
            })
        );
    }

    #[test]
    fn extra_roles_do_not_block() {
        let wf = default_workflow();
        let rs = roles(&["viewer", "editor", "approver"]);
        assert!(resolve_transition(&wf, "draft", "submit", &rs).is_ok());
    }

    #[test]
    fn full_happy_path_walks_to_approved() {
        let wf = default_workflow();
        let super_roles = roles(&["editor", "reviewer", "approver"]);
        let mut stage = "draft".to_string();
        for action in ["submit", "start_review", "approve"] {
            stage = resolve_transition(&wf, &stage, action, &super_roles)
                .unwrap()
                .to_stage
                .clone();
        }
        assert_eq!(stage, "approved");
    }

    #[test]
    fn session_staleness_respects_ttl() {
        let now = Utc::now();
        let ttl = Duration::minutes(5);
        assert!(!session_is_stale(now - Duration::minutes(4), now, ttl));
        assert!(session_is_stale(now - Duration::minutes(6), now, ttl));
        // Exactly at the TTL is not yet stale (strictly greater-than).
        assert!(!session_is_stale(now - ttl, now, ttl));
    }
}
