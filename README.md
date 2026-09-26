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

```bash
cargo install --path .
```

The binary is installed to `~/.cargo/bin/alsa-playback-monitor`.

## Usage

```bash
alsa-playback-monitor --on-start 'amp-on.sh' --on-stop 'amp-off.sh' --stop-delay 30
```

| Option | Default | Meaning |
|---|---|---|
| `--on-start CMD` | | Shell command to run when playback starts |
| `--on-stop CMD` | | Shell command to run when playback stops |
| `--start-delay SEC` | `0.2` | Playback must last this long before it is reported |
| `--stop-delay SEC` | `1.0` | Silence must last this long before it is reported |

Options also accept the `--option=value` form.

The first line of output is the state at startup. After that there is one line per change:

```
<date> <time> playing  hw:<card>,<device> <program>[<pid>] <ALSA state> <format> <rate>Hz <channels>ch
<date> <time> stopped
```

If several playback streams are open at once, their details are separated by `; `. The format fields are omitted if the player hasn't configured the device yet.

Commands run through `/bin/sh -c`, and only when the state changes. They do not run for the startup state. They run one at a time, and the monitor waits for each to finish, so a stop command can never overtake the start command before it.

No special permissions are needed: any user can read `/dev/snd` and `/proc/asound`.

## How it works

- An inotify watch on `/dev/snd` reports every open and close of a playback node (`pcmC<card>D<device>p`). It also picks up cards that are hot-plugged later.
- The monitor keeps a count of open handles per device from those events. It doesn't reread `/proc` after a close because the close event fires *before* the driver's release runs, so `/proc` can briefly still show the stream as open.
- `/proc/asound/card*/pcm*p/sub*/status` provides the startup state and the details (owner, state, format). It also corrects the counts when it shows a device fully closed.
- The start and stop delays act as a debounce: a change is reported only after it has held for the whole delay. This filters out momentary opens such as a player probing the device.

## Limitations

- **"Playing" means "a playback device is open".** ALSA raises no event for pause or resume inside an open stream. Players that close the device when they stop (librespot, shairport-sync, bluealsa-aplay and aplay all do) are tracked accurately. A player that holds the device open while paused shows as playing until it closes it.
- **Only hardware devices are watched.** With dmix or PipeWire, the monitor sees the hardware device being opened, not each app behind it. PipeWire also keeps the device open for a few seconds after the audio stops, so "stopped" is reported late.
