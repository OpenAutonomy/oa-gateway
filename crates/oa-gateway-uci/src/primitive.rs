//! What each `xs:` primitive accepts.
//!
//! Two callers need this and need to agree. Conversion asks how to carry a value
//! — whether `42` belongs in JSON as a number or a string — and validation asks
//! whether the value fits the type at all. Keeping the answer in one place is
//! what stops them from drifting: the XML reader once knew about `xs:int` and not
//! `xs:unsignedInt`, so the catalog's most common integer type arrived at every
//! JSON client as a quoted string.
//!
//! The lexical spaces here are XSD 1.0's, Part 2. They are checked to the extent
//! that a value can be told apart from a value of another type, which includes
//! the calendar: `2026-02-30` is not a date, and saying so needs more than a
//! shape.

use std::sync::OnceLock;

use regex::Regex;

/// What a primitive accepts, in the terms conversion and validation both need.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// `true`, `false`, `1`, `0`, and nothing else.
    Boolean,
    /// An integer within these bounds, inclusive. `i128` holds every XSD integer
    /// type with a bound, including `xs:unsignedLong`; `xs:integer` itself has
    /// none and is given the widest range this can express.
    Integer {
        min: i128,
        max: i128,
    },
    /// A decimal or floating value.
    Number {
        /// Whether `1.5E3` is in the lexical space. `xs:decimal` says no.
        exponent: bool,
        /// Whether `INF`, `-INF` and `NaN` are. Only the floating types say yes.
        special: bool,
    },
    DateTime,
    Date,
    Time,
    Duration,
    HexBinary,
    /// Text, with nothing to check beyond the facets of the type declaring it.
    Text,
}

/// What `primitive` accepts. An unrecognized name is treated as text, which is
/// what [`Kind::Text`] means: no opinion, rather than a guess.
#[must_use]
pub fn kind(primitive: &str) -> Kind {
    // Bounds are the value spaces XSD 1.0 Part 2 states for each type.
    let int = |min: i128, max: i128| Kind::Integer { min, max };
    match primitive {
        "xs:boolean" => Kind::Boolean,
        "xs:byte" => int(-128, 127),
        "xs:short" => int(-32_768, 32_767),
        "xs:int" => int(-2_147_483_648, 2_147_483_647),
        "xs:long" => int(i128::from(i64::MIN), i128::from(i64::MAX)),
        "xs:unsignedByte" => int(0, 255),
        "xs:unsignedShort" => int(0, 65_535),
        "xs:unsignedInt" => int(0, 4_294_967_295),
        "xs:unsignedLong" => int(0, i128::from(u64::MAX)),
        "xs:nonNegativeInteger" => int(0, i128::MAX),
        "xs:positiveInteger" => int(1, i128::MAX),
        "xs:nonPositiveInteger" => int(i128::MIN, 0),
        "xs:negativeInteger" => int(i128::MIN, -1),
        "xs:integer" => int(i128::MIN, i128::MAX),
        "xs:decimal" => Kind::Number {
            exponent: false,
            special: false,
        },
        "xs:double" | "xs:float" => Kind::Number {
            exponent: true,
            special: true,
        },
        "xs:dateTime" => Kind::DateTime,
        "xs:date" => Kind::Date,
        "xs:time" => Kind::Time,
        "xs:duration" => Kind::Duration,
        "xs:hexBinary" => Kind::HexBinary,
        _ => Kind::Text,
    }
}

/// Whether this build can tell a value of this kind from anything else.
///
/// `false` for [`Kind::Text`], which covers `xs:string` and everything the list
/// above does not name — `xs:base64Binary`, `xs:anyURI`, `xs:QName`. A program
/// whose message set uses one of those gets facet checking on it and nothing
/// more, which is worth knowing rather than assuming.
#[must_use]
pub fn is_checked(primitive: &str) -> bool {
    kind(primitive) != Kind::Text
}

/// Why `text` is not a value of this kind, phrased as what was expected.
///
/// `None` when it is one. Numbers are checked lexically and then by range, so an
/// `xs:int` of `99999999999999` is reported for the range rather than the shape.
#[must_use]
pub fn refuses(kind: &Kind, text: &str) -> Option<String> {
    match kind {
        Kind::Boolean => match text {
            "true" | "false" | "1" | "0" => None,
            _ => Some("one of 'true', 'false', '1' or '0'".into()),
        },
        Kind::Integer { min, max } => {
            if !integral(text) {
                return Some("a whole number, optionally signed".into());
            }
            // Leading zeros and a leading + are in the lexical space, and i128
            // parsing takes both. A value too long for i128 is out of the range
            // of every type that has one.
            match text.parse::<i128>() {
                Ok(value) if value >= *min && value <= *max => None,
                _ if *min == i128::MIN && *max == i128::MAX => None,
                _ => Some(format!("between {min} and {max}")),
            }
        }
        Kind::Number { exponent, special } => {
            if *special && matches!(text, "INF" | "-INF" | "NaN") {
                return None;
            }
            let shape = if *exponent {
                pattern(
                    &EXPONENTIAL,
                    r"[+-]?([0-9]+(\.[0-9]*)?|\.[0-9]+)([Ee][+-]?[0-9]+)?",
                )
            } else {
                pattern(&DECIMAL, r"[+-]?([0-9]+(\.[0-9]*)?|\.[0-9]+)")
            };
            if shape.is_match(text) {
                None
            } else if *exponent {
                Some("a number, with an optional exponent".into())
            } else {
                Some("a number, without an exponent".into())
            }
        }
        Kind::DateTime => {
            let shape = pattern(
                &DATE_TIME,
                &format!(r"(?<date>{DATE})T(?<time>{TIME}){ZONE}?"),
            );
            match shape.captures(text) {
                Some(caps) if calendar(&caps["date"]) && clock(&caps["time"]) => None,
                _ => Some(
                    "a date and time, as CCYY-MM-DDThh:mm:ss with an optional \
                           fraction and time zone"
                        .into(),
                ),
            }
        }
        Kind::Date => {
            let shape = pattern(&DATE_ONLY, &format!(r"(?<date>{DATE}){ZONE}?"));
            match shape.captures(text) {
                Some(caps) if calendar(&caps["date"]) => None,
                _ => Some("a date, as CCYY-MM-DD".into()),
            }
        }
        Kind::Time => {
            let shape = pattern(&TIME_ONLY, &format!(r"(?<time>{TIME}){ZONE}?"));
            match shape.captures(text) {
                Some(caps) if clock(&caps["time"]) => None,
                _ => Some(
                    "a time of day, as hh:mm:ss with an optional fraction and \
                           time zone"
                        .into(),
                ),
            }
        }
        Kind::Duration => {
            // At least one component, and a T with at least one behind it.
            let shape = pattern(
                &DURATION,
                r"-?P((\d+Y)?(\d+M)?(\d+D)?(T(\d+H)?(\d+M)?(\d+(\.\d+)?S)?)?)",
            );
            let empty = text.ends_with('P') || text.ends_with('T');
            if shape.is_match(text) && !empty {
                None
            } else {
                Some("a duration, as PnYnMnDTnHnMnS with at least one part".into())
            }
        }
        Kind::HexBinary => {
            if text.len() % 2 == 0 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
                None
            } else {
                Some("an even number of hex digits".into())
            }
        }
        Kind::Text => None,
    }
}

/// Digits with an optional sign, which is XSD's lexical space for an integer.
fn integral(text: &str) -> bool {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// The parts of a date, once the shape is known to hold.
///
/// The shape cannot say that April has 30 days or which years are leap, and a
/// day-of-month check is most of the value of validating a date at all.
fn calendar(date: &str) -> bool {
    let mut parts = date.rsplitn(3, '-');
    let (Some(day), Some(month), Some(year)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let (Ok(day), Ok(month)) = (day.parse::<u32>(), month.parse::<u32>()) else {
        return false;
    };
    // A leading '-' for a year BCE has been left on the front by rsplitn.
    let Ok(year) = year.trim_start_matches('-').parse::<i64>() else {
        return false;
    };
    if year == 0 || !(1..=12).contains(&month) || day == 0 {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let last = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if leap => 29,
        _ => 28,
    };
    day <= last
}

/// The parts of a time, once the shape is known to hold.
///
/// 24:00:00 is the one hour past 23 that XSD allows, and only on the stroke.
fn clock(time: &str) -> bool {
    let mut parts = time.splitn(3, ':');
    let (Some(hour), Some(minute), Some(second)) = (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let (Ok(hour), Ok(minute)) = (hour.parse::<u32>(), minute.parse::<u32>()) else {
        return false;
    };
    let Ok(second) = second.parse::<f64>() else {
        return false;
    };
    if minute > 59 || second >= 60.0 {
        return false;
    }
    match hour {
        0..=23 => true,
        24 => minute == 0 && second == 0.0,
        _ => false,
    }
}

/// A four-or-more digit year, a month, and a day. Whether the day exists is
/// [`calendar`]'s question.
const DATE: &str = r"-?\d{4,}-\d{2}-\d{2}";
/// Hours, minutes, and seconds with an optional fraction. Whether they are in
/// range is [`clock`]'s question.
const TIME: &str = r"\d{2}:\d{2}:\d{2}(\.\d+)?";
/// `Z`, or an offset up to 14 hours either way.
const ZONE: &str = r"(Z|[+-]((0\d|1[0-3]):[0-5]\d|14:00))";

static EXPONENTIAL: OnceLock<Regex> = OnceLock::new();
static DECIMAL: OnceLock<Regex> = OnceLock::new();
static DATE_TIME: OnceLock<Regex> = OnceLock::new();
static DATE_ONLY: OnceLock<Regex> = OnceLock::new();
static TIME_ONLY: OnceLock<Regex> = OnceLock::new();
static DURATION: OnceLock<Regex> = OnceLock::new();

/// Compile a shape once, anchored, and keep it.
fn pattern(slot: &'static OnceLock<Regex>, shape: &str) -> &'static Regex {
    slot.get_or_init(|| {
        Regex::new(&format!(r"\A(?:{shape})\z")).expect("these shapes are written here")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accepts(primitive: &str, text: &str) -> bool {
        refuses(&kind(primitive), text).is_none()
    }

    #[test]
    fn a_boolean_is_one_of_four_words() {
        for text in ["true", "false", "1", "0"] {
            assert!(accepts("xs:boolean", text), "{text}");
        }
        for text in ["yes", "True", "", "2"] {
            assert!(!accepts("xs:boolean", text), "{text}");
        }
    }

    #[test]
    fn an_integer_is_checked_for_shape_and_then_for_range() {
        assert!(accepts("xs:int", "-2147483648"));
        assert!(
            accepts("xs:int", "+07"),
            "leading zeros and a sign are lexical"
        );
        assert!(!accepts("xs:int", "2147483648"));
        assert!(!accepts("xs:int", "5.0"), "a decimal is not an integer");
        assert!(!accepts("xs:int", "abc"));

        // The distinction is worth keeping: the shape is wrong in one case and
        // the range in the other, and an operator reading a log wants to know.
        assert_eq!(
            refuses(&kind("xs:int"), "99999999999999"),
            Some("between -2147483648 and 2147483647".into())
        );
        assert_eq!(
            refuses(&kind("xs:int"), "5.7"),
            Some("a whole number, optionally signed".into())
        );
    }

    #[test]
    fn the_unsigned_types_have_the_bounds_they_are_named_for() {
        assert!(accepts("xs:unsignedByte", "255"));
        assert!(!accepts("xs:unsignedByte", "256"));
        assert!(!accepts("xs:unsignedInt", "-1"));
        assert!(accepts("xs:unsignedLong", "18446744073709551615"));
        assert!(!accepts("xs:unsignedLong", "18446744073709551616"));
        assert!(!accepts("xs:positiveInteger", "0"));
        assert!(accepts("xs:nonNegativeInteger", "0"));
    }

    /// `xs:integer` has no bounds, so a number too large for any machine type is
    /// still one.
    #[test]
    fn an_unbounded_integer_takes_a_number_of_any_size() {
        let long = "1".repeat(60);
        assert!(accepts("xs:integer", &long));
        assert!(!accepts("xs:long", &long));
    }

    #[test]
    fn an_exponent_belongs_to_the_floating_types_only() {
        assert!(accepts("xs:double", "1.5E3"));
        assert!(accepts("xs:double", "1e+30"));
        assert!(accepts("xs:double", "-.5"));
        assert!(accepts("xs:double", "INF"));
        assert!(accepts("xs:float", "NaN"));
        assert!(!accepts("xs:double", "1,5"));

        assert!(accepts("xs:decimal", "1.5"));
        assert!(!accepts("xs:decimal", "1.5E3"));
        assert!(!accepts("xs:decimal", "INF"));
    }

    #[test]
    fn a_date_and_time_is_checked_against_the_calendar_and_the_clock() {
        assert!(accepts("xs:dateTime", "2026-01-22T00:00:00Z"));
        assert!(accepts("xs:dateTime", "2026-01-22T00:00:00.123456-05:00"));
        assert!(
            accepts("xs:dateTime", "2026-01-22T24:00:00"),
            "midnight end"
        );
        assert!(accepts("xs:date", "2024-02-29"), "2024 is a leap year");

        assert!(!accepts("xs:dateTime", "nope"));
        assert!(
            !accepts("xs:dateTime", "2026-01-22"),
            "a date is not a dateTime"
        );
        assert!(!accepts("xs:dateTime", "2026-02-30T00:00:00"), "February");
        assert!(!accepts("xs:date", "2023-02-29"), "2023 is not a leap year");
        assert!(!accepts("xs:date", "1900-02-29"), "nor was 1900");
        assert!(!accepts("xs:dateTime", "2026-01-22T24:00:01"));
        assert!(!accepts("xs:dateTime", "2026-01-22T00:60:00"));
        assert!(!accepts("xs:dateTime", "2026-01-22T00:00:00+15:00"), "zone");
    }

    #[test]
    fn a_time_stands_on_its_own() {
        assert!(accepts("xs:time", "13:20:00"));
        assert!(accepts("xs:time", "13:20:00.5Z"));
        assert!(!accepts("xs:time", "13:20"));
        assert!(!accepts("xs:time", "25:00:00"));
    }

    #[test]
    fn a_duration_needs_at_least_one_part() {
        assert!(accepts("xs:duration", "P1Y2M3DT4H5M6S"));
        assert!(accepts("xs:duration", "PT30M"));
        assert!(accepts("xs:duration", "-P1D"));
        assert!(accepts("xs:duration", "PT0.5S"));
        assert!(!accepts("xs:duration", "P"));
        assert!(!accepts("xs:duration", "P1YT"));
        assert!(!accepts("xs:duration", "1Y"));
    }

    #[test]
    fn hex_comes_in_pairs() {
        assert!(accepts("xs:hexBinary", "deadBEEF"));
        assert!(accepts("xs:hexBinary", ""));
        assert!(!accepts("xs:hexBinary", "abc"));
        assert!(!accepts("xs:hexBinary", "zz"));
    }

    /// Anything not named is text, and text is the facets' business alone.
    #[test]
    fn an_unrecognized_primitive_is_left_to_its_facets() {
        assert!(!is_checked("xs:string"));
        assert!(!is_checked("xs:base64Binary"));
        assert!(accepts("xs:string", "anything at all"));
        assert!(is_checked("xs:dateTime"));
    }
}
