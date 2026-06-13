use crate::Instant;

/// Tracks explicit invalidation of this node's leader lease (W3.7).
///
/// The leader lease is normally derived implicitly from the vote's last-modified time plus the
/// `leader_lease` duration: while that window is open, the node refuses to grant (pre-)votes so it
/// does not disrupt a live leader (see [`Engine::leader_lease_is_valid`]).
///
/// That implicit derivation has one gap: when *this* node was the leader and then steps down (a
/// membership change committed it out of the configuration), its own committed-vote utime is still
/// recent, so the implicit lease keeps reporting "valid" and the former leader keeps rejecting
/// pre-votes for its successor — extending an avoidable leaderless gap.
///
/// `LeaderLease` closes that gap by recording the instant at which the lease was explicitly
/// invalidated. A lease derived from a vote modified at `vote_utime` is considered alive only if it
/// has **not** been invalidated at or after `vote_utime`; acknowledging a *new* leader (which moves
/// `vote_utime` forward past any prior invalidation) naturally re-arms the lease, so invalidation
/// suppresses only the stale lease, never a future leader's.
///
/// [`Engine::leader_lease_is_valid`]: `crate::engine::Engine::leader_lease_is_valid`
#[derive(Debug, Clone)]
#[derive(PartialEq, Eq)]
pub(crate) struct LeaderLease<I: Instant> {
    /// The most recent time the lease was explicitly invalidated, if ever.
    invalidated_at: Option<I>,
}

impl<I: Instant> Default for LeaderLease<I> {
    fn default() -> Self {
        Self { invalidated_at: None }
    }
}

impl<I: Instant> LeaderLease<I> {
    /// Explicitly invalidate the lease as of `now`.
    ///
    /// Called when this node relinquishes leadership (`leader_step_down`) so its own still-recent
    /// committed vote no longer keeps the implicit lease alive.
    pub(crate) fn invalidate(&mut self, now: I) {
        // Keep the latest invalidation instant. `invalidate` is monotone in time, so a later call
        // strictly dominates; guard against a non-monotone clock by taking the max.
        self.invalidated_at = match self.invalidated_at {
            Some(prev) if prev >= now => Some(prev),
            _ => Some(now),
        };
    }

    /// Whether the lease derived from a vote last modified at `vote_utime` has been invalidated.
    ///
    /// Returns `true` iff an explicit [`invalidate`](Self::invalidate) happened at or after
    /// `vote_utime` — i.e. the invalidation is at least as fresh as the vote it would suppress. A
    /// later vote (a new acknowledged leader) moves `vote_utime` past the invalidation and the
    /// lease becomes live again.
    pub(crate) fn is_invalidated_for(&self, vote_utime: I) -> bool {
        match self.invalidated_at {
            Some(at) => at >= vote_utime,
            None => false,
        }
    }
}
