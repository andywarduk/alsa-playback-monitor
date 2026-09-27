//! Report when ALSA playback starts and stops.
//!
//! Event-driven rather than polled: an inotify watch on /dev/snd wakes the
//! program whenever a playback PCM node (pcmC<card>D<dev>p) is opened or
//! closed. Between events it sits blocked in poll() and uses no CPU.
//!
//! "Playing" means at least one playback device is open. ALSA raises no event
//! for pause/resume inside an already-open stream, but most players (librespot,
//! shairport-sync, aplay, mpd, PipeWire after its idle suspend) close the
//! device when they stop, so open/close tracks playback well in practice.

#![forbid(unsafe_code)]

mod args;
mod asound;
mod hook;
mod tracker;

use std::ffi::OsStr;
use std::io::{self, Write};
use std::os::fd::AsFd;
use std::process;
use std::time::{Duration, Instant};

use chrono::Local;
use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};

use args::Args;
use asound::Stream;
use hook::{Change, Hook};
use tracker::Tracker;

const DEV_DIR: &str = "/dev/snd";

/// Run `f` again if a signal interrupts it.
fn retry_eintr<T>(mut f: impl FnMut() -> nix::Result<T>) -> nix::Result<T> {
    loop {
        match f() {
            Err(Errno::EINTR) => {}
            result => return result,
        }
    }
}

/// Block until `fd` is readable or `timeout` passes (`None` waits forever).
/// Returns true if readable.
fn wait_readable(fd: impl AsFd, timeout: Option<Duration>) -> nix::Result<bool> {
    // Round up to whole milliseconds so we never wake before the deadline.
    let timeout = timeout.map_or(PollTimeout::NONE, |t| {
        PollTimeout::try_from(t.as_nanos().div_ceil(1_000_000)).unwrap_or(PollTimeout::MAX)
    });
    let mut fds = [PollFd::new(fd.as_fd(), PollFlags::POLLIN)];
    retry_eintr(|| poll(&mut fds, timeout)).map(|ready| ready > 0)
}

fn timestamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Print `text` as one output line stamped with `time`. Used from both the
/// main thread and the hook worker; the lock keeps their lines whole.
fn log_at(time: &str, text: &str) {
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "{time} {text}");
    let _ = out.flush();
}

fn log(text: &str) {
    log_at(&timestamp(), text);
}

fn state_word(playing: bool) -> &'static str {
    if playing { "playing" } else { "stopped" }
}

/// Print a state line and return its timestamp.
fn report(playing: bool, streams: &[Stream]) -> String {
    let time = timestamp();
    let word = state_word(playing);
    if streams.is_empty() {
        log_at(&time, word);
    } else {
        log_at(&time, &format!("{word}  {}", asound::describe(streams)));
    }
    time
}

fn run(args: &Args) -> io::Result<()> {
    // Watch before taking the initial snapshot so no open/close slips between them.
    let inotify = Inotify::init(InitFlags::IN_CLOEXEC)?;
    let watch = AddWatchFlags::IN_OPEN | AddWatchFlags::IN_CLOSE | AddWatchFlags::IN_DELETE;
    inotify
        .add_watch(DEV_DIR, watch)
        .map_err(|err| io::Error::other(format!("{DEV_DIR}: {err}")))?;

    let active = asound::open_substreams();
    let mut tracker = Tracker::new(asound::count_open(&active), args.start_delay, args.stop_delay);
    report(tracker.playing(), &asound::streams(&active));
    let hook = args.hook.clone().map(Hook::spawn);

    loop {
        if wait_readable(&inotify, tracker.timeout(Instant::now()))? {
            let now = Instant::now();
            for event in retry_eintr(|| inotify.read_events())? {
                let node = event.name.as_deref().and_then(OsStr::to_str).and_then(asound::parse_playback_node);
                if event.mask.contains(AddWatchFlags::IN_Q_OVERFLOW) {
                    // Events were dropped; resync from /proc.
                    tracker.resync(asound::count_open(&asound::open_substreams()), now);
                } else if let Some(node) = node {
                    if event.mask.contains(AddWatchFlags::IN_OPEN) {
                        tracker.opened(node, now);
                    } else if event.mask.intersects(AddWatchFlags::IN_CLOSE) {
                        tracker.closed(node, now);
                    } else if event.mask.contains(AddWatchFlags::IN_DELETE) {
                        tracker.removed(node, now); // card unplugged
                    }
                }
            }
        } else {
            // A change of state has held for the full delay: report it.
            let active = asound::open_substreams();
            if let Some(playing) = tracker.expire(|node| active.contains_key(node)) {
                let streams = if playing { asound::streams(&active) } else { Vec::new() };
                let time = report(playing, &streams);
                if let Some(hook) = &hook {
                    hook.queue(Change { playing, time, streams, at: Instant::now() });
                }
            }
        }
    }
}

fn main() {
    let args = Args::parse();
    if let Err(err) = run(&args) {
        eprintln!("alsa-playback-monitor: {err}");
        process::exit(1);
    }
}
