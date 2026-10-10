//! The twin clock and the seeded randomness: the two sources of anything a
//! run could otherwise do differently.
//!
//! The clock is frozen by default: it moves only when the control plane
//! advances it, so a run with the same seed and the same requests writes the
//! same history, byte for byte. In wall mode it also follows real time.
//! ref:DEC-twin-clock

use serde::{Deserialize, Serialize};
use std::time::Instant;

/// How the twin clock moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClockMode {
    /// Only `advance` moves it.
    Frozen,
    /// It follows real time, and `advance` adds to it.
    Wall,
}

impl std::str::FromStr for ClockMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "frozen" => Ok(ClockMode::Frozen),
            "wall" => Ok(ClockMode::Wall),
            _ => Err(format!("the clock is frozen or wall, not {s}")),
        }
    }
}

/// The twin clock, in milliseconds since the Unix epoch.
#[derive(Clone, Debug)]
pub struct Clock {
    mode: ClockMode,
    start_ms: i64,
    advanced_ms: i64,
    wall_start: Instant,
}

impl Clock {
    /// A clock that reads `start_ms` now.
    pub fn new(mode: ClockMode, start_ms: i64) -> Self {
        Self { mode, start_ms, advanced_ms: 0, wall_start: Instant::now() }
    }

    /// The time now.
    pub fn now_ms(&self) -> i64 {
        let wall = match self.mode {
            ClockMode::Frozen => 0,
            ClockMode::Wall => self.wall_start.elapsed().as_millis() as i64,
        };
        self.start_ms.saturating_add(self.advanced_ms).saturating_add(wall)
    }

    /// Moves the clock forward.
    pub fn advance(&mut self, ms: i64) {
        self.advanced_ms = self.advanced_ms.saturating_add(ms.max(0));
    }

    /// How the clock moves.
    pub fn mode(&self) -> ClockMode {
        self.mode
    }
}

/// SplitMix64: a small, fast generator whose sequence is fixed by its seed.
#[derive(Clone, Debug)]
pub struct Random {
    state: u64,
}

impl Random {
    /// The generator for a seed.
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// Reads a duration such as `250ms`, `5s`, `2m` or `1h` as milliseconds.
pub fn parse_duration(s: &str) -> Result<i64, String> {
    let s = s.trim();
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (n, unit) = s.split_at(split);
    let n: i64 = n.parse().map_err(|_| format!("{s} is not a duration such as 500ms, 5s or 2m"))?;
    let scale = match unit {
        "ms" => 1,
        "s" | "" => 1000,
        "m" => 60_000,
        "h" => 3_600_000,
        _ => return Err(format!("{s} is not a duration such as 500ms, 5s or 2m")),
    };
    n.checked_mul(scale).ok_or_else(|| format!("{s} is too long"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("250ms"), Ok(250));
        assert_eq!(parse_duration("5s"), Ok(5000));
        assert_eq!(parse_duration("2m"), Ok(120_000));
        assert!(parse_duration("soon").is_err());
    }

    #[test]
    fn the_seed_fixes_the_sequence() {
        let (mut a, mut b) = (Random::new(7), Random::new(7));
        assert_eq!((a.next_u64(), a.next_u64()), (b.next_u64(), b.next_u64()));
        assert_ne!(Random::new(7).next_u64(), Random::new(8).next_u64());
    }

    #[test]
    fn a_frozen_clock_moves_only_when_advanced() {
        let mut c = Clock::new(ClockMode::Frozen, 1000);
        assert_eq!(c.now_ms(), 1000);
        c.advance(500);
        assert_eq!(c.now_ms(), 1500);
    }
}
