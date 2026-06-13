use std::sync::Arc;
use std::time::Duration;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::core::ServerState;
use crate::engine::testing::UTConfig;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::testing::log_id;
use crate::utime::UTime;
use crate::EffectiveMembership;
use crate::Membership;
use crate::TokioInstant;
use crate::Vote;

fn m01() -> Membership<u64, ()> {
    Membership::<u64, ()>::new(vec![btreeset! {0,1}], None)
}

/// Engine for a follower node (id=1) whose leader lease has expired by default, so a pre-vote can
/// be granted unless a test re-arms the lease.
fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::default();
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 1;
    // By default expire the leader lease so the pre-vote can be granted in these tests.
    eng.state.vote = UTime::new(TokioInstant::now() - Duration::from_millis(300), Vote::new(2, 1));
    eng.state.log_ids = LogIdList::new(vec![log_id(2, 1, 3)]);
    eng.state.server_state = ServerState::Follower;
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m01())));
    eng.output.take_commands();

    eng
}

/// A pre-vote must be a pure probe: it grants when the candidate's log is up-to-date AND no leader
/// lease is active, but it must NOT mutate persistent state — no vote change, no term advance, no
/// commands emitted.
#[test]
fn test_handle_pre_vote_req_granted_is_read_only() -> anyhow::Result<()> {
    let mut eng = eng();
    let vote_before = *eng.state.vote_ref();

    // Candidate at a strictly higher prospective term, log at least as up-to-date as ours.
    let resp = eng.handle_pre_vote_req(VoteRequest {
        vote: Vote::new(3, 0),
        last_log_id: Some(log_id(2, 1, 3)),
    });

    assert!(resp.vote_granted, "up-to-date log + expired lease => granted");
    // Read-only: our persisted vote and term are unchanged, and the response reports OUR vote.
    assert_eq!(
        vote_before,
        *eng.state.vote_ref(),
        "pre-vote must not change persisted vote"
    );
    assert_eq!(vote_before, resp.vote, "response carries our current (unchanged) vote");
    assert_eq!(Some(log_id(2, 1, 3)), resp.last_log_id);
    assert_eq!(
        0,
        eng.output.take_commands().len(),
        "pre-vote emits no state-changing commands"
    );
    assert_eq!(ServerState::Follower, eng.state.server_state);

    Ok(())
}

/// A node whose own log is strictly ahead of the candidate must reject the pre-vote (it would not
/// vote for a less-up-to-date candidate).
#[test]
fn test_handle_pre_vote_req_rejected_by_stale_candidate_log() -> anyhow::Result<()> {
    let mut eng = eng();
    let vote_before = *eng.state.vote_ref();

    let resp = eng.handle_pre_vote_req(VoteRequest {
        vote: Vote::new(3, 0),
        last_log_id: Some(log_id(2, 1, 2)), // behind our log_id(2,1,3)
    });

    assert!(!resp.vote_granted, "candidate log behind ours => rejected");
    assert_eq!(vote_before, *eng.state.vote_ref());
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

/// When a committed leader lease is still valid, a pre-vote must be rejected even if the
/// candidate's log is up-to-date — the node would not break a live leader.
#[test]
fn test_handle_pre_vote_req_rejected_by_leader_lease() -> anyhow::Result<()> {
    let mut eng = eng();
    // Arm a fresh committed vote so the leader lease is currently valid.
    eng.state.vote.update(TokioInstant::now(), Vote::new_committed(2, 1));
    let vote_before = *eng.state.vote_ref();

    let resp = eng.handle_pre_vote_req(VoteRequest {
        vote: Vote::new(3, 0),
        last_log_id: Some(log_id(2, 1, 3)),
    });

    assert!(!resp.vote_granted, "valid leader lease => pre-vote rejected");
    assert_eq!(
        vote_before,
        *eng.state.vote_ref(),
        "pre-vote must not change persisted vote"
    );
    assert_eq!(VoteResponse::new(vote_before, Some(log_id(2, 1, 3)), false), resp);
    assert_eq!(0, eng.output.take_commands().len());

    Ok(())
}

/// A higher prospective term in the pre-vote request must NOT advance our term or be persisted.
/// This is the responder side of the W3.4 runaway-term protection.
#[test]
fn test_handle_pre_vote_req_higher_term_does_not_advance_our_term() -> anyhow::Result<()> {
    let mut eng = eng();
    let vote_before = *eng.state.vote_ref();

    let _resp = eng.handle_pre_vote_req(VoteRequest {
        vote: Vote::new(999, 0),
        last_log_id: Some(log_id(2, 1, 3)),
    });

    assert_eq!(
        vote_before,
        *eng.state.vote_ref(),
        "huge prospective term must not bump our term"
    );
    assert_eq!(2, eng.state.vote_ref().leader_id().get_term());
    assert_eq!(
        0,
        eng.output.take_commands().len(),
        "pre-vote emits no state-changing commands"
    );

    Ok(())
}
