/// All possible states of a Raft node.
#[derive(Debug, Clone, Copy, Default)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum ServerState {
    /// The node is completely passive; replicating entries, but neither voting nor timing out.
    #[default]
    Learner,
    /// The node is replicating logs from the leader.
    Follower,
    /// The node timed out the leader and is probing peers with `PreVoteRequest`,
    /// without incrementing its term (W3.3, ADR-012).
    ///
    /// A node enters this state instead of [`Candidate`](Self::Candidate) when pre-vote is
    /// enabled. It only advances to [`Candidate`](Self::Candidate) (and bumps its term) after a
    /// quorum of peers pre-grant the prospective vote. On rejection it reverts to
    /// [`Follower`](Self::Follower) without ever advancing its term, preventing the runaway-term
    /// election storm a stale node would otherwise cause.
    PreCandidate,
    /// The node is campaigning to become the cluster leader.
    Candidate,
    /// The node is the Raft cluster leader.
    Leader,
    /// The Raft node is shutting down.
    Shutdown,
}

impl ServerState {
    /// Check if currently in learner state.
    pub fn is_learner(&self) -> bool {
        matches!(self, Self::Learner)
    }

    /// Check if currently in follower state.
    pub fn is_follower(&self) -> bool {
        matches!(self, Self::Follower)
    }

    /// Check if currently in pre-candidate state (probing peers with pre-vote).
    pub fn is_pre_candidate(&self) -> bool {
        matches!(self, Self::PreCandidate)
    }

    /// Check if currently in candidate state.
    pub fn is_candidate(&self) -> bool {
        matches!(self, Self::Candidate)
    }

    /// Check if currently in leader state.
    pub fn is_leader(&self) -> bool {
        matches!(self, Self::Leader)
    }
}
