//! What /proc/asound says about open playback devices, and who owns them.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// A playback PCM device: (card, device).
pub type Node = (u32, u32);
type Kv = HashMap<String, String>;
/// Open substreams per playback device: (substream dir, parsed status).
pub type Active = BTreeMap<Node, Vec<(PathBuf, Kv)>>;

/// (card, device) for a /dev/snd playback node name such as "pcmC0D0p".
pub fn parse_playback_node(name: &str) -> Option<Node> {
    let (card, dev) = name.strip_prefix("pcmC")?.strip_suffix('p')?.split_once('D')?;
    Some((numbered(card, "", "")?, numbered(dev, "", "")?))
}

/// Every open playback substream, from /proc/asound/card*/pcm*p/sub*/status.
pub fn open_substreams() -> Active {
    let mut active = Active::new();
    for (card_dir, card) in numbered_entries(Path::new("/proc/asound"), "card", "") {
        for (pcm_dir, dev) in numbered_entries(&card_dir, "pcm", "p") {
            for (sub_dir, _) in numbered_entries(&pcm_dir, "sub", "") {
                let status = read_kv(&sub_dir.join("status"));
                if !status.is_empty() {
                    active.entry((card, dev)).or_default().push((sub_dir, status));
                }
            }
        }
    }
    active
}

/// Number of open substreams per device.
pub fn count_open(active: &Active) -> HashMap<Node, u32> {
    active.iter().map(|(&node, subs)| (node, u32::try_from(subs.len()).unwrap_or(u32::MAX))).collect()
}

/// One open playback substream.
pub struct Stream {
    pub card: u32,
    pub device: u32,
    /// Program name (argv[0]), if the owning process could be read.
    pub program: Option<String>,
    /// Owning process id: the thread group id when known.
    pub pid: String,
    /// ALSA PCM state, e.g. RUNNING or PREPARED.
    pub state: String,
    /// Absent until the player has configured the device.
    pub hw: Option<HwParams>,
}

pub struct HwParams {
    pub format: String,
    pub rate: String,
    pub channels: String,
}

/// "hw:C,D program[pid] STATE FORMAT RATEHz CHANNELSch"
impl fmt::Display for Stream {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "hw:{},{} ", self.card, self.device)?;
        match &self.program {
            Some(program) => write!(f, "{program}[{}]", self.pid)?,
            None => write!(f, "pid {}", self.pid)?,
        }
        if !self.state.is_empty() {
            write!(f, " {}", self.state)?;
        }
        if let Some(hw) = &self.hw {
            write!(f, " {} {}Hz {}ch", hw.format, hw.rate, hw.channels)?;
        }
        Ok(())
    }
}

/// Details of every open substream in `active`.
pub fn streams(active: &Active) -> Vec<Stream> {
    let mut streams = Vec::new();
    for (&(card, device), subs) in active {
        for (sub_dir, status) in subs {
            let params = read_kv(&sub_dir.join("hw_params")); // empty until configured
            let hw = match (params.get("format"), params.get("rate"), params.get("channels")) {
                (Some(format), Some(rate), Some(channels)) => Some(HwParams {
                    format: format.clone(),
                    rate: rate.split_whitespace().next().unwrap_or(rate).to_owned(),
                    channels: channels.clone(),
                }),
                _ => None,
            };
            let (program, pid) = owner(status.get("owner_pid").map_or("?", String::as_str));
            let state = status.get("state").cloned().unwrap_or_default();
            streams.push(Stream { card, device, program, pid, state, hw });
        }
    }
    streams
}

/// The streams as logged: each one's text, joined with "; ".
pub fn describe(streams: &[Stream]) -> String {
    streams.iter().map(Stream::to_string).collect::<Vec<_>>().join("; ")
}

/// Parse a /proc "key: value" file; empty if closed/unreadable.
fn read_kv(path: &Path) -> Kv {
    let Ok(text) = fs::read_to_string(path) else {
        return Kv::new();
    };
    text.lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect()
}

/// The N in a name like "card0", "pcm0p" or "sub0", given its prefix and suffix.
fn numbered(name: &str, prefix: &str, suffix: &str) -> Option<u32> {
    let digits = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Entries of `dir` named <prefix>N<suffix>, sorted by N.
fn numbered_entries(dir: &Path, prefix: &str, suffix: &str) -> Vec<(PathBuf, u32)> {
    let mut found: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let n = numbered(entry.file_name().to_str()?, prefix, suffix)?;
            Some((entry.path(), n))
        })
        .collect();
    found.sort_by_key(|&(_, n)| n);
    found
}

/// Program name and process id of the owner of `pid` (which may be a thread
/// id). The name is `None`, and the pid returned unchanged, if it can't be read.
fn owner(pid: &str) -> (Option<String>, String) {
    let status = read_kv(Path::new(&format!("/proc/{pid}/status")));
    let tgid = status.get("Tgid").map_or(pid, String::as_str);
    // argv[0] rather than comm: some players rename their main thread
    // (shairport-sync's comm is "convolver").
    let argv0 = fs::read(format!("/proc/{tgid}/cmdline")).ok().and_then(|cmdline| {
        let argv0 = cmdline.split(|&b| b == 0).next().filter(|a| !a.is_empty())?;
        Some(String::from_utf8_lossy(argv0).into_owned())
    });
    let name = argv0.or_else(|| {
        fs::read_to_string(format!("/proc/{tgid}/comm")).ok().map(|comm| comm.trim().to_owned())
    });
    match name {
        Some(name) => (Some(name.rsplit('/').next().unwrap_or(&name).to_owned()), tgid.to_owned()),
        None => (None, pid.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_text() {
        let configured = Stream {
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
        let unconfigured = Stream {
            card: 1,
            device: 0,
            program: None,
            pid: "7".to_owned(),
            state: "OPEN".to_owned(),
            hw: None,
        };
        assert_eq!(configured.to_string(), "hw:0,0 librespot[539] RUNNING S16_LE 44100Hz 2ch");
        assert_eq!(unconfigured.to_string(), "hw:1,0 pid 7 OPEN");
        assert_eq!(
            describe(&[configured, unconfigured]),
            "hw:0,0 librespot[539] RUNNING S16_LE 44100Hz 2ch; hw:1,0 pid 7 OPEN"
        );
    }

    #[test]
    fn playback_node_names() {
        assert_eq!(parse_playback_node("pcmC0D0p"), Some((0, 0)));
        assert_eq!(parse_playback_node("pcmC12D3p"), Some((12, 3)));
        for name in ["pcmC0D0c", "controlC0", "pcmC0D0p.bak", "pcmCD0p", "pcmC+1D0p", "by-path"] {
            assert_eq!(parse_playback_node(name), None, "{name}");
        }
    }
}
