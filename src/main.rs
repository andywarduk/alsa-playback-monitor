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
mod tracker;

use std::ffi::OsStr;
use std::io::{self, Write};
use std::os::fd::AsFd;
use std::process::{self, Command};
use std::time::{Duration, Instant};

use chrono::Local;
use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};

use args::Args;
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

fn report(playing: bool, detail: &str) {
    let stamp = Local::now().format("%Y-%m-%d %H:%M:%S");
    let word = if playing { "playing" } else { "stopped" };
    let mut out = io::stdout().lock();
    let _ = if detail.is_empty() {
        writeln!(out, "{stamp} {word}")
    } else {
        writeln!(out, "{stamp} {word}  {detail}")
    };
    let _ = out.flush();
}

fn run_hook(cmd: Option<&str>) {
    let Some(cmd) = cmd.filter(|cmd| !cmd.is_empty()) else {
        return;
    };
    if let Err(err) = Command::new("/bin/sh").arg("-c").arg(cmd).status() {
        eprintln!("alsa-playback-monitor: cannot run {cmd:?}: {err}");
    }
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
    report(tracker.playing(), &asound::describe(&active));

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
                report(playing, &if playing { asound::describe(&active) } else { String::new() });
                run_hook(if playing { args.on_start.as_deref() } else { args.on_stop.as_deref() });
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
