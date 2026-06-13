//! A proposer includes the Candidate(phase-1) state and Leader(phase-2) state.

pub(crate) mod candidate;
pub(crate) mod leader;
pub(crate) mod leader_state;
pub(crate) mod pre_candidate;

pub(crate) use candidate::Candidate;
pub(crate) use leader::Leader;
pub(crate) use leader_state::CandidateState;
pub(crate) use leader_state::LeaderQuorumSet;
pub(crate) use leader_state::LeaderState;
#[allow(unused_imports)]
pub(crate) use pre_candidate::PreCandidate;
#[allow(unused_imports)]
pub(crate) use pre_candidate::PreVoteResult;
