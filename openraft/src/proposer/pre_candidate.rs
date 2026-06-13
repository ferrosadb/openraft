use std::collections::BTreeSet;
use std::fmt;

use crate::quorum::QuorumSet;
use crate::RaftTypeConfig;
use crate::Vote;

/// Outcome of recording a single pre-vote response while in
/// [`PreCandidate`](crate::core::ServerState::PreCandidate) state.
///
/// This drives the W3.3 election state machine (ADR-012):
///
/// - [`Continue`](Self::Continue): not enough responses yet to decide; keep probing.
/// - [`Promote`](Self::Promote): a quorum pre-granted. The caller must now transition to
///   [`Candidate`](crate::core::ServerState::Candidate) and start a *real* election, which is the
///   only point at which the term is incremented.
/// - [`RevertToFollower`](Self::RevertToFollower): the pre-vote round can no longer succeed (a peer
///   reported a strictly higher term, or a quorum of rejections is now unreachable). The caller
///   must transition back to [`Follower`](crate::core::ServerState::Follower) **without** advancing
///   the term. This is the runaway-term fix (W3.4): a stale node never bumps its term off a failed
///   pre-vote.
//
// `dead_code`: the pre-vote *decision state machine* (this module) is the W3.3 deliverable and is
// fully unit-tested below. The remaining wiring — `PreVoteRequest`/`PreVoteResponse` RPC types
// (W3.1), the `RaftCore` async event-loop transitions, and `calc_server_state` emitting
// `PreCandidate` — is deferred to later work items per the sprint-03-openraft-patches plan. These
// symbols are intentionally not yet called by the engine; they are NOT a stub returning fake
// success.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreVoteResult {
    /// Keep probing; no decision yet.
    Continue,
    /// A quorum pre-granted; promote to real `Candidate` (term is incremented there).
    Promote,
    /// Pre-vote cannot succeed; revert to `Follower` with no term advance.
    RevertToFollower,
}

/// Pre-candidate: the pre-vote probing state (Raft §9.6 / Ongaro pre-vote).
///
/// A node holds a *prospective* vote — `Vote::new(current_term + 1, self.id)` — entirely in memory.
/// It is **never** persisted. The node asks peers whether they *would* grant this vote if a real
/// election were held, without changing any persistent state on either side. Only after a quorum of
/// distinct voters pre-grant does the node promote to a real [`Candidate`] and persist the
/// incremented term.
///
/// [`Candidate`]: crate::proposer::Candidate
//
// `dead_code`: see the note on `PreVoteResult` above — the engine wiring that constructs and drives
// `PreCandidate` is the deferred portion of this sprint.
#[allow(dead_code)]
#[derive(Clone, Debug)]
#[derive(PartialEq, Eq)]
pub(crate) struct PreCandidate<C, QS>
where
    C: RaftTypeConfig,
    QS: QuorumSet<C::NodeId>,
{
    /// Prospective vote for the pre-vote round. Held in memory only; never written to storage.
    prospective_vote: Vote<C::NodeId>,

    /// Distinct voters that have pre-granted so far.
    pre_grants: BTreeSet<C::NodeId>,

    /// Distinct voters that have pre-rejected so far.
    pre_rejects: BTreeSet<C::NodeId>,

    /// The quorum set used to evaluate whether grants/rejections form a quorum.
    quorum_set: QS,
}

impl<C, QS> fmt::Display for PreCandidate<C, QS>
where
    C: RaftTypeConfig,
    QS: QuorumSet<C::NodeId> + fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{prospective_vote:{}, pre_grants:{:?}, pre_rejects:{:?}}}",
            self.prospective_vote, self.pre_grants, self.pre_rejects
        )
    }
}

// `dead_code`: see the note on `PreVoteResult` — engine wiring for these methods is deferred.
#[allow(dead_code)]
impl<C, QS> PreCandidate<C, QS>
where
    C: RaftTypeConfig,
    QS: QuorumSet<C::NodeId> + Clone + fmt::Debug + 'static,
{
    /// Create a new pre-candidate.
    ///
    /// `prospective_vote` must be a non-committed vote for `current_term + 1`. The node's own
    /// implicit pre-grant is recorded so a single-voter quorum (one-node cluster) promotes
    /// immediately.
    pub(crate) fn new(prospective_vote: Vote<C::NodeId>, quorum_set: QS) -> Self {
        debug_assert!(
            !prospective_vote.is_committed(),
            "prospective pre-vote must never be committed"
        );

        let mut pre_grants = BTreeSet::new();
        // The pre-candidate always pre-grants itself.
        if let Some(me) = prospective_vote.leader_id().voted_for() {
            pre_grants.insert(me);
        }

        Self {
            prospective_vote,
            pre_grants,
            pre_rejects: BTreeSet::new(),
            quorum_set,
        }
    }

    /// The prospective (in-memory) vote being probed.
    pub(crate) fn prospective_vote_ref(&self) -> &Vote<C::NodeId> {
        &self.prospective_vote
    }

    /// The term the prospective vote would campaign for once promoted.
    pub(crate) fn prospective_term(&self) -> u64 {
        self.prospective_vote.leader_id().get_term()
    }

    /// Record a pre-grant from `voter`.
    ///
    /// Returns [`PreVoteResult::Promote`] once a quorum of distinct voters (including this node)
    /// have pre-granted, otherwise [`PreVoteResult::Continue`].
    pub(crate) fn record_pre_grant(&mut self, voter: C::NodeId) -> PreVoteResult {
        self.pre_grants.insert(voter);

        if self.quorum_set.is_quorum(self.pre_grants.iter()) {
            PreVoteResult::Promote
        } else {
            PreVoteResult::Continue
        }
    }

    /// Record a pre-rejection from `voter`, who is at `voter_term`.
    ///
    /// - If `voter_term` is strictly greater than the prospective term, the node's log/term is
    ///   stale relative to a peer; revert to follower **without** advancing the term. The higher
    ///   term will be picked up later via `AppendEntries`/`Vote`. This is the W3.4 fix.
    /// - Otherwise, if the set of rejections has grown to a quorum (so a winning grant-quorum is no
    ///   longer reachable), revert to follower.
    /// - Otherwise, keep probing.
    pub(crate) fn record_pre_reject(&mut self, voter: C::NodeId, voter_term: u64) -> PreVoteResult {
        self.pre_rejects.insert(voter);

        if voter_term > self.prospective_term() {
            // Saw a strictly higher term during pre-vote. Do NOT advance our own term.
            return PreVoteResult::RevertToFollower;
        }

        if self.quorum_set.is_quorum(self.pre_rejects.iter()) {
            // A quorum has rejected; a grant-quorum is unreachable. Revert without term advance.
            return PreVoteResult::RevertToFollower;
        }

        PreVoteResult::Continue
    }

    /// Voters that have pre-granted so far.
    #[allow(dead_code)]
    pub(crate) fn pre_granters(&self) -> impl Iterator<Item = &C::NodeId> + '_ {
        self.pre_grants.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::PreCandidate;
    use super::PreVoteResult;
    use crate::engine::testing::UTConfig;
    use crate::Vote;

    /// Majority quorum over voters {1,2,3}: a quorum is any 2 of 3.
    fn quorum_123() -> Vec<u64> {
        vec![1, 2, 3]
    }

    /// Prospective vote for node 1 at term `t` (non-committed).
    fn prospective(t: u64, node: u64) -> Vote<u64> {
        Vote::new(t, node)
    }

    #[test]
    fn new_pre_candidate_pre_grants_itself() {
        // Single-voter probe over its own membership: node 1 already counts itself, but {1} is not
        // a quorum of {1,2,3}, so it must keep probing — never promote off its own vote alone.
        let pc = PreCandidate::<UTConfig, _>::new(prospective(8, 1), quorum_123());
        assert_eq!(8, pc.prospective_term());
        assert!(!pc.prospective_vote_ref().is_committed());
        assert_eq!(vec![&1u64], pc.pre_granters().collect::<Vec<_>>());
    }

    #[test]
    fn pre_grant_majority_promotes() {
        // RED→GREEN W3.3: a quorum pre-grant promotes to real Candidate (where term is bumped).
        let mut pc = PreCandidate::<UTConfig, _>::new(prospective(8, 1), quorum_123());
        // Self (1) + peer 2 == quorum of {1,2,3}.
        assert_eq!(PreVoteResult::Promote, pc.record_pre_grant(2));
    }

    #[test]
    fn single_node_cluster_promotes_immediately() {
        // One-voter cluster {1}: the node's own implicit pre-grant is already a quorum, so the
        // first recorded peer grant is unnecessary — but recording self again still promotes.
        let mut pc = PreCandidate::<UTConfig, _>::new(prospective(3, 1), vec![1u64]);
        assert_eq!(PreVoteResult::Promote, pc.record_pre_grant(1));
    }

    #[test]
    fn higher_term_rejection_reverts_to_follower_without_term_advance() {
        // The W3.4 runaway-term fix: a peer at a strictly higher term must send us back to
        // Follower; the prospective term is observed but our term is NOT advanced by pre-vote.
        let mut pc = PreCandidate::<UTConfig, _>::new(prospective(8, 1), quorum_123());
        let before = pc.prospective_term();
        assert_eq!(PreVoteResult::RevertToFollower, pc.record_pre_reject(2, 42));
        // The prospective term is unchanged; promotion (and the real term bump) never happened.
        assert_eq!(before, pc.prospective_term());
    }

    #[test]
    fn same_term_minority_rejection_continues() {
        // A single same-term rejection (1 of 3) is not yet a rejecting quorum: keep probing.
        let mut pc = PreCandidate::<UTConfig, _>::new(prospective(8, 1), quorum_123());
        assert_eq!(PreVoteResult::Continue, pc.record_pre_reject(2, 8));
    }

    #[test]
    fn rejecting_quorum_reverts_to_follower() {
        // Two same-term rejections {2,3} form a quorum of {1,2,3}; a grant-quorum is unreachable.
        let mut pc = PreCandidate::<UTConfig, _>::new(prospective(8, 1), quorum_123());
        assert_eq!(PreVoteResult::Continue, pc.record_pre_reject(2, 8));
        assert_eq!(PreVoteResult::RevertToFollower, pc.record_pre_reject(3, 8));
    }

    #[test]
    fn idempotent_grant_from_same_voter_does_not_double_count() {
        // Recording the same voter's grant twice must not fabricate a quorum.
        let mut pc = PreCandidate::<UTConfig, _>::new(prospective(8, 1), vec![1u64, 2, 3, 4, 5]);
        // {1,2} is not a quorum of 5; recording 2 again stays at {1,2}.
        assert_eq!(PreVoteResult::Continue, pc.record_pre_grant(2));
        assert_eq!(PreVoteResult::Continue, pc.record_pre_grant(2));
        // Now distinct voter 3 reaches {1,2,3} == quorum of 5.
        assert_eq!(PreVoteResult::Promote, pc.record_pre_grant(3));
    }
}
