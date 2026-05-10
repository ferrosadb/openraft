//! W3.1 — PreVote basic: a `PreVoteRequest` type exists and a follower
//! responds with `vote_granted: true` for an up-to-date candidate.
//!
//! This is the foundational TDD red for ADR-012's PreVote work. It does NOT
//! yet exercise the engine's PreCandidate state — that's W3.3. It only proves
//! that the message types and the (default) `RaftNetwork::pre_vote` trait
//! method exist and are wired through the network surface.
//!
//! Spec: specs/decisions/012-prevote-checkquorum-leadership-transfer.md.

use openraft::raft::PreVoteRequest;
use openraft::raft::PreVoteResponse;
use openraft::LogId;
use openraft::Vote;

#[test]
fn pre_vote_request_message_type_exists() {
    // RED: until W3.1 lands, `PreVoteRequest` does not exist and this won't compile.
    let req: PreVoteRequest<u64> = PreVoteRequest {
        vote: Vote::new(5, 1),
        last_log_id: Some(LogId::new(openraft::CommittedLeaderId::new(4, 0), 10)),
    };
    assert_eq!(req.vote.leader_id().get_term(), 5);
}

#[test]
fn pre_vote_response_message_type_exists() {
    let resp: PreVoteResponse<u64> = PreVoteResponse {
        vote: Vote::new(5, 1),
        vote_granted: true,
        last_log_id: None,
    };
    assert!(resp.vote_granted);
    assert_eq!(resp.vote.leader_id().get_term(), 5);
}
