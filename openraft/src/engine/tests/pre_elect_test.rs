use std::sync::Arc;

use maplit::btreeset;
use pretty_assertions::assert_eq;

use crate::core::ServerState;
use crate::engine::testing::UTConfig;
use crate::engine::Command;
use crate::engine::Engine;
use crate::engine::LogIdList;
use crate::raft::VoteRequest;
use crate::raft::VoteResponse;
use crate::testing::log_id;
use crate::CommittedLeaderId;
use crate::EffectiveMembership;
use crate::LogId;
use crate::Membership;
use crate::Vote;

fn m1() -> Membership<u64, ()> {
    Membership::new(vec![btreeset! {1}], None)
}

fn m123() -> Membership<u64, ()> {
    Membership::new(vec![btreeset! {1,2,3}], None)
}

fn eng() -> Engine<UTConfig> {
    let mut eng = Engine::default();
    eng.config.id = 1;
    eng.config.enable_pre_vote = true;
    eng.state.log_ids = LogIdList::new([LogId::new(CommittedLeaderId::new(0, 0), 0)]);
    eng.state.enable_validation(false); // Disable validation for incomplete state
    eng
}

/// W3.3: on election timeout with pre-vote enabled, a multi-voter node enters `PreCandidate`,
/// probes peers with a prospective (non-committed) vote, and does NOT advance its persistent term
/// or persist any vote.
#[test]
fn pre_elect_multi_node_enters_pre_candidate_without_term_advance() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(0, 1, 1)), m123())));
    eng.state.log_ids = LogIdList::new(vec![log_id(1, 1, 1)]);

    let term_before = eng.state.vote_ref().leader_id().get_term();

    eng.pre_elect();

    // No persistent state change: term is unchanged and no committed/persisted vote.
    assert_eq!(
        term_before,
        eng.state.vote_ref().leader_id().get_term(),
        "pre-vote must not advance the persistent term"
    );
    assert!(eng.candidate_ref().is_none(), "no real candidate during pre-vote");
    assert!(eng.leader.is_none());
    assert_eq!(ServerState::PreCandidate, eng.state.server_state);

    // The prospective vote campaigns for term+1 and is broadcast as a SendPreVote, never a SaveVote.
    let cmds = eng.output.take_commands();
    assert_eq!(
        vec![Command::SendPreVote {
            vote_req: VoteRequest::new(Vote::new(term_before + 1, 1), Some(log_id(1, 1, 1))),
        }],
        cmds,
        "pre-vote emits exactly one SendPreVote and NO SaveVote"
    );

    Ok(())
}

/// W3.3: a single-voter cluster pre-grants itself a quorum immediately and promotes straight to a
/// real `Candidate` (which bumps the term and persists the vote).
#[test]
fn pre_elect_single_node_promotes_to_candidate() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(0, 1, 1)), m1())));

    let term_before = eng.state.vote_ref().leader_id().get_term();

    eng.pre_elect();

    // Self pre-grant is already a quorum of {1}: promote to real Candidate, bumping the term.
    assert_eq!(Vote::new(term_before + 1, 1), *eng.state.vote_ref());
    assert!(eng.candidate_ref().is_some(), "promoted to real candidate");
    assert_eq!(ServerState::Candidate, eng.state.server_state);

    let cmds = eng.output.take_commands();
    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new(term_before + 1, 1)
            },
            Command::SendVote {
                vote_req: VoteRequest::new(Vote::new(term_before + 1, 1), Some(log_id(0, 0, 0))),
            },
        ],
        cmds,
        "single-node pre-vote promotes immediately to a real election"
    );

    Ok(())
}

/// W3.3 / W3.4 runaway-term fix: when a peer pre-rejects at a strictly higher term, the
/// pre-candidate reverts to `Follower` WITHOUT advancing its persistent term.
#[test]
fn pre_vote_resp_higher_term_rejection_reverts_to_follower_no_term_advance() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(0, 1, 1)), m123())));
    eng.state.log_ids = LogIdList::new(vec![log_id(1, 1, 1)]);

    eng.pre_elect();
    let _ = eng.output.take_commands();
    let term_before = eng.state.vote_ref().leader_id().get_term();
    assert_eq!(ServerState::PreCandidate, eng.state.server_state);

    // Peer 2 rejects at a much higher term.
    eng.handle_pre_vote_resp(2, VoteResponse::new(Vote::new(42, 2), Some(log_id(1, 1, 1)), false));

    assert_eq!(
        term_before,
        eng.state.vote_ref().leader_id().get_term(),
        "higher-term pre-vote rejection must NOT advance our term"
    );
    assert!(eng.candidate_ref().is_none(), "must not become a real candidate");
    assert_eq!(ServerState::Follower, eng.state.server_state);

    Ok(())
}

/// W3.3: when a quorum of peers pre-grant, the pre-candidate promotes to a real `Candidate`,
/// bumping the term and persisting the vote exactly once at promotion time.
#[test]
fn pre_vote_resp_quorum_grant_promotes_to_candidate() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(0, 1, 1)), m123())));
    eng.state.log_ids = LogIdList::new(vec![log_id(1, 1, 1)]);

    eng.pre_elect();
    let _ = eng.output.take_commands();
    let prospective_term = eng.state.vote_ref().leader_id().get_term() + 1;

    // Peer 2 pre-grants for the prospective term: self {1} + {2} == quorum of {1,2,3}.
    eng.handle_pre_vote_resp(
        2,
        VoteResponse::new(Vote::new(prospective_term, 1), Some(log_id(1, 1, 1)), true),
    );

    assert_eq!(Vote::new(prospective_term, 1), *eng.state.vote_ref());
    assert!(eng.candidate_ref().is_some(), "promoted to real candidate");
    assert_eq!(ServerState::Candidate, eng.state.server_state);

    let cmds = eng.output.take_commands();
    assert_eq!(
        vec![
            Command::SaveVote {
                vote: Vote::new(prospective_term, 1)
            },
            Command::SendVote {
                vote_req: VoteRequest::new(Vote::new(prospective_term, 1), Some(log_id(1, 1, 1))),
            },
        ],
        cmds,
        "quorum pre-grant promotes to a real election (SaveVote + SendVote)"
    );

    Ok(())
}

/// W3.3: with pre-vote DISABLED, `pre_elect` falls through to the legacy direct `elect` path:
/// straight to `Candidate`, term bumped, vote persisted. This preserves backward compatibility.
#[test]
fn pre_elect_with_pre_vote_disabled_falls_back_to_direct_elect() -> anyhow::Result<()> {
    let mut eng = eng();
    eng.config.enable_pre_vote = false;
    eng.state
        .membership_state
        .set_effective(Arc::new(EffectiveMembership::new(Some(log_id(0, 1, 1)), m123())));
    eng.state.log_ids = LogIdList::new(vec![log_id(1, 1, 1)]);

    let term_before = eng.state.vote_ref().leader_id().get_term();

    eng.pre_elect();

    assert_eq!(Vote::new(term_before + 1, 1), *eng.state.vote_ref());
    assert!(eng.candidate_ref().is_some());
    assert_eq!(ServerState::Candidate, eng.state.server_state);

    Ok(())
}
