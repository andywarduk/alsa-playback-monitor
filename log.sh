#!/bin/bash
# Print the alsa-playback-monitor service's journal.
#
# Extra arguments are passed to journalctl, and later options override the
# defaults here, e.g.:
#   ./log.sh -f              follow new entries
#   ./log.sh --since today   today's entries only
#   ./log.sh -o short        add the journal's own timestamps and PIDs
#
# The monitor's lines carry their own timestamps, so the default output shows
# messages only. Reading the system journal needs membership of the adm or
# systemd-journal group (or sudo).

exec journalctl --unit alsa-playback-monitor --output cat --no-pager "$@"
