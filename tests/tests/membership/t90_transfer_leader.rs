//! Tests for `Trigger::transfer_to` (ferrosa fork — ADR-012, W3.9, W3.10).
//!
//! Leadership transfer (Ongaro Sec 3.10): the current leader catches up the target,
//! sends a `TimeoutNow` directive, and waits for the target to win an election.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use maplit::btreeset;
use openraft::error::TransferError;
use openraft::Config;
use openraft::ServerState;

use crate::fixtures::init_default_ut_tracing;
use crate::fixtures::Direction;
use crate::fixtures::RPCErrorType;
use crate::fixtures::RaftRouter;

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn transfer_to_makes_target_leader() -> Result<()> {
    // W3.9 — happy path: leader transfers to a healthy voter; target becomes leader.
    let config = Arc::new(
        Config {
            enable_tick: true,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing 3-node cluster");
    let _ = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    // Identify the current leader.
    let leader_id = router.leader().expect("expected a leader after init");
    tracing::info!(leader_id, "--- current leader identified");

    // Pick a target that is not the leader.
    let target: u64 = if leader_id == 0 { 1 } else { 0 };
    tracing::info!(target, "--- transferring leadership to target");

    // Trigger transfer.
    let leader_handle = router.get_raft_handle(&leader_id)?;
    let res = leader_handle.trigger().transfer_to(target).await;
    assert!(
        res.is_ok(),
        "transfer_to should succeed for a caught-up voter; got {:?}",
        res
    );

    // After transfer, target should be the new leader.
    router
        .wait_for_metrics(
            &target,
            |m| m.current_leader == Some(target) && m.state == ServerState::Leader,
            Some(Duration::from_secs(3)),
            "target becomes leader after transfer",
        )
        .await?;

    Ok(())
}

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn transfer_to_returns_timeout_if_target_does_not_win() -> Result<()> {
    // W3.10 — timeout safety: if the target cannot reach the other voters
    // (e.g., its outgoing RPCs all fail), it cannot win an election. The
    // leader's `transfer_to` must return `TransferError::Timeout` within
    // `election_timeout_max × 2` and the original leader must remain leader.
    let config = Arc::new(
        Config {
            enable_tick: true,
            // Keep election timeout small so the test runs quickly.
            election_timeout_min: 200,
            election_timeout_max: 400,
            heartbeat_interval: 50,
            ..Default::default()
        }
        .validate()?,
    );
    let mut router = RaftRouter::new(config.clone());

    tracing::info!("--- initializing 3-node cluster");
    let _ = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    let leader_id = router.leader().expect("expected a leader after init");
    let target: u64 = if leader_id == 0 { 1 } else { 0 };
    let third: u64 = match (leader_id, target) {
        (0, 1) | (1, 0) => 2,
        (0, 2) | (2, 0) => 1,
        (1, 2) | (2, 1) => 0,
        _ => unreachable!(),
    };

    // Poison only the target's OUTGOING RPCs. The leader's TimeoutNow
    // RPC (target receives it) still lands, but the target's VoteRequests to
    // peers fail — so it can never win an election.
    router.set_rpc_failure(target, Direction::NetSend, Some(RPCErrorType::NetworkError));
    let _ = third;

    let leader_handle = router.get_raft_handle(&leader_id)?;

    // Run the transfer; bind trigger() to a let to keep its borrow alive.
    let target_clone = target;
    let trigger = leader_handle.trigger();
    let transfer_fut = trigger.transfer_to(target_clone);
    // Give it room: catchup_deadline + dispatch + 2*election_timeout_max
    // ≈ 800ms catchup + dispatch + 800ms watch = ~2s. Use a generous 5s.
    let res = tokio::time::timeout(Duration::from_secs(5), transfer_fut).await;

    match res {
        Ok(Err(TransferError::Timeout)) => {
            tracing::info!("transfer_to correctly returned Timeout");
        }
        Ok(other) => panic!(
            "expected TransferError::Timeout (or comparable failure); got {:?}",
            other
        ),
        Err(_) => panic!("test deadline exceeded — transfer_to did not return at all"),
    }

    // Heal the target's network so it stops being isolated for cleanup.
    router.set_rpc_failure(target, Direction::NetSend, None);

    Ok(())
}

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn transfer_to_rejects_when_not_leader() -> Result<()> {
    // Sanity: a follower cannot initiate transfer_to.
    let config = Arc::new(Config { ..Default::default() }.validate()?);
    let mut router = RaftRouter::new(config.clone());
    let _ = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    let leader_id = router.leader().expect("expected a leader after init");
    let follower: u64 = if leader_id == 0 { 1 } else { 0 };

    let follower_handle = router.get_raft_handle(&follower)?;
    let res = follower_handle.trigger().transfer_to(2).await;
    match res {
        Err(TransferError::NotLeader) => {}
        other => panic!("expected NotLeader; got {:?}", other),
    }

    Ok(())
}

#[async_entry::test(worker_threads = 8, init = "init_default_ut_tracing()", tracing_span = "debug")]
async fn transfer_to_rejects_self_target() -> Result<()> {
    // Sanity: cannot transfer to self.
    let config = Arc::new(Config { ..Default::default() }.validate()?);
    let mut router = RaftRouter::new(config.clone());
    let _ = router.new_cluster(btreeset! {0, 1, 2}, btreeset! {}).await?;

    let leader_id = router.leader().expect("expected a leader after init");

    let leader_handle = router.get_raft_handle(&leader_id)?;
    let res = leader_handle.trigger().transfer_to(leader_id).await;
    match res {
        Err(TransferError::TargetIsSelf(t)) if t == leader_id => {}
        other => panic!("expected TargetIsSelf; got {:?}", other),
    }

    Ok(())
}
