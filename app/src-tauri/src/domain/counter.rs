//! Turning the core's cumulative counters into traffic deltas.
//!
//! mihomo reports `upTotal` / `downTotal`: bytes since the core process started. Every delta we
//! record is a difference of two such readings, so nothing is lost between samples, and after a
//! pause (Monitor restarted, the machine slept) the gap is recovered from the counters as long as
//! the same core kept running.

use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Bytes {
    pub up: u64,
    pub down: u64,
}

impl Bytes {
    pub const ZERO: Bytes = Bytes { up: 0, down: 0 };

    pub fn new(up: u64, down: u64) -> Self {
        Bytes { up, down }
    }

    pub fn is_zero(self) -> bool {
        self == Self::ZERO
    }
}

impl std::ops::Add for Bytes {
    type Output = Bytes;
    fn add(self, o: Bytes) -> Bytes {
        Bytes { up: self.up + o.up, down: self.down + o.down }
    }
}

impl std::ops::AddAssign for Bytes {
    fn add_assign(&mut self, o: Bytes) {
        *self = *self + o;
    }
}

/// One run of a core process. Its counters start at zero, so a different run means a restart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreId {
    pub pid: u32,
    /// Process start, unix seconds.
    pub started_at: i64,
}

/// The last counters already accounted for, and which core run they belong to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// `None` when the core is not a local process we can see (a remote controller).
    pub core: Option<CoreId>,
    pub totals: Bytes,
    /// When the counters were read, unix seconds.
    pub at: i64,
}

/// Traffic between two readings of the same core run. Counters going backwards mean the core
/// restarted between them, so everything it reports now is new.
pub fn step(prev: Bytes, cur: Bytes) -> Bytes {
    if cur.up < prev.up || cur.down < prev.down {
        cur
    } else {
        Bytes { up: cur.up - prev.up, down: cur.down - prev.down }
    }
}

/// Traffic to credit for the first reading after connecting to a core, given the last
/// checkpoint of the same source.
pub fn resume(last: Option<&Checkpoint>, core: Option<&CoreId>, first: Bytes) -> Bytes {
    // Never seen this source: what the core counted before Monitor was there is not ours to
    // attribute (vnstat behaves the same way on a new interface).
    let Some(last) = last else { return Bytes::ZERO };
    match (&last.core, core) {
        (Some(a), Some(b)) if a == b => step(last.totals, first),
        // A run that started after the checkpoint: all of its traffic happened since.
        (_, Some(b)) if b.started_at >= last.at => first,
        // A different run that started before the checkpoint cannot be told apart from traffic
        // already counted; start from its current counters instead of risking a double count.
        (Some(_), Some(_)) => Bytes::ZERO,
        // Without a process to compare, the counters themselves are the only evidence.
        _ => step(last.totals, first),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkpoint(core: Option<CoreId>, up: u64, down: u64, at: i64) -> Checkpoint {
        Checkpoint { core, totals: Bytes::new(up, down), at }
    }

    fn run(pid: u32, started_at: i64) -> CoreId {
        CoreId { pid, started_at }
    }

    #[test]
    fn step_is_the_difference_of_growing_counters() {
        // Arrange
        let prev = Bytes::new(100, 1_000);

        // Act
        let delta = step(prev, Bytes::new(150, 1_400));

        // Assert
        assert_eq!(delta, Bytes::new(50, 400));
    }

    #[test]
    fn step_counts_everything_after_counters_go_backwards() {
        // Arrange
        let prev = Bytes::new(5_000, 9_000);

        // Act
        let delta = step(prev, Bytes::new(20, 30));

        // Assert
        assert_eq!(delta, Bytes::new(20, 30));
    }

    #[test]
    fn step_treats_either_direction_going_backwards_as_a_restart() {
        // Arrange
        let prev = Bytes::new(100, 9_000);

        // Act
        let delta = step(prev, Bytes::new(200, 30));

        // Assert
        assert_eq!(delta, Bytes::new(200, 30));
    }

    #[test]
    fn first_sight_of_a_source_is_only_a_baseline() {
        // Act
        let delta = resume(None, Some(&run(7, 100)), Bytes::new(10, 20));

        // Assert
        assert!(delta.is_zero());
    }

    #[test]
    fn same_core_run_recovers_the_gap() {
        // Arrange
        let last = checkpoint(Some(run(7, 100)), 1_000, 5_000, 500);

        // Act
        let delta = resume(Some(&last), Some(&run(7, 100)), Bytes::new(1_300, 9_000));

        // Assert
        assert_eq!(delta, Bytes::new(300, 4_000));
    }

    #[test]
    fn core_started_after_the_checkpoint_counts_all_its_traffic() {
        // Arrange
        let last = checkpoint(Some(run(7, 100)), 1_000_000, 5_000_000, 500);

        // Act: the new run has not caught up with the old counters yet, and has passed them.
        let behind = resume(Some(&last), Some(&run(8, 600)), Bytes::new(40, 50));
        let ahead = resume(Some(&last), Some(&run(8, 600)), Bytes::new(2_000_000, 9_000_000));

        // Assert
        assert_eq!(behind, Bytes::new(40, 50));
        assert_eq!(ahead, Bytes::new(2_000_000, 9_000_000));
    }

    #[test]
    fn different_core_from_before_the_checkpoint_is_only_a_baseline() {
        // Arrange
        let last = checkpoint(Some(run(7, 100)), 1_000, 5_000, 500);

        // Act
        let delta = resume(Some(&last), Some(&run(8, 300)), Bytes::new(3_000, 8_000));

        // Assert
        assert!(delta.is_zero());
    }

    #[test]
    fn unknown_core_falls_back_to_the_counters() {
        // Arrange
        let last = checkpoint(None, 1_000, 5_000, 500);

        // Act
        let grew = resume(Some(&last), None, Bytes::new(1_200, 5_100));
        let restarted = resume(Some(&last), None, Bytes::new(10, 20));

        // Assert
        assert_eq!(grew, Bytes::new(200, 100));
        assert_eq!(restarted, Bytes::new(10, 20));
    }
}
