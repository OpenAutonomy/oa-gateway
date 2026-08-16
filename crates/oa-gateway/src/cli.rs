//! Parses argv into help, version, or a config path.
//!
//! The binary takes one positional argument and no optional flags beyond
//! `--help` and `--version`. Unknown flags are errors, not config paths,
//! so a typo is not treated as a missing file.

use std::path::PathBuf;

/// Operator-facing usage text for `-h` / `--help`.
///
/// Kept as a single string so `--help` and a missing-argument hint can
/// point at the same wording.
pub(crate) const USAGE: &str = "\
OA-Gateway — protocol-agnostic routing with pluggable adapters (prototype)

Usage:
  oa-gateway CONFIG

Arguments:
  CONFIG           Path to a TOML config file (required)

Options:
  -h, --help       Print this help and exit
  -V, --version    Print the version and exit

Examples:
  oa-gateway config/default.toml   OWP on ws://127.0.0.1:9000/ (no broker needed)
  oa-gateway config/asb.toml       adds a STOMP client for ActiveMQ on :61613

Set RUST_LOG to change log filtering (default: info).
";

/// Outcome of parsing the command line.
///
/// Help and version are first-class results rather than errors so the
/// process can exit 0 after printing.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Cli {
    /// Run the gateway with this config file.
    Run(PathBuf),
    /// Print usage and exit successfully.
    Help,
    /// Print the crate version and exit successfully.
    Version,
}

/// Parses command-line arguments after the program name.
///
/// A single positional path means run. `-h`/`--help` and `-V`/`--version`
/// win immediately. Anything else that looks like a flag is rejected so it
/// is not mistaken for a config path.
///
/// # Errors
///
/// Returns an error if a flag is unknown, a second positional argument is
/// present, or no config path was given.
pub(crate) fn parse_args<I>(args: I) -> Result<Cli, String>
where
    I: IntoIterator<Item = String>,
{
    let mut config = None;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Cli::Help),
            "-V" | "--version" => return Ok(Cli::Version),
            flag if flag.starts_with('-') => {
                return Err(format!("unknown option `{flag}`. Try `oa-gateway --help`."))
            }
            path if config.is_none() => config = Some(PathBuf::from(path)),
            extra => {
                return Err(format!(
                    "unexpected second argument `{extra}`. oa-gateway takes one config path."
                ))
            }
        }
    }
    match config {
        Some(path) => Ok(Cli::Run(path)),
        None => Err("missing CONFIG. Try `oa-gateway --help`.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn help_and_version_flags_are_recognized() {
        assert_eq!(parse_args(args(&["--help"])).unwrap(), Cli::Help);
        assert_eq!(parse_args(args(&["-h"])).unwrap(), Cli::Help);
        assert_eq!(parse_args(args(&["--version"])).unwrap(), Cli::Version);
        assert_eq!(parse_args(args(&["-V"])).unwrap(), Cli::Version);
    }

    #[test]
    fn config_path_is_required() {
        let err = parse_args(args(&[])).unwrap_err();
        assert!(err.contains("missing CONFIG"), "{err}");
        assert_eq!(
            parse_args(args(&["config/asb.toml"])).unwrap(),
            Cli::Run(PathBuf::from("config/asb.toml"))
        );
    }

    #[test]
    fn unknown_flag_is_not_taken_as_a_config_path() {
        let err = parse_args(args(&["--verbose"])).unwrap_err();
        assert!(err.contains("--verbose"), "{err}");
    }

    #[test]
    fn second_positional_argument_is_rejected() {
        let err = parse_args(args(&["a.toml", "b.toml"])).unwrap_err();
        assert!(err.contains("b.toml"), "{err}");
    }
}
