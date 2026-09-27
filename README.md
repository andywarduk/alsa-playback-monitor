# alsa-playback-monitor

Reports when anything starts or stops playing through ALSA, and can run a command on each change, e.g. to switch an amplifier on and off.

It is event-driven rather than polled. It sleeps in the kernel until a playback device is opened or closed, so it uses no CPU while idle.

```
2026-09-26 15:19:54 stopped
2026-09-26 15:20:20 playing  hw:0,0 bluealsa-aplay[395] PREPARED S16_LE 48000Hz 2ch
2026-09-26 15:20:29 stopped
2026-09-26 15:21:03 playing  hw:0,0 librespot[539] RUNNING S16_LE 44100Hz 2ch
2026-09-26 15:21:09 stopped
2026-09-26 15:22:29 playing  hw:0,0 shairport-sync[554] RUNNING S16_LE 48000Hz 2ch
2026-09-26 15:22:45 stopped
```

## Install

Linux only. It depends on the `nix` crate for inotify and `poll`, and on `chrono` for timestamps.

Run this as your normal user. It builds with cargo, then uses sudo for the install steps:

```bash
./install.sh
```

It installs:
- the binary to `/usr/local/bin/alsa-playback-monitor`
- a systemd service, `alsa-playback-monitor.service`, to `/usr/local/lib/systemd/system`
- the service's hook script, `/etc/alsa-playback-monitor/hook.sh`, copied from [examples/hook.sh](examples/hook.sh), but only if it doesn't already exist, so your edits survive reinstalls

The service is enabled at boot and (re)started immediately. Rerun the script after changing the code.

To install under `/usr` instead, run `PREFIX=/usr ./install.sh`; the hook stays in `/etc`. To remove everything, run `./install.sh uninstall`. It deletes the hook only if it still matches `examples/hook.sh`. An edited hook is kept; delete `/etc/alsa-playback-monitor` yourself if you no longer want it. If you used a `PREFIX`, pass the same one when uninstalling.

## Usage

```bash
alsa-playback-monitor --hook /usr/local/bin/amp.sh --stop-delay 30
```

| Option | Default | Meaning |
|---|---|---|
| `--hook PROGRAM` | | Program to run on each change of state (see [Hooks](#hooks)) |
| `--start-delay SEC` | `0.2` | Playback must last this long before it is reported |
| `--stop-delay SEC` | `1.0` | Silence must last this long before it is reported |

Options also accept the `--option=value` form.

The first line of output is the state at startup. After that there is one line per change:

```
<date> <time> playing  hw:<card>,<device> <program>[<pid>] <ALSA state> <format> <rate>Hz <channels>ch
<date> <time> stopped
```

If several playback streams are open at once, their details are separated by `; `. The format fields are omitted if the player hasn't configured the device yet.

No special permissions are needed: any user can read `/dev/snd` and `/proc/asound`.

## Hooks

The hook program runs once per change of state, with no arguments. It doesn't run for the state at startup. It is run directly, not through a shell, so give the path to an executable script or program. Under systemd that path must be absolute.

The new state and its details are passed in environment variables. All of them are always set, and empty when they don't apply, so scripts can use `set -u`:

| Variable | Example | Meaning |
|---|---|---|
| `PLAYBACK_STATE` | `playing` | The new state: `playing` or `stopped` |
| `PLAYBACK_TIME` | `2026-09-27 10:00:01` | When the change happened, as logged |
| `PLAYBACK_DETAILS` | `hw:0,0 librespot[539] RUNNING S16_LE 44100Hz 2ch` | The logged details of every open stream |
| `PLAYBACK_DEVICE` | `hw:0,0` | The first open stream's device |
| `PLAYBACK_PROGRAM` | `librespot` | The program playing on it |
| `PLAYBACK_PID` | `539` | That program's process id |
| `PLAYBACK_PCM_STATE` | `RUNNING` | The stream's ALSA state (not to be confused with `PLAYBACK_STATE`) |
| `PLAYBACK_FORMAT` | `S16_LE` | Sample format |
| `PLAYBACK_RATE` | `44100` | Sample rate in Hz |
| `PLAYBACK_CHANNELS` | `2` | Channel count |

Everything after `PLAYBACK_DETAILS` describes the first open stream, and is empty on `stopped`. There is normally only one stream. A hardware device plays one stream at a time, so there are more only when several cards or devices are playing at once. `PLAYBACK_PROGRAM` lets one script react differently to Spotify, AirPlay and Bluetooth.

[examples/hook.sh](examples/hook.sh), the service's default hook, is a starting point:

```sh
case "$PLAYBACK_STATE" in
playing) echo "$PLAYBACK_PROGRAM started playing on $PLAYBACK_DEVICE" ;;
stopped) echo "playback stopped at $PLAYBACK_TIME" ;;
esac
```

Hooks run on a separate thread, one at a time and in order. A slow hook never delays the monitor's own lines or timing, and a `stopped` hook never overtakes the `playing` one before it. A hook can therefore start later than its change, so each run is logged. Anything the hook prints appears between these lines:

```
2026-09-27 06:57:57 playing  hw:0,0 aplay[20795] RUNNING S16_LE 44100Hz 2ch
2026-09-27 06:57:57 hook started for playing, 0.0s after the change
2026-09-27 06:57:59 stopped
2026-09-27 06:58:00 hook finished for playing after 3.0s: exit 0
2026-09-27 06:58:00 hook started for stopped, 1.2s after the change
2026-09-27 06:58:03 hook finished for stopped after 3.0s: exit 0
```

A hook that can't be started logs `hook failed for …` with the reason, such as a missing file or missing execute permission.

## The service

The service's output goes to the journal. `./log.sh` prints it, and passes any extra arguments on to `journalctl`:

```bash
./log.sh                 # everything so far
./log.sh -f              # follow new entries
./log.sh --since today
```

The service runs `/etc/alsa-playback-monitor/hook.sh`, so to change what happens on each change, edit that script. No restart is needed: it's run afresh each time.

To change the options instead, such as the stop delay or which hook runs, override `ExecStart` with a drop-in:

```bash
sudo systemctl edit alsa-playback-monitor
```

In the editor, add:

```ini
[Service]
ExecStart=
ExecStart=/usr/local/bin/alsa-playback-monitor --hook /etc/alsa-playback-monitor/hook.sh --stop-delay 30
```

The empty `ExecStart=` line is required: it clears the original command. The override is stored in `/etc/systemd/system/alsa-playback-monitor.service.d/` and survives reinstalls.

The service runs as a throwaway unprivileged user (`DynamicUser=yes`), and the hook runs as that user too. So the hook script must be readable and executable by everyone (`chmod 755`, as installed), and so must every directory above it. The sandbox has these limits:
- the filesystem is read-only
- home directories are readable, not writable
- `/tmp` is private
- `sudo` and other setuid programs don't work

A hook that needs more access can get it in the same drop-in:
- **Hardware access:** add `SupplementaryGroups=gpio`, or whichever group owns the device the hook uses.
- **Full access:** add `DynamicUser=no` and `User=ajw` to run hooks as you, without the sandbox.

## How it works

- An inotify watch on `/dev/snd` reports every open and close of a playback node (`pcmC<card>D<device>p`). It also picks up cards that are hot-plugged later.
- The monitor keeps a count of open handles per device from those events. It doesn't reread `/proc` after a close because the close event fires *before* the driver's release runs, so `/proc` can briefly still show the stream as open.
- `/proc/asound/card*/pcm*p/sub*/status` provides the startup state and the details (owner, state, format). It also corrects the counts when it shows a device fully closed.
- The start and stop delays act as a debounce: a change is reported only after it has held for the whole delay. This filters out momentary opens such as a player probing the device.

## Limitations

- **"Playing" means "a playback device is open".** ALSA raises no event for pause or resume inside an open stream. Players that close the device when they stop (librespot, shairport-sync, bluealsa-aplay and aplay all do) are tracked accurately. A player that holds the device open while paused shows as playing until it closes it.
- **Only hardware devices are watched.** With dmix or PipeWire, the monitor sees the hardware device being opened, not each app behind it. PipeWire also keeps the device open for a few seconds after the audio stops, so "stopped" is reported late.
