//! Goal loop: coder ⟷ reviewer rounds until the reviewer returns
//! [`Verdict::Pass`], the round cap is reached, or the coder stops making
//! progress.
//!
//! The reviewer's exit-code contract ([`Verdict::exit_code`]) already
//! documents this as "Empryo's goal-loop contract" (PASS=0, FAIL=3,
//! PARTIAL=1); this module is the loop that produces a final [`Verdict`] for
//! that contract to report.
//!
//! Spec: `docs/research/empryo-port/STATE.md`, `## NEXT` item 3.

use smol_str::SmolStr;

use crate::agents::{AgentContext, AgentRequest, AgentRunner};
use crate::protocol::AgentKind;
use crate::review::{Review, ReviewRequest, Reviewer, Verdict};

/// Why the loop stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalStopReason {
    /// The reviewer returned [`Verdict::Pass`].
    Passed,
    /// The round cap was reached without a pass.
    RoundCapReached,
    /// The coder produced the same evidence as the previous round. A round
    /// that changes nothing cannot earn a different verdict, so the loop
    /// stops instead of spending the rest of the cap restating the same
    /// question to the reviewer.
    Oscillating,
}

/// One coder round and the reviewer's judgment of it.
#[derive(Debug, Clone)]
pub struct GoalRound {
    pub round: u32,
    /// The coder's reply for this round, verbatim. This is what the reviewer
    /// judged, and what the next round's oscillation check compares against.
    pub evidence: SmolStr,
    pub verdict: Verdict,
    pub notes: SmolStr,
}

/// The loop's final outcome: every round it ran, and why it stopped.
#[derive(Debug, Clone)]
pub struct GoalOutcome {
    pub stop_reason: GoalStopReason,
    pub rounds: Vec<GoalRound>,
}

impl GoalOutcome {
    /// The last round's verdict. A loop always runs at least one round
    /// (`max_rounds` is clamped to 1), so this only falls back to
    /// [`Verdict::Partial`] in the unreachable case of an empty round list —
    /// kept honest rather than panicking.
    pub fn verdict(&self) -> Verdict {
        self.rounds
            .last()
            .map(|round| round.verdict)
            .unwrap_or(Verdict::Partial)
    }

    /// Process exit code, matching [`Verdict::exit_code`]. [`GoalStopReason`]
    /// explains *why* the loop stopped; the exit code always comes from the
    /// last verdict a reviewer actually gave — oscillation and the round cap
    /// are reasons that verdict was final, not codes of their own.
    pub fn exit_code(&self) -> i32 {
        self.verdict().exit_code()
    }
}

/// Runs a coder↔reviewer cycle for `goal` until pass, the round cap, or
/// oscillation.
///
/// `coder` does the work each round; its reply *is* the evidence handed to
/// `reviewer`. On anything but a pass, the reviewer's notes are folded into
/// the next round's task so the coder sees what it needs to fix.
///
/// `max_rounds` is clamped to at least 1: a goal loop that cannot run is not
/// a useful contract to expose.
pub async fn run_goal_loop(
    coder: &dyn AgentRunner,
    reviewer: &dyn Reviewer,
    goal: &str,
    max_rounds: u32,
) -> Result<GoalOutcome, SmolStr> {
    let max_rounds = max_rounds.max(1);
    let mut rounds = Vec::new();
    let mut previous_evidence: Option<SmolStr> = None;
    let mut task: SmolStr = goal.into();

    for round in 1..=max_rounds {
        let evidence = coder
            .run(
                AgentRequest {
                    id: format!("goal-loop-round-{round}").into(),
                    name: "coder".into(),
                    task: task.clone(),
                    kind: AgentKind::Subagent,
                    parent_id: None,
                },
                AgentContext::detached(),
            )
            .await?;

        if previous_evidence.as_deref() == Some(evidence.as_str()) {
            // Same reply as last round: the previous round already holds a
            // non-passing verdict for this exact evidence (that is the only
            // way execution reaches round 2+ at all), so re-reviewing it
            // would ask the same question twice. Stop without spending
            // another review call.
            return Ok(GoalOutcome {
                stop_reason: GoalStopReason::Oscillating,
                rounds,
            });
        }

        let review: Review = reviewer
            .review(ReviewRequest::new(goal, evidence.clone()))
            .await?;
        let verdict = review.verdict;

        rounds.push(GoalRound {
            round,
            evidence: evidence.clone(),
            verdict,
            notes: review.notes.clone(),
        });

        if verdict == Verdict::Pass {
            return Ok(GoalOutcome {
                stop_reason: GoalStopReason::Passed,
                rounds,
            });
        }

        previous_evidence = Some(evidence);
        task = format!(
            "{goal}\n\nA reviewer looked at your last attempt and said {}:\n{}\n\nAddress this and try again.",
            verdict.as_str(),
            review.notes
        )
        .into();
    }

    Ok(GoalOutcome {
        stop_reason: GoalStopReason::RoundCapReached,
        rounds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    /// A coder that returns each entry in `replies` in order, then repeats
    /// the last one for any further round — enough to script a fixed round
    /// count or a stuck coder without a real provider.
    struct ScriptedCoder {
        replies: Vec<SmolStr>,
        tasks_seen: Mutex<Vec<SmolStr>>,
    }

    impl ScriptedCoder {
        fn new(replies: Vec<&str>) -> Self {
            Self {
                replies: replies.into_iter().map(SmolStr::from).collect(),
                tasks_seen: Mutex::new(Vec::new()),
            }
        }

        fn tasks(&self) -> Vec<SmolStr> {
            self.tasks_seen.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl AgentRunner for ScriptedCoder {
        async fn run(
            &self,
            request: AgentRequest,
            _context: AgentContext,
        ) -> Result<SmolStr, SmolStr> {
            self.tasks_seen.lock().push(request.task.clone());
            let index = self.tasks_seen.lock().len() - 1;
            let index = index.min(self.replies.len() - 1);
            Ok(self.replies[index].clone())
        }
    }

    /// A coder whose every run fails, to test error propagation.
    struct FailingCoder;

    #[async_trait::async_trait]
    impl AgentRunner for FailingCoder {
        async fn run(
            &self,
            _request: AgentRequest,
            _context: AgentContext,
        ) -> Result<SmolStr, SmolStr> {
            Err("provider unavailable".into())
        }
    }

    /// A reviewer that returns each verdict in `verdicts` in order, then
    /// repeats the last one. Counts calls so oscillation tests can prove a
    /// second review never happened.
    struct ScriptedReviewer {
        verdicts: Vec<Verdict>,
        calls: Mutex<usize>,
    }

    impl ScriptedReviewer {
        fn new(verdicts: Vec<Verdict>) -> Self {
            Self {
                verdicts,
                calls: Mutex::new(0),
            }
        }

        fn call_count(&self) -> usize {
            *self.calls.lock()
        }
    }

    #[async_trait::async_trait]
    impl Reviewer for ScriptedReviewer {
        async fn review(&self, _request: ReviewRequest) -> Result<Review, SmolStr> {
            let mut calls = self.calls.lock();
            let index = (*calls).min(self.verdicts.len() - 1);
            *calls += 1;
            Ok(Review {
                verdict: self.verdicts[index],
                notes: format!("round {} notes", index + 1).into(),
            })
        }
    }

    #[tokio::test]
    async fn a_pass_on_the_first_round_stops_immediately() {
        let coder = ScriptedCoder::new(vec!["fixed the bug"]);
        let reviewer = ScriptedReviewer::new(vec![Verdict::Pass]);

        let outcome = run_goal_loop(&coder, &reviewer, "fix the bug", 5)
            .await
            .unwrap();

        assert_eq!(outcome.stop_reason, GoalStopReason::Passed);
        assert_eq!(outcome.rounds.len(), 1);
        assert_eq!(outcome.verdict(), Verdict::Pass);
        assert_eq!(outcome.exit_code(), 0);
    }

    #[tokio::test]
    async fn a_fail_that_keeps_changing_hits_the_round_cap() {
        let coder = ScriptedCoder::new(vec!["attempt 1", "attempt 2", "attempt 3"]);
        let reviewer = ScriptedReviewer::new(vec![Verdict::Fail]);

        let outcome = run_goal_loop(&coder, &reviewer, "fix the bug", 3)
            .await
            .unwrap();

        assert_eq!(outcome.stop_reason, GoalStopReason::RoundCapReached);
        assert_eq!(outcome.rounds.len(), 3);
        assert_eq!(outcome.exit_code(), Verdict::Fail.exit_code());
    }

    #[tokio::test]
    async fn identical_evidence_two_rounds_running_stops_as_oscillating() {
        let coder = ScriptedCoder::new(vec!["same attempt every time"]);
        let reviewer = ScriptedReviewer::new(vec![Verdict::Fail]);

        let outcome = run_goal_loop(&coder, &reviewer, "fix the bug", 10)
            .await
            .unwrap();

        assert_eq!(outcome.stop_reason, GoalStopReason::Oscillating);
        // Round 1 got a real review; round 2 repeated round 1's evidence and
        // was never sent to the reviewer at all.
        assert_eq!(outcome.rounds.len(), 1);
        assert_eq!(reviewer.call_count(), 1);
        assert_eq!(outcome.exit_code(), Verdict::Fail.exit_code());
    }

    #[tokio::test]
    async fn a_recovery_after_a_fail_still_passes() {
        let coder = ScriptedCoder::new(vec!["broken attempt", "fixed attempt"]);
        let reviewer = ScriptedReviewer::new(vec![Verdict::Fail, Verdict::Pass]);

        let outcome = run_goal_loop(&coder, &reviewer, "fix the bug", 5)
            .await
            .unwrap();

        assert_eq!(outcome.stop_reason, GoalStopReason::Passed);
        assert_eq!(outcome.rounds.len(), 2);
        assert_eq!(outcome.rounds[0].verdict, Verdict::Fail);
        assert_eq!(outcome.rounds[1].verdict, Verdict::Pass);
    }

    #[tokio::test]
    async fn a_max_rounds_of_zero_still_runs_one_round() {
        let coder = ScriptedCoder::new(vec!["attempt"]);
        let reviewer = ScriptedReviewer::new(vec![Verdict::Fail]);

        let outcome = run_goal_loop(&coder, &reviewer, "fix the bug", 0)
            .await
            .unwrap();

        assert_eq!(outcome.rounds.len(), 1);
        assert_eq!(outcome.stop_reason, GoalStopReason::RoundCapReached);
    }

    #[tokio::test]
    async fn a_coder_error_stops_the_loop_instead_of_reviewing_nothing() {
        let coder = FailingCoder;
        let reviewer = ScriptedReviewer::new(vec![Verdict::Pass]);

        let error = run_goal_loop(&coder, &reviewer, "fix the bug", 3)
            .await
            .unwrap_err();

        assert_eq!(error.as_str(), "provider unavailable");
        assert_eq!(reviewer.call_count(), 0);
    }

    #[tokio::test]
    async fn each_round_after_a_fail_carries_the_reviewers_notes_forward() {
        let coder = ScriptedCoder::new(vec!["broken attempt", "fixed attempt"]);
        let reviewer = ScriptedReviewer::new(vec![Verdict::Fail, Verdict::Pass]);

        run_goal_loop(&coder, &reviewer, "fix the bug", 5)
            .await
            .unwrap();

        let tasks = coder.tasks();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].as_str(), "fix the bug");
        assert!(tasks[1].contains("round 1 notes"));
        assert!(tasks[1].contains("fix the bug"));
    }
}
