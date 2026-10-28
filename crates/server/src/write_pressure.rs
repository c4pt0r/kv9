//! Process configuration is validated once, before any runtime owner starts.
use kv9_common::{Error, Result};

pub(crate) const DEFAULT_MAX_RAFT_LOG_ENTRIES: u64 = 65_536;

pub(crate) fn from_env() -> Result<u64> {
    parse(std::env::var("KV9_MAX_RAFT_LOG_ENTRIES"))
}

fn parse(value: std::result::Result<String, std::env::VarError>) -> Result<u64> {
    match value {
        Err(std::env::VarError::NotPresent) => Ok(DEFAULT_MAX_RAFT_LOG_ENTRIES),
        Ok(value) if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
            value.parse().map_err(|_| invalid())
        }
        _ => Err(invalid()),
    }
}

fn invalid() -> Error {
    Error::Config(
        "KV9_MAX_RAFT_LOG_ENTRIES must be a u64 decimal (0 explicitly disables it)".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_default_and_explicit_opt_out() {
        assert_eq!(parse(Err(std::env::VarError::NotPresent)).unwrap(), 65_536);
        for value in [0, 1, 120, u64::MAX] {
            assert_eq!(parse(Ok(value.to_string())).unwrap(), value);
        }
    }

    #[test]
    fn invalid_configuration_never_silently_disables_admission() {
        for value in [
            "",
            "-1",
            "+1",
            " 1",
            "1 ",
            "off",
            "1.0",
            "18446744073709551616",
        ] {
            assert!(matches!(parse(Ok(value.into())), Err(Error::Config(_))));
        }
        assert!(parse(Err(std::env::VarError::NotUnicode("bad".into()))).is_err());
    }
}
