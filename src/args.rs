//! Command-line options.

use std::process;
use std::time::Duration;

const USAGE: &str = "\
usage: alsa-playback-monitor [--hook PROGRAM] [--start-delay SEC] [--stop-delay SEC]

Report when ALSA playback starts and stops.

options:
  --hook PROGRAM     program to run on each change; the new state (\"playing\" or
                     \"stopped\") and details are in PLAYBACK_* variables
  --start-delay SEC  playback must last this long before it is reported (default 0.2)
  --stop-delay SEC   silence must last this long before it is reported (default 1.0)
";

pub struct Args {
    pub hook: Option<String>,
    pub start_delay: Duration,
    pub stop_delay: Duration,
}

impl Args {
    /// Parse the process arguments, exiting with usage on error or `--help`.
    pub fn parse() -> Self {
        let mut args = Self {
            hook: None,
            start_delay: Duration::from_millis(200),
            stop_delay: Duration::from_secs(1),
        };
        let mut argv = std::env::args().skip(1);
        while let Some(arg) = argv.next() {
            let (flag, inline) = match arg.split_once('=') {
                Some((flag, value)) => (flag, Some(value.to_owned())),
                None => (arg.as_str(), None),
            };
            match flag {
                "-h" | "--help" => {
                    print!("{USAGE}");
                    process::exit(0);
                }
                "--hook" | "--start-delay" | "--stop-delay" => {}
                _ => usage_error(&format!("unrecognized argument: {arg}")),
            }
            let Some(value) = inline.or_else(|| argv.next()) else {
                usage_error(&format!("{flag} needs a value"));
            };
            match flag {
                "--hook" => args.hook = Some(value).filter(|program| !program.is_empty()),
                "--start-delay" => args.start_delay = seconds(flag, &value),
                _ => args.stop_delay = seconds(flag, &value),
            }
        }
        args
    }
}

fn usage_error(msg: &str) -> ! {
    eprint!("{USAGE}");
    eprintln!("error: {msg}");
    process::exit(2);
}

fn seconds(flag: &str, value: &str) -> Duration {
    match value.parse::<f64>() {
        Ok(secs) if secs.is_finite() && secs >= 0.0 => Duration::from_secs_f64(secs),
        _ => usage_error(&format!("{flag}: invalid number of seconds: {value:?}")),
    }
}
