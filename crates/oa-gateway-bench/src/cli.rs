//! Hand-rolled argv, in the same style as the host binary.

use std::path::PathBuf;
use std::time::Duration;

/// Operator-facing usage text for `-h` / `--help`.
pub(crate) const USAGE: &str = "\
OA-Gateway bench — latency and throughput against public gateway APIs

Usage:
  oa-gateway-bench engine   [options]
  oa-gateway-bench loopback [options]
  oa-gateway-bench owp      [options]
  oa-gateway-bench uci      [options]

Shared options:
  --duration DUR       How long to publish (default 10s). 200ms, 10s, 1m, or seconds.
  --warmup DUR         Discard latency samples sent in this prefix of --duration (default 1s).
  --rate N             Publishes per second. 0 means as-fast-as-possible (default 0).
  --payload-bytes N    Minimum JSON payload size (default 64).
  --json PATH          Write the same numbers as one JSON object.
  -h, --help           Print this help and exit.

engine options:
  --subscribers N      How many engine subscribers (default 1).
  --capacity N         Delivery channel size passed to Engine::subscribe (default 4096).

owp options:
  --publishers N       WebSocket publishers (default 1).
  --subscribers N      WebSocket subscribers (default 1).
  --xml-baseline       Embedded OWP converts OMS JSON ↔ UCI XML (fixture schema).
  --url URL            Attach to a running gateway instead of starting OWP.
  --ack-latency        Also time PUB → +OK on the publisher socket.

uci options:
  --iterations N       Convert this many times (default 2000).
  --direction DIR      json-to-xml (default) or xml-to-json.

Examples:
  oa-gateway-bench engine --duration 5s --warmup 1s
  oa-gateway-bench engine --capacity 64
  oa-gateway-bench owp --xml-baseline --duration 5s
  oa-gateway-bench owp --url ws://127.0.0.1:9000/
  oa-gateway-bench uci --iterations 2000
";

/// Parsed command line.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Command {
    Engine(EngineArgs),
    Loopback(LoopbackArgs),
    Owp(OwpArgs),
    Uci(UciArgs),
    Help,
}

/// Flags shared by the duration-based scenarios.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SharedArgs {
    pub duration: Duration,
    pub warmup: Duration,
    pub rate: u64,
    pub payload_bytes: usize,
    pub json: Option<PathBuf>,
}

impl Default for SharedArgs {
    fn default() -> Self {
        Self {
            duration: Duration::from_secs(10),
            warmup: Duration::from_secs(1),
            rate: 0,
            payload_bytes: 64,
            json: None,
        }
    }
}

/// `engine` scenario.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EngineArgs {
    pub shared: SharedArgs,
    pub subscribers: usize,
    pub capacity: usize,
}

impl Default for EngineArgs {
    fn default() -> Self {
        Self {
            shared: SharedArgs::default(),
            subscribers: 1,
            capacity: 4096,
        }
    }
}

/// `loopback` scenario.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct LoopbackArgs {
    pub shared: SharedArgs,
}

/// `owp` scenario.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OwpArgs {
    pub shared: SharedArgs,
    pub publishers: usize,
    pub subscribers: usize,
    pub xml_baseline: bool,
    pub url: Option<String>,
    pub ack_latency: bool,
}

impl Default for OwpArgs {
    fn default() -> Self {
        Self {
            shared: SharedArgs::default(),
            publishers: 1,
            subscribers: 1,
            xml_baseline: false,
            url: None,
            ack_latency: false,
        }
    }
}

/// Convert direction for `uci`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UciDirection {
    JsonToXml,
    XmlToJson,
}

/// `uci` scenario.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UciArgs {
    pub iterations: u64,
    pub direction: UciDirection,
    pub json: Option<PathBuf>,
}

impl Default for UciArgs {
    fn default() -> Self {
        Self {
            iterations: 2000,
            direction: UciDirection::JsonToXml,
            json: None,
        }
    }
}

/// Parses arguments after the program name.
///
/// # Errors
///
/// Returns a message if the command is missing, a flag is unknown, or a
/// value cannot be parsed.
pub(crate) fn parse_args<I>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter().peekable();
    let Some(first) = args.next() else {
        return Err("missing command. Try `oa-gateway-bench --help`.".into());
    };
    match first.as_str() {
        "-h" | "--help" => Ok(Command::Help),
        "engine" => Ok(Command::Engine(parse_engine(&mut args)?)),
        "loopback" => Ok(Command::Loopback(parse_loopback(&mut args)?)),
        "owp" => Ok(Command::Owp(parse_owp(&mut args)?)),
        "uci" => Ok(Command::Uci(parse_uci(&mut args)?)),
        other if other.starts_with('-') => Err(format!(
            "unknown option `{other}`. Try `oa-gateway-bench --help`."
        )),
        other => Err(format!(
            "unknown command `{other}`. Try `oa-gateway-bench --help`."
        )),
    }
}

fn parse_engine<I>(args: &mut I) -> Result<EngineArgs, String>
where
    I: Iterator<Item = String>,
{
    let mut out = EngineArgs::default();
    let mut rest = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(USAGE.trim_end().to_owned()),
            "--subscribers" => out.subscribers = parse_nonzero(next(args, "--subscribers")?)?,
            "--capacity" => out.capacity = parse_nonzero(next(args, "--capacity")?)?,
            other => rest.push(other.to_owned()),
        }
    }
    out.shared = parse_shared(rest)?;
    Ok(out)
}

fn parse_loopback<I>(args: &mut I) -> Result<LoopbackArgs, String>
where
    I: Iterator<Item = String>,
{
    let rest: Vec<String> = args.collect();
    if rest.iter().any(|a| a == "-h" || a == "--help") {
        return Err(USAGE.trim_end().to_owned());
    }
    Ok(LoopbackArgs {
        shared: parse_shared(rest)?,
    })
}

fn parse_owp<I>(args: &mut I) -> Result<OwpArgs, String>
where
    I: Iterator<Item = String>,
{
    let mut out = OwpArgs::default();
    let mut rest = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(USAGE.trim_end().to_owned()),
            "--publishers" => out.publishers = parse_nonzero(next(args, "--publishers")?)?,
            "--subscribers" => out.subscribers = parse_nonzero(next(args, "--subscribers")?)?,
            "--xml-baseline" => out.xml_baseline = true,
            "--url" => out.url = Some(next(args, "--url")?),
            "--ack-latency" => out.ack_latency = true,
            other => rest.push(other.to_owned()),
        }
    }
    out.shared = parse_shared(rest)?;
    Ok(out)
}

fn parse_uci<I>(args: &mut I) -> Result<UciArgs, String>
where
    I: Iterator<Item = String>,
{
    let mut out = UciArgs::default();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(USAGE.trim_end().to_owned()),
            "--iterations" => out.iterations = parse_u64(&next(&mut args, "--iterations")?)?,
            "--direction" => {
                out.direction = match next(&mut args, "--direction")?.as_str() {
                    "json-to-xml" => UciDirection::JsonToXml,
                    "xml-to-json" => UciDirection::XmlToJson,
                    other => {
                        return Err(format!(
                            "--direction must be json-to-xml or xml-to-json, not `{other}`"
                        ))
                    }
                }
            }
            "--json" => out.json = Some(PathBuf::from(next(&mut args, "--json")?)),
            other => {
                return Err(format!(
                    "unknown option `{other}` for uci. Try `oa-gateway-bench --help`."
                ))
            }
        }
    }
    Ok(out)
}

fn parse_shared(args: Vec<String>) -> Result<SharedArgs, String> {
    let mut out = SharedArgs::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--duration" => out.duration = parse_duration(&next(&mut args, "--duration")?)?,
            "--warmup" => out.warmup = parse_duration(&next(&mut args, "--warmup")?)?,
            "--rate" => out.rate = parse_u64(&next(&mut args, "--rate")?)?,
            "--payload-bytes" => {
                out.payload_bytes = parse_nonzero(next(&mut args, "--payload-bytes")?)?
            }
            "--json" => out.json = Some(PathBuf::from(next(&mut args, "--json")?)),
            other => {
                return Err(format!(
                    "unknown option `{other}`. Try `oa-gateway-bench --help`."
                ))
            }
        }
    }
    Ok(out)
}

fn next<I>(args: &mut I, flag: &str) -> Result<String, String>
where
    I: Iterator<Item = String>,
{
    args.next()
        .ok_or_else(|| format!("{flag} needs a value. Try `oa-gateway-bench --help`."))
}

fn parse_u64(s: &str) -> Result<u64, String> {
    s.parse()
        .map_err(|_| format!("`{s}` is not a non-negative integer"))
}

fn parse_nonzero(s: String) -> Result<usize, String> {
    let n: usize = s
        .parse()
        .map_err(|_| format!("`{s}` is not a positive integer"))?;
    if n == 0 {
        return Err("value must be at least 1".into());
    }
    Ok(n)
}

/// Parses `200ms`, `10s`, `1m`, or a number of seconds (integer or float).
pub(crate) fn parse_duration(s: &str) -> Result<Duration, String> {
    if let Some(ms) = s.strip_suffix("ms") {
        let n: u64 = ms.parse().map_err(|_| format!("`{s}` is not a duration"))?;
        return Ok(Duration::from_millis(n));
    }
    if let Some(secs) = s.strip_suffix('s') {
        return parse_secs(secs, s);
    }
    if let Some(mins) = s.strip_suffix('m') {
        let n: f64 = mins
            .parse()
            .map_err(|_| format!("`{s}` is not a duration"))?;
        return Ok(Duration::from_secs_f64(n * 60.0));
    }
    parse_secs(s, s)
}

fn parse_secs(text: &str, original: &str) -> Result<Duration, String> {
    let n: f64 = text
        .parse()
        .map_err(|_| format!("`{original}` is not a duration"))?;
    if n < 0.0 {
        return Err(format!("`{original}` is not a duration"));
    }
    Ok(Duration::from_secs_f64(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn help_is_recognized() {
        assert_eq!(parse_args(args(&["--help"])).unwrap(), Command::Help);
        assert_eq!(parse_args(args(&["-h"])).unwrap(), Command::Help);
    }

    #[test]
    fn engine_defaults_and_overrides() {
        let Command::Engine(e) = parse_args(args(&["engine"])).unwrap() else {
            panic!("expected engine");
        };
        assert_eq!(e.subscribers, 1);
        assert_eq!(e.capacity, 4096);
        assert_eq!(e.shared.duration, Duration::from_secs(10));

        let Command::Engine(e) = parse_args(args(&[
            "engine",
            "--duration",
            "200ms",
            "--warmup",
            "0",
            "--subscribers",
            "2",
            "--capacity",
            "64",
        ]))
        .unwrap() else {
            panic!("expected engine");
        };
        assert_eq!(e.shared.duration, Duration::from_millis(200));
        assert_eq!(e.shared.warmup, Duration::ZERO);
        assert_eq!(e.subscribers, 2);
        assert_eq!(e.capacity, 64);
    }

    #[test]
    fn owp_flags() {
        let Command::Owp(o) = parse_args(args(&[
            "owp",
            "--xml-baseline",
            "--ack-latency",
            "--url",
            "ws://127.0.0.1:9000/",
        ]))
        .unwrap() else {
            panic!("expected owp");
        };
        assert!(o.xml_baseline);
        assert!(o.ack_latency);
        assert_eq!(o.url.as_deref(), Some("ws://127.0.0.1:9000/"));
    }

    #[test]
    fn unknown_command_is_rejected() {
        let err = parse_args(args(&["nope"])).unwrap_err();
        assert!(err.contains("nope"), "{err}");
    }

    #[test]
    fn parse_duration_units() {
        assert_eq!(parse_duration("200ms").unwrap(), Duration::from_millis(200));
        assert_eq!(parse_duration("10s").unwrap(), Duration::from_secs(10));
        assert_eq!(parse_duration("1m").unwrap(), Duration::from_secs(60));
        assert_eq!(parse_duration("2").unwrap(), Duration::from_secs(2));
    }
}
