use std::{collections::HashMap, io::Write};

#[cfg(feature = "nyx")]
use fuzzamoto_nyx_sys::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Assertion {
    Condition(bool),
    LessThan(u64, u64),
    LessThanOrEqual(u64, u64),
    GreaterThan(u64, u64),
    GreaterThanOrEqual(u64, u64),
}

impl Assertion {
    #[must_use]
    pub fn distance(&self, inverted: bool) -> u64 {
        match self {
            Assertion::Condition(value) => u64::from(*value == inverted),
            Assertion::LessThan(a, b) => {
                if inverted {
                    // Inverted: distance to a < b being false (i.e., a >= b)
                    if a >= b { 0 } else { b - a }
                } else {
                    // Normal: distance to a < b being true
                    if a < b { 0 } else { a - b + 1 }
                }
            }
            Assertion::LessThanOrEqual(a, b) => {
                if inverted {
                    // Inverted: distance to a <= b being false (i.e., a > b)
                    if a > b { 0 } else { b - a + 1 }
                } else {
                    // Normal: distance to a <= b being true
                    if a <= b { 0 } else { a - b }
                }
            }
            Assertion::GreaterThan(a, b) => {
                if inverted {
                    // Inverted: distance to a > b being false (i.e., a <= b)
                    if a <= b { 0 } else { a - b }
                } else {
                    // Normal: distance to a > b being true
                    if a > b { 0 } else { b - a + 1 }
                }
            }
            Assertion::GreaterThanOrEqual(a, b) => {
                if inverted {
                    // Inverted: distance to a >= b being false (i.e., a < b)
                    if a < b { 0 } else { a - b + 1 }
                } else {
                    // Normal: distance to a >= b being true
                    if a >= b { 0 } else { b - a }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AssertionScope {
    Sometimes(Assertion, String),
    Always(Assertion, String),
}

impl AssertionScope {
    #[must_use]
    pub fn evaluate(&self) -> bool {
        self.distance() == 0
    }

    #[must_use]
    pub fn distance(&self) -> u64 {
        match self {
            AssertionScope::Sometimes(assertion, _) => {
                // "Sometimes" fires when the condition IS true
                assertion.distance(false)
            }
            AssertionScope::Always(assertion, _) => {
                // "Always" fires when the condition IS NOT true (violation)
                assertion.distance(true)
            }
        }
    }

    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Always(_, msg) | Self::Sometimes(_, msg) => msg.clone(),
        }
    }
}

pub fn write_assertions<W: Write, S: ::std::hash::BuildHasher>(
    writer: &mut W,
    assertions: &HashMap<String, AssertionScope, S>,
) -> std::io::Result<()> {
    // ANSI color codes
    const GREEN: &str = "\x1b[32m";
    const RED: &str = "\x1b[31m";
    const RESET: &str = "\x1b[0m";
    const BOLD: &str = "\x1b[1m";

    for assertion in assertions.values() {
        let mut fires = assertion.evaluate();

        let (assertion_type, assertion_detail, message) = match assertion {
            AssertionScope::Sometimes(inner, msg) => {
                let detail = format_assertion_detail(inner);
                ("Sometimes", detail, msg)
            }
            AssertionScope::Always(inner, msg) => {
                fires = !fires;
                let detail = format_assertion_detail(inner);
                ("Always", detail, msg)
            }
        };

        if fires {
            writeln!(
                writer,
                "{BOLD}{GREEN}✓{RESET} {assertion_type} {assertion_detail}: {message}",
            )?;
        } else {
            writeln!(
                writer,
                "{BOLD}{RED}✗{RESET} {assertion_type} {assertion_detail}: {message}",
            )?;
        }
    }

    Ok(())
}

/// Helper function to format assertion details for display
fn format_assertion_detail(assertion: &Assertion) -> String {
    match assertion {
        Assertion::Condition(value) => {
            format!("cond({value})")
        }
        Assertion::LessThan(a, b) => {
            format!("lt({a}, {b})")
        }
        Assertion::LessThanOrEqual(a, b) => {
            format!("lte({a}, {b})")
        }
        Assertion::GreaterThan(a, b) => {
            format!("gt({a}, {b})")
        }
        Assertion::GreaterThanOrEqual(a, b) => {
            format!("gte({a}, {b})")
        }
    }
}

#[cfg(feature = "nyx")]
pub fn log_assertion(assertion: &AssertionScope) {
    use base64::prelude::{BASE64_STANDARD, Engine};
    use std::ffi::CString;

    if let Ok(json) = serde_json::to_string(assertion) {
        let encoded = BASE64_STANDARD.encode(json.as_bytes());
        let message = crate::StdoutMessage::Assertion(encoded);
        if let Ok(envelope) = serde_json::to_string(&message)
            && let Ok(c_envelope) = CString::new(envelope.as_bytes())
        {
            unsafe {
                nyx_println(c_envelope.as_ptr(), envelope.len());
            }
        }
    }
}

#[cfg(not(feature = "nyx"))]
pub fn log_assertion(assertion: &AssertionScope) {
    if let Ok(json) = serde_json::to_string(assertion) {
        log::debug!("{json}");
    }
}

#[macro_export]
macro_rules! assert_sometimes {
    (cond: $cond:expr, $msg:expr) => {
        $crate::assertions::log_assertion(&$crate::assertions::AssertionScope::Sometimes(
            $crate::assertions::Assertion::Condition($cond),
            format!("{} ({}, {}, {})", $msg, file!(), line!(), column!()),
        ));
    };
    (lt: $left:expr, $right:expr, $msg:expr) => {
        $crate::assertions::log_assertion(&$crate::assertions::AssertionScope::Sometimes(
            $crate::assertions::Assertion::LessThan($left, $right),
            format!("{} ({}, {}, {})", $msg, file!(), line!(), column!()),
        ));
    };
    (lte: $left:expr, $right:expr, $msg:expr) => {
        $crate::assertions::log_assertion(&$crate::assertions::AssertionScope::Sometimes(
            $crate::assertions::Assertion::LessThanOrEqual($left, $right),
            format!("{} ({}, {}, {})", $msg, file!(), line!(), column!()),
        ));
    };
    (gt: $left:expr, $right:expr, $msg:expr) => {
        $crate::assertions::log_assertion(&$crate::assertions::AssertionScope::Sometimes(
            $crate::assertions::Assertion::GreaterThan($left, $right),
            format!("{} ({}, {}, {})", $msg, file!(), line!(), column!()),
        ));
    };
    (gte: $left:expr, $right:expr, $msg:expr) => {
        $crate::assertions::log_assertion(&$crate::assertions::AssertionScope::Sometimes(
            $crate::assertions::Assertion::GreaterThanOrEqual($left, $right),
            format!("{} ({}, {}, {})", $msg, file!(), line!(), column!()),
        ));
    };
}

#[macro_export]
macro_rules! assert_always {
    (cond: $cond:expr, $msg:expr) => {
        $crate::assertions::log_assertion(&$crate::assertions::AssertionScope::Always(
            $crate::assertions::Assertion::Condition($cond),
            format!("{} ({}, {}, {})", $msg, file!(), line!(), column!()),
        ));
    };
    (lt: $left:expr, $right:expr, $msg:expr) => {
        $crate::assertions::log_assertion(&$crate::assertions::AssertionScope::Always(
            $crate::assertions::Assertion::LessThan($left, $right),
            format!("{} ({}, {}, {})", $msg, file!(), line!(), column!()),
        ));
    };
    (lte: $left:expr, $right:expr, $msg:expr) => {
        $crate::assertions::log_assertion(&$crate::assertions::AssertionScope::Always(
            $crate::assertions::Assertion::LessThanOrEqual($left, $right),
            format!("{} ({}, {}, {})", $msg, file!(), line!(), column!()),
        ));
    };
    (gt: $left:expr, $right:expr, $msg:expr) => {
        $crate::assertions::log_assertion(&$crate::assertions::AssertionScope::Always(
            $crate::assertions::Assertion::GreaterThan($left, $right),
            format!("{} ({}, {}, {})", $msg, file!(), line!(), column!()),
        ));
    };
    (gte: $left:expr, $right:expr, $msg:expr) => {
        $crate::assertions::log_assertion(&$crate::assertions::AssertionScope::Always(
            $crate::assertions::Assertion::GreaterThanOrEqual($left, $right),
            format!("{} ({}, {}, {})", $msg, file!(), line!(), column!()),
        ));
    };
}
