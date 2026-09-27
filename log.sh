#!/bin/bash
# Print the alsa-playback-monitor service's journal.
#
# Extra arguments are passed to journalctl, and later options override the
# defaults here, e.g.:
#   ./log.sh -f              follow new entries
#   ./log.sh --since today   today's entries only
#   ./log.sh -o cat          messages only, without timestamps
#
# Under systemd the monitor leaves timestamps to the journal. (Entries logged
# before it did so carry both.) Reading the system journal needs membership of
# the adm or systemd-journal group (or sudo).

exec journalctl --unit alsa-playback-monitor --no-hostname --no-pager "$@"
