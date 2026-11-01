//! Aumentum workflow rules, as pure functions (no database), so each rule is unit-tested.
//!
//! A job type is a directed graph of tasks (`wf_task_flow` edges). A job instance sits on one
//! task at a time and moves only along an edge from it; a task with several outgoing edges is a
//! branch (or a rework loop back to an earlier task). A job completes from its job type's end
//! task, or from a task whose only continuation is "end" (`next_task_id` NULL).

use std::collections::BTreeSet;

/// One edge of a job type's task graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub task_id: i32,
    /// `None`: this task can end the job.
    pub next_task_id: Option<i32>,
}

/// The tasks a job on `current` may move to next, in ascending id order (deduplicated).
pub fn next_tasks(edges: &[Edge], current: i32) -> Vec<i32> {
    edges
        .iter()
        .filter(|e| e.task_id == current)
        .filter_map(|e| e.next_task_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Whether a job on `current` may be completed: it is the job type's end task, or the graph marks
/// `current` as ending the job.
pub fn can_complete(edges: &[Edge], end_task_id: Option<i32>, current: i32) -> bool {
    end_task_id == Some(current) || edges.iter().any(|e| e.task_id == current && e.next_task_id.is_none())
}

/// What a request to advance a job resolves to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advance {
    /// Move to this task.
    To(i32),
    /// Complete the job.
    Complete,
}

/// Why an advance was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdvanceError {
    /// The job is not open (completed, ended, ...), so it cannot move.
    NotOpen,
    /// The requested task does not follow the current task in this job type.
    NotANextTask { requested: i32, allowed: Vec<i32> },
    /// No target was given and the job cannot complete from here; one of these must be chosen.
    ChooseNextTask { allowed: Vec<i32> },
}

/// Resolve an advance request.
///
/// * `to` = `Some(task)`: move there, which must be a next task of `current`.
/// * `to` = `None`: complete the job if allowed here; else, if exactly one next task exists,
///   move to it (the common linear step); else the caller must choose.
pub fn resolve_advance(
    edges: &[Edge],
    end_task_id: Option<i32>,
    current: i32,
    is_open: bool,
    to: Option<i32>,
) -> Result<Advance, AdvanceError> {
    if !is_open {
        return Err(AdvanceError::NotOpen);
    }
    let allowed = next_tasks(edges, current);
    match to {
        Some(t) if allowed.contains(&t) => Ok(Advance::To(t)),
        Some(t) => Err(AdvanceError::NotANextTask { requested: t, allowed }),
        None if can_complete(edges, end_task_id, current) => Ok(Advance::Complete),
        None if allowed.len() == 1 => Ok(Advance::To(allowed[0])),
        None => Err(AdvanceError::ChooseNextTask { allowed }),
    }
}

/// Whether a user may act on a task.
///
/// `permitted` are the principals granted the task (`wf_task_permission`); `user_sids` are the
/// user's own sid plus every group it belongs to (directly or through nesting). A task with no
/// grants at all is unrestricted, as in Aumentum. Administrators may always act.
pub fn may_act(permitted: &[i32], user_sids: &[i32], is_admin: bool) -> bool {
    is_admin || permitted.is_empty() || permitted.iter().any(|s| user_sids.contains(s))
}

/// The sids a user acts as: itself plus all groups reachable through `membership`
/// (`(group_sid, member_sid)` pairs; groups may nest).
pub fn effective_sids(user_sid: i32, membership: &[(i32, i32)]) -> Vec<i32> {
    let mut out: BTreeSet<i32> = BTreeSet::from([user_sid]);
    loop {
        let before = out.len();
        for (group, member) in membership {
            if out.contains(member) {
                out.insert(*group);
            }
        }
        if out.len() == before {
            return out.into_iter().collect();
        }
    }
}

/// Parse Aumentum's `Step.commands` text (`<command>X</command>...`) into the ordered tool list.
pub fn parse_commands(xml: &str) -> Vec<String> {
    xml.split("<command>")
        .skip(1)
        .filter_map(|part| part.split("</command>").next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Aumentum job type 9, "Deed Plan Creationv2", exactly as wftaskflow stores it:
    /// 10 Search -> 13 Plot | 12 TDP; 13 Plot -> 14 Review; 14 Review -> 9 Commit | 13 Plot (rework);
    /// 9 Commit -> 12 TDP. End task 12.
    fn deed_plan_v2() -> Vec<Edge> {
        [(10, 13), (13, 14), (14, 9), (9, 12), (14, 13), (10, 12)]
            .into_iter()
            .map(|(t, n)| Edge { task_id: t, next_task_id: Some(n) })
            .collect()
    }

    #[test]
    fn next_tasks_follow_the_graph_including_branches_and_rework() {
        let g = deed_plan_v2();
        assert_eq!(next_tasks(&g, 10), vec![12, 13]); // branch: plot first, or straight to the plan
        assert_eq!(next_tasks(&g, 14), vec![9, 13]); // review: commit, or back to plotting
        assert_eq!(next_tasks(&g, 12), Vec::<i32>::new());
    }

    #[test]
    fn advance_moves_only_along_an_edge() {
        let g = deed_plan_v2();
        assert_eq!(resolve_advance(&g, Some(12), 14, true, Some(13)), Ok(Advance::To(13)));
        assert_eq!(
            resolve_advance(&g, Some(12), 13, true, Some(9)),
            Err(AdvanceError::NotANextTask { requested: 9, allowed: vec![14] })
        );
    }

    #[test]
    fn advance_without_a_target_takes_the_only_path_or_asks() {
        let g = deed_plan_v2();
        assert_eq!(resolve_advance(&g, Some(12), 13, true, None), Ok(Advance::To(14)));
        assert_eq!(
            resolve_advance(&g, Some(12), 14, true, None),
            Err(AdvanceError::ChooseNextTask { allowed: vec![9, 13] })
        );
    }

    #[test]
    fn the_end_task_completes_the_job_and_closed_jobs_do_not_move() {
        let g = deed_plan_v2();
        assert!(can_complete(&g, Some(12), 12));
        assert!(!can_complete(&g, Some(12), 9));
        assert_eq!(resolve_advance(&g, Some(12), 12, true, None), Ok(Advance::Complete));
        assert_eq!(resolve_advance(&g, Some(12), 12, false, None), Err(AdvanceError::NotOpen));
        // A NULL next task also marks an ending task (job types 1-4 are single-task).
        let single = vec![Edge { task_id: 1, next_task_id: None }];
        assert_eq!(resolve_advance(&single, None, 1, true, None), Ok(Advance::Complete));
    }

    #[test]
    fn permissions_come_from_the_user_and_its_nested_groups() {
        // 101 is in group 1 ("users"); group 1 is in group 3 ("everyone").
        let membership = [(1, 101), (3, 1)];
        let sids = effective_sids(101, &membership);
        assert_eq!(sids, vec![1, 3, 101]);
        assert!(may_act(&[3], &sids, false)); // granted to everyone
        assert!(!may_act(&[2], &sids, false)); // admins only
        assert!(may_act(&[2], &sids, true)); // an administrator always may
        assert!(may_act(&[], &sids, false)); // no grants: unrestricted
    }

    #[test]
    fn step_commands_parse_in_order() {
        let xml = "<command>{52D2FA3A-60EF-43bd-A7F9-61C247312080}</command><command>Ils.MultiCadastre.Desktop.SaveLayoutCommand</command>";
        assert_eq!(parse_commands(xml), vec!["{52D2FA3A-60EF-43bd-A7F9-61C247312080}", "Ils.MultiCadastre.Desktop.SaveLayoutCommand"]);
        assert!(parse_commands("").is_empty());
    }
}
