#!/bin/sh
# Hook for alsa-playback-monitor, run on each change of playback state.
#
# install.sh copies this to /etc/alsa-playback-monitor/hook.sh, which the
# service runs, but only if that file doesn't already exist, so edit the
# installed copy freely. The state and its details are in PLAYBACK_*
# environment variables (see README.md). Replace the echo lines with whatever
# should happen, such as switching an amplifier on and off. Anything this
# prints goes to the monitor's output (the journal, under systemd).

case "$PLAYBACK_STATE" in
playing)
    echo "$PLAYBACK_PROGRAM started playing on $PLAYBACK_DEVICE" \
        "($PLAYBACK_FORMAT ${PLAYBACK_RATE}Hz ${PLAYBACK_CHANNELS}ch) at $PLAYBACK_TIME"
    ;;
stopped)
    echo "playback stopped at $PLAYBACK_TIME"
    ;;
esac
