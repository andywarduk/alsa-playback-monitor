//! Runs the `--hook` program for each change of state.
//!
//! Changes are queued to a worker thread that runs the hook for each one in
//! turn. A slow hook therefore never delays event handling (so the logged
//! change times stay accurate), and a "stopped" hook never overtakes the
//! "playing" one before it. Because a hook can start later than its change,
//! the worker logs when each hook starts and finishes.
//!
//! With a stop-hook delay, the hook for "stopped" waits that long after the
//! change (see `Schedule`), so the log shows the real stop time while the
//! hook, e.g. switching an amplifier off, runs only after a longer silence.

use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitStatus};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::asound::{self, Stream};

/// A reported change of state.
pub struct Change {
    pub playing: bool,
    /// The change's timestamp, as logged.
    pub time: String,
    /// The open streams; empty when stopped.
    pub streams: Vec<Stream>,
    /// When the change was reported, to log how long its hook waited.
    pub at: Instant,
}

pub struct Hook {
    queue: Sender<Change>,
}

impl Hook {
    /// Start a worker thread that runs `program` for queued changes, holding
    /// back "stopped" ones for `stop_delay`.
    pub fn spawn(program: String, stop_delay: Duration) -> Self {
        let (queue, changes) = mpsc::channel::<Change>();
        thread::spawn(move || worker(&program, stop_delay, &changes));
        Self { queue }
    }

    pub fn queue(&self, change: Change) {
        // Sending fails only if the worker has died, and then there's nothing to do.
        let _ = self.queue.send(change);
    }
}

fn worker(program: &str, stop_delay: Duration, changes: &Receiver<Change>) {
    let mut schedule = Schedule { stop_delay, pending: None };
    loop {
        let received = match schedule.due() {
            None => changes.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(due) => changes.recv_timeout(due.saturating_duration_since(Instant::now())),
        };
        match received {
            Ok(change) => match schedule.change(change) {
                Action::Run(change) => run(program, &change),
                Action::Wait => {}
                Action::Cancel(stopped) => {
                    let after = stopped.at.elapsed().as_secs_f64();
                    crate::log(&format!("hook for stopped cancelled: playing again {after:.1}s after the change"));
                }
            },
            Err(RecvTimeoutError::Timeout) => {
                if let Some(stopped) = schedule.expire() {
                    run(program, &stopped);
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Which changes the hook runs for, and when. Pure logic, so it can be
/// unit tested; the worker supplies the waiting.
struct Schedule {
    stop_delay: Duration,
    /// A "stopped" change waiting out `stop_delay`.
    pending: Option<Change>,
}

enum Action {
    /// Run the hook for this change now.
    Run(Change),
    /// Hold the "stopped" change until `due()`, then `expire()`.
    Wait,
    /// Playing again before the pending "stopped" hook ran: drop it. The hook
    /// never saw the stop, so this "playing" needs no hook either.
    Cancel(Change),
}

impl Schedule {
    /// When the pending "stopped" hook is due, if there is one.
    fn due(&self) -> Option<Instant> {
        self.pending.as_ref().map(|stopped| stopped.at + self.stop_delay)
    }

    fn change(&mut self, change: Change) -> Action {
        match (change.playing, self.pending.take()) {
            (true, Some(stopped)) => Action::Cancel(stopped),
            (false, _) if !self.stop_delay.is_zero() => {
                self.pending = Some(change);
                Action::Wait
            }
            _ => Action::Run(change),
        }
    }

    /// The pending "stopped" change, once it's due.
    fn expire(&mut self) -> Option<Change> {
        self.pending.take()
    }
}

fn run(program: &str, change: &Change) {
    let word = crate::state_word(change.playing);
    let waited = change.at.elapsed().as_secs_f64();
    crate::log(&format!("hook started for {word}, {waited:.1}s after the change"));
    let started = Instant::now();
    match command(program, change).status() {
        Ok(status) => {
            let took = started.elapsed().as_secs_f64();
            crate::log(&format!("hook finished for {word} after {took:.1}s: {}", outcome(status)));
        }
        Err(err) => crate::log(&format!("hook failed for {word}: {program}: {err}")),
    }
}

fn outcome(status: ExitStatus) -> String {
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("exit {code}"),
        (None, Some(signal)) => format!("killed by signal {signal}"),
        _ => status.to_string(),
    }
}

/// `program`, with no arguments: the new state and its details are in
/// PLAYBACK_* variables. Every variable is always set, empty if it doesn't
/// apply, so scripts can use `set -u`. Stream variables describe the first
/// open stream; there is normally only one.
fn command(program: &str, change: &Change) -> Command {
    let word = crate::state_word(change.playing);
    let stream = change.streams.first();
    let hw = stream.and_then(|stream| stream.hw.as_ref());
    let vars = [
        ("PLAYBACK_STATE", word.to_owned()),
        ("PLAYBACK_TIME", change.time.clone()),
        ("PLAYBACK_DETAILS", asound::describe(&change.streams)),
        ("PLAYBACK_DEVICE", stream.map(|s| format!("hw:{},{}", s.card, s.device)).unwrap_or_default()),
        ("PLAYBACK_PROGRAM", stream.and_then(|s| s.program.clone()).unwrap_or_default()),
        ("PLAYBACK_PID", stream.map(|s| s.pid.clone()).unwrap_or_default()),
        ("PLAYBACK_PCM_STATE", stream.map(|s| s.state.clone()).unwrap_or_default()),
        ("PLAYBACK_FORMAT", hw.map(|hw| hw.format.clone()).unwrap_or_default()),
        ("PLAYBACK_RATE", hw.map(|hw| hw.rate.clone()).unwrap_or_default()),
        ("PLAYBACK_CHANNELS", hw.map(|hw| hw.channels.clone()).unwrap_or_default()),
    ];
    let mut command = Command::new(program);
    command.envs(vars);
    command
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::asound::HwParams;

    fn change(streams: Vec<Stream>) -> Change {
        Change { playing: !streams.is_empty(), time: "2026-09-27 10:00:01".to_owned(), streams, at: Instant::now() }
    }

    fn change_at(playing: bool, at: Instant) -> Change {
        Change { playing, time: String::new(), streams: Vec::new(), at }
    }

    fn schedule(stop_delay: Duration) -> Schedule {
        Schedule { stop_delay, pending: None }
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn args_and_env(command: &Command) -> (Vec<&str>, HashMap<&str, &str>) {
        let args = command.get_args().map(|arg| arg.to_str().unwrap()).collect();
        let env = command
            .get_envs()
            .map(|(key, value)| (key.to_str().unwrap(), value.unwrap().to_str().unwrap()))
            .collect();
        (args, env)
    }

    #[test]
    fn playing_sets_state_and_first_stream() {
        let stream = Stream {
            card: 0,
            device: 0,
            program: Some("librespot".to_owned()),
            pid: "539".to_owned(),
            state: "RUNNING".to_owned(),
            hw: Some(HwParams {
                format: "S16_LE".to_owned(),
                rate: "44100".to_owned(),
                channels: "2".to_owned(),
            }),
        };
        let command = command("/usr/local/bin/amp", &change(vec![stream]));
        assert_eq!(command.get_program(), "/usr/local/bin/amp");
        let (args, env) = args_and_env(&command);
        assert!(args.is_empty());
        assert_eq!(
            env,
            HashMap::from([
                ("PLAYBACK_STATE", "playing"),
                ("PLAYBACK_TIME", "2026-09-27 10:00:01"),
                ("PLAYBACK_DETAILS", "hw:0,0 librespot[539] RUNNING S16_LE 44100Hz 2ch"),
                ("PLAYBACK_DEVICE", "hw:0,0"),
                ("PLAYBACK_PROGRAM", "librespot"),
                ("PLAYBACK_PID", "539"),
                ("PLAYBACK_PCM_STATE", "RUNNING"),
                ("PLAYBACK_FORMAT", "S16_LE"),
                ("PLAYBACK_RATE", "44100"),
                ("PLAYBACK_CHANNELS", "2"),
            ])
        );
    }

    #[test]
    fn stopped_sets_stream_variables_empty() {
        let command = command("amp", &change(Vec::new()));
        let (args, env) = args_and_env(&command);
        assert!(args.is_empty());
        assert_eq!(env.len(), 10);
        assert_eq!(env["PLAYBACK_STATE"], "stopped");
        assert_eq!(env["PLAYBACK_TIME"], "2026-09-27 10:00:01");
        for (key, value) in env {
            if !matches!(key, "PLAYBACK_STATE" | "PLAYBACK_TIME") {
                assert_eq!(value, "", "{key}");
            }
        }
    }

    #[test]
    fn without_stop_delay_every_change_runs_at_once() {
        let (mut s, t0) = (schedule(Duration::ZERO), Instant::now());
        assert!(matches!(s.change(change_at(true, t0)), Action::Run(c) if c.playing));
        assert!(matches!(s.change(change_at(false, t0)), Action::Run(c) if !c.playing));
        assert_eq!(s.due(), None);
    }

    #[test]
    fn stopped_hook_waits_for_stop_delay() {
        let (mut s, t0) = (schedule(secs(300)), Instant::now());
        assert!(matches!(s.change(change_at(true, t0)), Action::Run(_)));
        assert!(matches!(s.change(change_at(false, t0 + secs(10))), Action::Wait));
        assert_eq!(s.due(), Some(t0 + secs(310)));
        assert!(s.expire().is_some_and(|c| !c.playing));
        assert_eq!(s.due(), None);
        // The hook saw the stop, so playing again runs it.
        assert!(matches!(s.change(change_at(true, t0 + secs(400))), Action::Run(_)));
    }

    #[test]
    fn playing_again_cancels_pending_stop_and_skips_playing_hook() {
        let (mut s, t0) = (schedule(secs(300)), Instant::now());
        assert!(matches!(s.change(change_at(false, t0)), Action::Wait));
        assert!(matches!(s.change(change_at(true, t0 + secs(60))), Action::Cancel(c) if c.at == t0));
        assert_eq!(s.due(), None);
        assert!(s.expire().is_none());
        // The next stop waits afresh from its own time.
        assert!(matches!(s.change(change_at(false, t0 + secs(120))), Action::Wait));
        assert_eq!(s.due(), Some(t0 + secs(420)));
    }
}
