use std::fmt;

/// What an adapter does about a message that does not follow the schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Do not check. Nothing is parsed on validation's behalf.
    Off,
    /// Check and report, but carry the message anyway.
    ///
    /// The default where a schema is loaded: a gateway that holds the standard
    /// and stays quiet about a producer departing from it is the silent kind of
    /// wrong, while refusing traffic that flowed yesterday is a decision an
    /// operator should make deliberately.
    #[default]
    Warn,
    /// Refuse the message and tell the peer.
    Reject,
}

impl Mode {
    /// Whether this mode will run [`super::validate`].
    #[must_use]
    pub fn is_on(self) -> bool {
        !matches!(self, Self::Off)
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "off",
            Self::Warn => "warn",
            Self::Reject => "reject",
        })
    }
}

impl std::str::FromStr for Mode {
    type Err = String;

    /// `off`, `warn`, or `reject`.
    ///
    /// # Errors
    ///
    /// Returns a message if `s` is none of those.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "off" => Ok(Self::Off),
            "warn" => Ok(Self::Warn),
            "reject" => Ok(Self::Reject),
            other => Err(format!(
                "unknown validation mode '{other}'; expected off, warn, or reject"
            )),
        }
    }
}
