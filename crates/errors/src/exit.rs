use std::process::ExitCode;

/// Stable process exit-code policy for the `cortex` binary.
///
/// Every successful invocation exits [`Exit::Success`] (`0`); each failure class maps to a
/// fixed, non-zero code that never changes across runs for the same class, so machine
/// consumers and CI can branch on `$?`. The numeric values are part of the contract: do not
/// renumber existing variants.
///
/// `Usage` deliberately matches the conventional shell "command line usage" code (`2`) that
/// clap and most POSIX tools already emit for argument errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Exit {
    /// The command completed successfully.
    Success = 0,
    /// An unclassified runtime failure (a path that has no more specific class).
    Failure = 1,
    /// An argument/usage error: unknown subcommand, unknown flag, bad value, or a conflict.
    Usage = 2,
    /// A required input was absent (e.g. no prompt argument and empty stdin).
    MissingInput = 3,
    /// The `CORTEX_NO_NET` kill-switch denied a network-requiring operation.
    NetDisabled = 4,
    /// Configuration was invalid (parse error, failed substitution, or type mismatch).
    Config = 5,
}

impl Exit {
    /// The raw numeric code as it will be observed by a parent process via `$?`.
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }
}

impl From<Exit> for ExitCode {
    fn from(exit: Exit) -> Self {
        ExitCode::from(exit.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_and_distinct() {
        assert_eq!(Exit::Success.code(), 0);
        assert_eq!(Exit::Failure.code(), 1);
        assert_eq!(Exit::Usage.code(), 2);
        assert_eq!(Exit::MissingInput.code(), 3);
        assert_eq!(Exit::NetDisabled.code(), 4);
        assert_eq!(Exit::Config.code(), 5);
    }

    #[test]
    fn only_success_is_zero() {
        for exit in [
            Exit::Failure,
            Exit::Usage,
            Exit::MissingInput,
            Exit::NetDisabled,
            Exit::Config,
        ] {
            assert_ne!(exit.code(), 0, "{exit:?} must be a non-zero failure code");
        }
    }
}
