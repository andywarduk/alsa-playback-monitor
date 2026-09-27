//! Runs the `--hook` program for each change of state.
//!
//! Changes are queued to a worker thread that runs the hook for each one in
//! turn. A slow hook therefore never delays event handling (so the logged
//! change times stay accurate), and a "stopped" hook never overtakes the
//! "playing" one before it. Because a hook can start later than its change,
//! the worker logs when each hook starts and finishes.

use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitStatus};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Instant;

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
    /// Start a worker thread that runs `program` for each queued change.
    pub fn spawn(program: String) -> Self {
        let (queue, changes) = mpsc::channel::<Change>();
        thread::spawn(move || {
            for change in changes {
                run(&program, &change);
            }
        });
        Self { queue }
    }

    pub fn queue(&self, change: Change) {
        // Sending fails only if the worker has died, and then there's nothing to do.
        let _ = self.queue.send(change);
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
}
