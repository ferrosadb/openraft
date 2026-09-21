use crate::CommittedLeaderId;
use crate::LogId;
use crate::RaftState;
use crate::TokioInstant;

fn log_id(term: u64, index: u64) -> LogId<u64> {
    LogId::<u64> {
        leader_id: CommittedLeaderId::new(term, 0),
        index,
    }
}

fn state_committed_at(term: u64, index: u64) -> RaftState<u64, (), TokioInstant> {
    RaftState::<u64, (), TokioInstant> {
        committed: Some(log_id(term, index)),
        ..Default::default()
    }
}

/// The ordinary case: a later term at a higher index advances committed.
#[test]
fn a_higher_term_at_a_higher_index_advances_committed() {
    let mut rs = state_committed_at(3, 17453);

    let prev = rs.update_committed(&Some(log_id(4, 17600)));

    assert_eq!(prev, Some(Some(log_id(3, 17453))), "the previous value is returned");
    assert_eq!(rs.committed, Some(log_id(4, 17600)));
}

/// The same term at a higher index advances committed.
#[test]
fn the_same_term_at_a_higher_index_advances_committed() {
    let mut rs = state_committed_at(3, 17453);

    let prev = rs.update_committed(&Some(log_id(3, 17454)));

    assert_eq!(prev, Some(Some(log_id(3, 17453))));
    assert_eq!(rs.committed, Some(log_id(3, 17454)));
}

/// The regression. `LogId`'s `Ord` is lexicographic on `(leader_id, index)`
/// with `leader_id` first, so a **higher term at a lower index** compares
/// greater and was accepted — moving the committed *index* backwards.
///
/// `Command::Commit` then derives the apply window from the pair:
///
/// ```text
/// apply_to_state_machine(seq, already_committed.next_index(), upto.index)
/// ```
///
/// giving `since = 17515`, `end = 17454` — inverted. `defensive.rs` classifies
/// an inverted range as *empty* and returns `Ok`, so `raft_core.rs:769` then
/// indexes `entries[entries.len() - 1]` on an empty `Vec`: `0usize - 1` wraps
/// to `usize::MAX` and the consensus runtime panics.
///
/// This is the state ferrosa's node1 restarted into on 2026-09-19 and never
/// left, logging `reversed Raft log range: start=17515, end=17454` on every
/// apply tick for two days.
///
/// The committed index must never regress.
#[test]
fn a_higher_term_at_a_lower_index_must_not_move_committed_backwards() {
    let mut rs = state_committed_at(3, 17514);

    let prev = rs.update_committed(&Some(log_id(4, 17453)));

    assert_eq!(
        prev, None,
        "a committed index regression must not be accepted, however high its term"
    );
    assert_eq!(
        rs.committed,
        Some(log_id(3, 17514)),
        "committed must be left untouched by a rejected regression"
    );
}

/// The same index under a higher term is not a regression — the index does not
/// move — so it is still accepted and re-commits at the newer leader.
#[test]
fn a_higher_term_at_the_same_index_is_accepted() {
    let mut rs = state_committed_at(3, 17453);

    let prev = rs.update_committed(&Some(log_id(4, 17453)));

    assert_eq!(prev, Some(Some(log_id(3, 17453))));
    assert_eq!(rs.committed, Some(log_id(4, 17453)));
}

/// A lower term is already rejected by the existing `>` comparison, whatever
/// its index. Pinned so the new guard does not change it.
#[test]
fn a_lower_term_is_still_rejected() {
    let mut rs = state_committed_at(3, 17453);

    assert_eq!(rs.update_committed(&Some(log_id(2, 17600))), None);
    assert_eq!(rs.committed, Some(log_id(3, 17453)));

    assert_eq!(rs.update_committed(&Some(log_id(2, 17400))), None);
    assert_eq!(rs.committed, Some(log_id(3, 17453)));
}

/// The first commit, from no committed state, is accepted at any index.
#[test]
fn the_first_commit_is_accepted() {
    let mut rs = RaftState::<u64, (), TokioInstant>::default();

    let prev = rs.update_committed(&Some(log_id(1, 5)));

    assert_eq!(prev, Some(None));
    assert_eq!(rs.committed, Some(log_id(1, 5)));
}

/// The property the guard exists for: whatever `update_committed` accepts, the
/// apply window `[prev.index + 1, new.index + 1)` that `Command::Commit`
/// derives from it must never be inverted.
#[test]
fn no_accepted_commit_can_produce_an_inverted_apply_window() {
    for cur_term in 1u64..6 {
        for cur_index in 0u64..40 {
            for new_term in 1u64..6 {
                for new_index in 0u64..40 {
                    let mut rs = state_committed_at(cur_term, cur_index);

                    if rs.update_committed(&Some(log_id(new_term, new_index))).is_some() {
                        let since = cur_index + 1;
                        let end = new_index + 1;
                        assert!(
                            since <= end,
                            "accepted committed {new_term}/{new_index} over \
                             {cur_term}/{cur_index} yields the inverted apply \
                             window [{since}, {end})"
                        );
                    }
                }
            }
        }
    }
}
