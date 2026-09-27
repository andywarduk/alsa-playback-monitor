# AGENTS.md

## Project

A single-binary Rust tool that reports when ALSA playback starts and stops. It is a port of an earlier Python script, and its output and options were verified to match it. See `README.md` for user-facing behaviour.

| File | Contents |
|---|---|
| `src/main.rs` | the event loop: inotify watch, `poll()` with a timeout, feeding events to the tracker, output lines |
| `src/hook.rs` | the `--hook` worker thread: runs the hook for each change in order, sets `PLAYBACK_*` variables, logs each run |
| `src/tracker.rs` | the playing/stopped decision: per-device open counts and the start/stop delays. No I/O; the clock is passed in |
| `src/asound.rs` | reading `/proc/asound`, device-node names, process names, the details text |
| `src/args.rs` | command-line options |

## Commands

```bash
cargo build --release
cargo test
cargo clippy --all-targets   # must stay warning-free
./install.sh                 # build, install to /usr/local, enable and restart the service
./install.sh uninstall
./log.sh [journalctl args]   # the service's journal, e.g. ./log.sh -f
```

`install.sh` must run as a normal user. It refuses to run as root, and it uses sudo only for the install steps. It installs the binary to `/usr/local/bin` and `alsa-playback-monitor.service` to `/usr/local/lib/systemd/system`. It rewrites the unit's `ExecStart` path if `PREFIX` is set. The unit runs `--hook /etc/alsa-playback-monitor/hook.sh`. `install.sh` copies that from `examples/hook.sh` only if it doesn't exist. Uninstall deletes it only if it's byte-identical to `examples/hook.sh`. An edited hook is the user's config: never overwrite or delete it. The service runs with `DynamicUser=yes`. Don't add `PrivateDevices=` or `ProtectProc=`: they hide `/dev/snd` and other processes' names. After changing the code or the unit, rerun `./install.sh` and check `./log.sh`.

`.cargo/config.toml` builds with `-Ctarget-cpu=native` on aarch64, the same setup as the other projects in `~/Git`. On the development Pi (a 3B+) that means Cortex-A53. The release profile also uses `lto = true` and `opt-level = "s"`, so release rebuilds take about 45 s on the Pi. Use debug builds (`cargo build`, `cargo test`) while iterating.

## Design rules

- **No polling.** The only place the program blocks is `poll()` on the inotify fd. A timeout is set only while a start or stop delay is pending. Don't add periodic timers, sleep loops or repeated `/proc` scans.
- **inotify open counts decide whether a device is open.** Treat `/proc/asound` as secondary. It provides the startup snapshot and the output details. When it shows a device fully closed, that is definitive and clears the count. When it shows a device open, it may be stale: a close event arrives before the driver's release finishes.
- **Dependencies: `nix` (features `inotify` and `poll`) and `chrono` (feature `clock` only).** Together they keep the code free of unsafe FFI. Argument parsing and `/proc` parsing are hand-written on purpose. `clap` was considered and left out because of its build cost on the Pi (1 GB RAM). Adding a crate needs a clear readability or correctness gain, and new features should be enabled only as needed.
- **The output format is an interface.** People pipe it into scripts, so keep `YYYY-MM-DD HH:MM:SS playing|stopped[  details]` stable. When a hook is configured, `… hook started|finished|failed for …` lines are mixed in. Every line goes through `log`/`log_at` in `main.rs`, which hold the stdout lock so lines from the two threads never interleave.
- **Hooks** (`--hook PROGRAM`) are executed directly (no shell) with no arguments. The state (`PLAYBACK_STATE`, `playing`/`stopped`) and details are in `PLAYBACK_*` variables. Every variable is always set, even if empty. Don't use `ALSA_*` names: alsa-lib reads variables such as `ALSA_CARD` and `ALSA_PCM_DEVICE`. Hooks run only on a change of state, never for the startup state. They run on one worker thread, strictly in order, so the main loop never blocks on them. Keep that: a blocking hook would delay event handling and skew the tracker's timing. The variables are documented in a README table, and `hook::tests` checks them; keep the two in step.
- **Process names come from argv[0], not `comm`.** shairport-sync renames its main thread to `convolver`.
- **Keep `tracker.rs` free of I/O** so its timing logic stays unit-testable. Any change to how opens, closes or delays are handled needs a tracker test.
- **No unsafe code.** `#![forbid(unsafe_code)]` in `main.rs` enforces this. Use a `nix` wrapper instead of raw `libc`.
- **Wake on time, not early.** `wait_readable()` rounds poll timeouts *up* to whole milliseconds, because nix rounds down. `Tracker::expire()` assumes the deadline has passed.

## Testing

The unit tests cover the tracker logic (debounce, flip-back, close racing `/proc`, unplug, resync) and device-node name parsing. They need no sound card. The inotify and `/proc` plumbing still has to be checked live on a machine with a sound card.

Play **silence only**, never audible audio. On the development Pi, card 0 is a USB audio device:

```bash
aplay -q -D plughw:0,0 -f S16_LE -r 44100 -c 2 -d 2 /dev/zero
```

- **Playback:** 2 s of silence should give a `playing` line, then `stopped` about 1 s after it ends, and with `--hook examples/hook.sh` there should be `hook started`/`hook finished` lines for both. To check the queueing, use a hook that sleeps a few seconds: the `stopped` line must stay on time while its hook starts late.
- **Short blip:** `timeout 0.1 aplay ... /dev/zero` should produce no output (filtered by the start delay).
- **Mid-playback start:** a monitor started while audio is playing should report `playing` immediately.
- **Idle:** utime+stime in `/proc/<pid>/stat` should stay unchanged for 10 s, and `/proc/<pid>/wchan` should be `poll_schedule_timeout`.

Notes:
- `aplay -d` only accepts whole seconds.
- `aplay` fails with "Device or resource busy" while another player holds the device. Wait until `/proc/asound/card0/pcm0p/sub0/status` reads `closed`.
- The players on the Pi are librespot (raspotify), shairport-sync and bluealsa-aplay.
