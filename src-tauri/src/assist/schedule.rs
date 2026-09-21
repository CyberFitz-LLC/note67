//! When a pass may run.
//!
//! Separated from the passes themselves because the rule is the part that
//! matters and the part that is easy to get wrong, and because it can then be
//! tested without a model, a network or a clock.
//!
//! The rule: **never queue.** Transcript arriving while a pass is in flight
//! marks the state dirty and does nothing else. When the pass returns, one more
//! runs if anything changed since it started.
//!
//! Dropping intermediate states is correct rather than merely convenient. Only
//! the current state of a conversation is worth describing, and a queue turns a
//! model that is a minute behind into one that spends an hour answering the
//! meeting's first ten minutes — confidently, while the room has moved on. The
//! recogniser feeding this has already been observed two minutes behind on a
//! loaded appliance.

use std::time::{Duration, Instant};

/// Why a pass is being run, or why it is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Run now.
    Run,
    /// A pass is already in flight; this one is folded into it.
    AlreadyRunning,
    /// Nothing has changed since the last pass.
    NothingNew,
    /// Changed, but not enough time has passed.
    TooSoon,
}

/// Tracks whether a periodic pass should run.
#[derive(Debug)]
pub struct Cadence {
    interval: Duration,
    in_flight: bool,
    dirty: bool,
    last_started: Option<Instant>,
}

impl Cadence {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            in_flight: false,
            dirty: false,
            last_started: None,
        }
    }

    /// New material arrived.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn is_in_flight(&self) -> bool {
        self.in_flight
    }

    /// Ask whether to run, at `now`.
    ///
    /// Takes the instant rather than reading the clock so the rule can be
    /// tested at speed instead of in real time.
    pub fn decide(&mut self, now: Instant) -> Decision {
        if self.in_flight {
            return Decision::AlreadyRunning;
        }
        if !self.dirty {
            return Decision::NothingNew;
        }
        if let Some(last) = self.last_started
            && now.duration_since(last) < self.interval
        {
            return Decision::TooSoon;
        }
        self.in_flight = true;
        // Cleared at the start, not the end: anything arriving *during* the
        // pass is genuinely new to it and must survive to trigger the next one.
        // Clearing on completion would swallow everything said while the model
        // was thinking, which on a slow endpoint is most of the conversation.
        self.dirty = false;
        self.last_started = Some(now);
        Decision::Run
    }

    /// A pass finished, however it went.
    ///
    /// Called on failure as well as success: a pass that errored still has to
    /// release the flag, or one bad response stops the pane for the rest of the
    /// meeting.
    pub fn finished(&mut self) {
        self.in_flight = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn nothing_runs_until_there_is_something_to_say() {
        let mut c = Cadence::new(Duration::from_secs(60));
        assert_eq!(c.decide(Instant::now()), Decision::NothingNew);
    }

    #[test]
    fn the_first_change_runs_immediately() {
        // No waiting out an interval before the first brief: a meeting that has
        // just started is exactly when someone looks at the pane.
        let mut c = Cadence::new(Duration::from_secs(60));
        c.mark_dirty();
        assert_eq!(c.decide(Instant::now()), Decision::Run);
    }

    #[test]
    fn passes_never_stack() {
        // The rule this module exists for.
        let base = Instant::now();
        let mut c = Cadence::new(Duration::from_secs(60));
        c.mark_dirty();
        assert_eq!(c.decide(base), Decision::Run);

        for second in 1..30 {
            c.mark_dirty();
            assert_eq!(
                c.decide(at(base, second * 10)),
                Decision::AlreadyRunning,
                "a second pass started while the first was still running"
            );
        }
    }

    #[test]
    fn work_arriving_during_a_pass_triggers_the_next_one() {
        // The reason dirty is cleared when a pass starts rather than when it
        // ends: everything said while the model was thinking is new to it, and
        // on a slow endpoint that is most of the conversation.
        let base = Instant::now();
        let mut c = Cadence::new(Duration::from_secs(60));
        c.mark_dirty();
        assert_eq!(c.decide(base), Decision::Run);

        c.mark_dirty(); // said while the model was working
        c.finished();

        assert_eq!(c.decide(at(base, 61)), Decision::Run);
    }

    #[test]
    fn a_quiet_stretch_costs_nothing() {
        let base = Instant::now();
        let mut c = Cadence::new(Duration::from_secs(60));
        c.mark_dirty();
        assert_eq!(c.decide(base), Decision::Run);
        c.finished();

        // Nobody said anything for ten minutes.
        assert_eq!(c.decide(at(base, 600)), Decision::NothingNew);
    }

    #[test]
    fn the_interval_is_respected_once_running() {
        let base = Instant::now();
        let mut c = Cadence::new(Duration::from_secs(60));
        c.mark_dirty();
        assert_eq!(c.decide(base), Decision::Run);
        c.finished();

        c.mark_dirty();
        assert_eq!(c.decide(at(base, 30)), Decision::TooSoon);
        assert_eq!(c.decide(at(base, 60)), Decision::Run);
    }

    #[test]
    fn a_failed_pass_does_not_stop_the_pane() {
        // finished() is called however a pass went. Without that, one bad
        // response leaves the flag set and the pane dead for the rest of the
        // meeting.
        let base = Instant::now();
        let mut c = Cadence::new(Duration::from_secs(60));
        c.mark_dirty();
        assert_eq!(c.decide(base), Decision::Run);
        c.finished(); // the pass errored

        c.mark_dirty();
        assert_eq!(c.decide(at(base, 61)), Decision::Run);
    }
}
