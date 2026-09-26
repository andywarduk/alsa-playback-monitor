//! Command-line options.

use std::process;
use std::time::Duration;

const USAGE: &str = "\
usage: alsa-playback-monitor [--on-start CMD] [--on-stop CMD] [--start-delay SEC] [--stop-delay SEC]

Report when ALSA playback starts and stops.

options:
  --on-start CMD     shell command to run when playback starts
  --on-stop CMD      shell command to run when playback stops
  --start-delay SEC  playback must last this long before it is reported (default 0.2)
  --stop-delay SEC   silence must last this long before it is reported (default 1.0)
";

pub struct Args {
    pub on_start: Option<String>,
    pub on_stop: Option<String>,
    pub start_delay: Duration,
    pub stop_delay: Duration,
}

impl Args {
    /// Parse the process arguments, exiting with usage on error or `--help`.
    pub fn parse() -> Self {
        let mut args = Self {
            on_start: None,
            on_stop: None,
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
                "--on-start" | "--on-stop" | "--start-delay" | "--stop-delay" => {}
                _ => usage_error(&format!("unrecognized argument: {arg}")),
            }
            let Some(value) = inline.or_else(|| argv.next()) else {
                usage_error(&format!("{flag} needs a value"));
            };
            match flag {
                "--on-start" => args.on_start = Some(value),
                "--on-stop" => args.on_stop = Some(value),
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
