//! The playing/stopped decision: open counts per device plus the start/stop
//! delays. Pure logic with the clock passed in, so it can be unit tested
//! without a sound card.
//!
//! Counts come from inotify open/close events rather than rereading /proc
//! after each one, because the close event fires *before* the driver's
//! release runs, so /proc can still show a just-closed stream as open.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::asound::Node;

pub struct Tracker {
    /// Open handles per playback device.
    counts: HashMap<Node, u32>,
    /// The state last reported.
    playing: bool,
    /// When a pending change of state is due to be reported.
    deadline: Option<Instant>,
    start_delay: Duration,
    stop_delay: Duration,
}

impl Tracker {
    /// Start from the devices already open.
    pub fn new(counts: HashMap<Node, u32>, start_delay: Duration, stop_delay: Duration) -> Self {
        let playing = counts.values().any(|&n| n > 0);
        Self { counts, playing, deadline: None, start_delay, stop_delay }
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    /// Time left until a pending change is due, or `None` if nothing is pending.
    pub fn timeout(&self, now: Instant) -> Option<Duration> {
        self.deadline.map(|deadline| deadline.saturating_duration_since(now))
    }

    pub fn opened(&mut self, node: Node, now: Instant) {
        *self.counts.entry(node).or_default() += 1;
        self.update_deadline(now);
    }

    pub fn closed(&mut self, node: Node, now: Instant) {
        if let Some(n) = self.counts.get_mut(&node) {
            *n = n.saturating_sub(1);
        }
        self.update_deadline(now);
    }

    /// The device node was deleted (card unplugged).
    pub fn removed(&mut self, node: Node, now: Instant) {
        self.counts.remove(&node);
        self.update_deadline(now);
    }

    /// Replace all counts, after inotify reports that it dropped events.
    pub fn resync(&mut self, counts: HashMap<Node, u32>, now: Instant) {
        self.counts = counts;
        self.update_deadline(now);
    }

    fn update_deadline(&mut self, now: Instant) {
        let now_playing = self.counts.values().any(|&n| n > 0);
        if now_playing == self.playing {
            self.deadline = None; // flipped back before the delay ran out
        } else if self.deadline.is_none() {
            let delay = if now_playing { self.start_delay } else { self.stop_delay };
            self.deadline = Some(now + delay);
        }
    }

    /// Call when the timeout has run out. `open_in_proc` says whether
    /// /proc/asound shows a device open. Returns the new state if it changed.
    pub fn expire(&mut self, open_in_proc: impl Fn(&Node) -> bool) -> Option<bool> {
        self.deadline = None;
        // /proc showing a device fully closed is definitive, so clear any
        // stale count; /proc showing it open is not (see module docs).
        self.counts.retain(|node, n| *n > 0 && open_in_proc(node));
        let now_playing = !self.counts.is_empty();
        if now_playing == self.playing {
            return None;
        }
        self.playing = now_playing;
        Some(now_playing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Node = (0, 0);
    const B: Node = (1, 0);
    const START: Duration = Duration::from_millis(200);
    const STOP: Duration = Duration::from_secs(1);

    fn tracker(open: &[(Node, u32)]) -> (Tracker, Instant) {
        (Tracker::new(open.iter().copied().collect(), START, STOP), Instant::now())
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn starting_state_comes_from_initial_counts() {
        assert!(!tracker(&[]).0.playing());
        assert!(!tracker(&[(A, 0)]).0.playing());
        assert!(tracker(&[(A, 1)]).0.playing());
    }

    #[test]
    fn open_is_reported_after_start_delay() {
        let (mut t, t0) = tracker(&[]);
        t.opened(A, t0);
        assert_eq!(t.timeout(t0), Some(START));
        assert_eq!(t.timeout(t0 + ms(150)), Some(ms(50)));
        assert_eq!(t.expire(|_| true), Some(true));
        assert!(t.playing());
        assert_eq!(t.timeout(t0), None);
    }

    #[test]
    fn blip_shorter_than_start_delay_reports_nothing() {
        let (mut t, t0) = tracker(&[]);
        t.opened(A, t0);
        t.closed(A, t0 + ms(100));
        assert_eq!(t.timeout(t0 + ms(100)), None);
        assert!(!t.playing());
    }

    #[test]
    fn reopening_cancels_stop_delay_and_next_close_restarts_it() {
        let (mut t, t0) = tracker(&[(A, 1)]);
        t.closed(A, t0);
        assert_eq!(t.timeout(t0), Some(STOP));
        t.opened(A, t0 + ms(500));
        assert_eq!(t.timeout(t0 + ms(500)), None);
        t.closed(A, t0 + ms(600));
        assert_eq!(t.timeout(t0 + ms(600)), Some(STOP));
        assert_eq!(t.expire(|_| false), Some(false));
    }

    #[test]
    fn repeated_opens_keep_the_first_deadline() {
        let (mut t, t0) = tracker(&[]);
        t.opened(A, t0);
        t.opened(B, t0 + ms(100));
        assert_eq!(t.timeout(t0 + ms(100)), Some(ms(100)));
    }

    #[test]
    fn close_wins_when_proc_still_shows_device_open() {
        // The close event arrives before the driver's release, so /proc lags.
        let (mut t, t0) = tracker(&[(A, 1)]);
        t.closed(A, t0);
        assert_eq!(t.expire(|_| true), Some(false));
    }

    #[test]
    fn proc_showing_closed_clears_count_whose_close_is_still_queued() {
        let (mut t, t0) = tracker(&[]);
        t.opened(A, t0);
        // By the deadline the player has closed, but that event is unread.
        assert_eq!(t.expire(|_| false), None);
        t.closed(A, t0 + ms(300));
        assert_eq!(t.timeout(t0 + ms(300)), None);
        assert!(!t.playing());
    }

    #[test]
    fn playing_continues_while_any_handle_is_open() {
        let (mut t, t0) = tracker(&[(A, 2), (B, 1)]);
        t.closed(A, t0);
        t.closed(B, t0);
        assert_eq!(t.timeout(t0), None);
        t.closed(A, t0);
        assert_eq!(t.timeout(t0), Some(STOP));
    }

    #[test]
    fn close_of_untracked_device_is_ignored() {
        let (mut t, t0) = tracker(&[]);
        t.closed(A, t0);
        assert_eq!(t.timeout(t0), None);
        t.opened(A, t0);
        assert_eq!(t.timeout(t0), Some(START));
    }

    #[test]
    fn unplugged_card_stops_playback() {
        let (mut t, t0) = tracker(&[(A, 1)]);
        t.removed(A, t0);
        assert_eq!(t.timeout(t0), Some(STOP));
        assert_eq!(t.expire(|_| false), Some(false));
    }

    #[test]
    fn resync_replaces_counts() {
        let (mut t, t0) = tracker(&[(A, 1)]);
        t.resync([(B, 1)].into(), t0);
        assert_eq!(t.timeout(t0), None);
        assert_eq!(t.expire(|&node| node == B), None);
        t.closed(A, t0);
        assert_eq!(t.timeout(t0), None);
        t.closed(B, t0);
        assert_eq!(t.timeout(t0), Some(STOP));
    }
}
