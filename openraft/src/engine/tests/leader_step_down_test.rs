use std::sync::Arc;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::core::ServerState;
use crate::engine::testing::UTConfig;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::raft::VoteRequest;
use crate::testing::log_id;
use crate::utime::UTime;
use crate::EffectiveMembership;
use crate::Membership;
use crate::TokioInstant;
use crate::Vote;

fn m0() -> Membership<u64, ()> {
    // Membership that does NOT contain this node (id=1): committing it is what makes a stepped-down
    // leader actually relinquish leadership.
    Membership::<u64, ()>::new(vec![btreeset! {0}], None)
}

/// Engine for a node (id=1) that holds a *fresh committed* vote — i.e. its leader lease is
/// currently valid, which is the precondition that, post-step-down, must no longer block pre-votes.
fn eng_leader() -> Engine<UTConfig> {
    let mut eng = Engine::default();
    eng.state.enable_validation(false); // Disable validation for incomplete state

    eng.config.id = 1;
    // Fresh committed vote => leader lease is valid right now.
    eng.state.vote = UTime::new(TokioInstant::now(), Vote::new_committed(2, 1));
    eng.state.log_ids = LogIdList::new(vec![log_id(2, 1, 3)]);
    eng.state.committed = Some(log_id(1, 1, 1));
    eng.state.server_state = ServerState::Leader;
    // Effective membership excludes this node and its log id is already committed, so step-down
    // proceeds to actually relinquish leadership.
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(1, 1, 1)), m0())));
    eng.output.take_commands();

    eng
}

/// A leader's own fresh committed vote keeps its leader lease valid, so before stepping down it
/// would reject a successor's pre-vote. This documents the precondition for the bug the wiring
/// fixes.
#[test]
fn test_leader_lease_blocks_pre_vote_before_step_down() -> anyhow::Result<()> {
    let eng = eng_leader();

    assert!(eng.leader_lease_is_valid(), "fresh committed vote => lease valid");

    let resp = eng.handle_pre_vote_req(VoteRequest {
        vote: Vote::new(3, 0),
        last_log_id: Some(log_id(2, 1, 3)),
    });
    assert!(!resp.vote_granted, "valid leader lease blocks the pre-vote");

    Ok(())
}

/// After `leader_step_down`, the (now former) leader must invalidate its own lease so it stops
/// blocking pre-votes for a successor — even though its committed vote utime is still recent.
/// This is the W3.7 leader_step_down <-> LeaderLease::invalidate wiring.
#[test]
fn test_leader_step_down_invalidates_lease_and_unblocks_pre_vote() -> anyhow::Result<()> {
    let mut eng = eng_leader();
    let vote_before = *eng.state.vote_ref();

    eng.leader_step_down();

    // The lease must be invalidated even though the committed vote's utime is still within the lease
    // window.
    assert!(
        !eng.leader_lease_is_valid(),
        "stepped-down leader must invalidate its own lease"
    );

    let resp = eng.handle_pre_vote_req(VoteRequest {
        vote: Vote::new(3, 0),
        last_log_id: Some(log_id(2, 1, 3)),
    });
    assert!(
        resp.vote_granted,
        "after step-down the invalidated lease no longer blocks pre-votes"
    );

    // Step-down + pre-vote are still read-only w.r.t. the persisted vote term: the pre-vote probe
    // must not advance our term.
    assert_eq!(
        vote_before.leader_id().get_term(),
        eng.state.vote_ref().leader_id().get_term(),
        "lease invalidation must not bump the persisted term"
    );

    Ok(())
}

/// Acknowledging a *new* leader after a step-down (an inbound vote that updates our vote) must
/// re-establish a fresh, valid lease: a stale invalidation must not permanently suppress lease
/// protection for the next leader.
#[test]
fn test_new_committed_vote_after_step_down_re_arms_lease() -> anyhow::Result<()> {
    let mut eng = eng_leader();

    eng.leader_step_down();
    assert!(!eng.leader_lease_is_valid(), "lease invalidated by step-down");

    // A new leader is acknowledged: our vote is updated to a fresh committed vote.
    eng.state.vote.update(TokioInstant::now(), Vote::new_committed(3, 0));

    assert!(
        eng.leader_lease_is_valid(),
        "a fresh committed vote acknowledging a new leader re-arms the lease"
    );

    Ok(())
}
