use kv9_common::{retention::OwnerId, RootDigest};
use kv9_meta::retention::{encode_owner_observation, MAX_LEDGER_REQUEST_BYTES};
use kv9_server::retention::{RetentionClient, RetentionRpcError};
use std::{collections::HashMap, fs::File, io::Read, process::ExitCode};

pub(super) fn run(mut args: impl Iterator<Item = String>, apply: bool) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(flag.as_str(), "--addr" | "--root-digest")
            && flag
                != if apply {
                    "--request-file"
                } else {
                    "--owner-id"
                }
        {
            return super::command_error(&format!("unknown retention flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate retention flag {flag}"));
        }
    }
    match execute(values, apply) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}
fn execute(values: HashMap<String, String>, apply: bool) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    if root.as_bytes() == &[0; 32] {
        return Err("--root-digest must be nonzero".into());
    }
    let (command, owner) = if apply {
        let mut command = Vec::new();
        File::open(required("--request-file")?)
            .map_err(|e| e.to_string())?
            .take((MAX_LEDGER_REQUEST_BYTES + 1) as u64)
            .read_to_end(&mut command)
            .map_err(|e| e.to_string())?;
        kv9_meta::retention::decode_request(&command, root).map_err(|e| e.to_string())?;
        (Some(command), None)
    } else {
        let id = OwnerId::new(
            super::decode_hex("--owner-id", required("--owner-id")?)?
                .try_into()
                .map_err(|_| "--owner-id must contain 16 bytes")?,
        )
        .map_err(|e| e.to_string())?;
        (None, Some(id))
    };
    let address = required("--addr")?;
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    let mut client = match RetentionClient::connect(address, &token) {
        Ok(c) => c,
        Err(e) => return Ok(rpc_error(e)),
    };
    if let Some(command) = command {
        match client.apply_encoded(root, command) {
            Ok(r) => {
                let kind = if r.changed {
                    "mutation"
                } else {
                    "confirmation"
                };
                println!(
                    "retention_outcome={}\nrevision={}\n{kind}_term={}\n{kind}_index={}",
                    if r.changed { "changed" } else { "confirmed" },
                    r.revision,
                    r.applied.term,
                    r.applied.index
                );
            }
            Err(e) => return Ok(rpc_error(e)),
        }
    } else {
        match client.get(root, owner.unwrap()) {
            Ok(Some(owner)) => {
                println!(
                    "found=true\nowner_hex={}",
                    super::encode_hex(
                        &encode_owner_observation(&owner).map_err(|e| e.to_string())?
                    )
                );
            }
            Ok(None) => println!("found=false"),
            Err(e) => return Ok(rpc_error(e)),
        }
    }
    Ok(ExitCode::SUCCESS)
}
fn rpc_error(error: RetentionRpcError) -> ExitCode {
    match error {
        RetentionRpcError::Local(e) => super::command_error(&e),
        RetentionRpcError::NotLeader { leader } => super::print_not_leader(leader),
        RetentionRpcError::AdmissionRefused { reason } => {
            println!("retention_outcome=refused\nadmission_refused={reason}");
            ExitCode::FAILURE
        }
        RetentionRpcError::Unconfirmed(e) => {
            println!("retention_outcome=unconfirmed");
            eprintln!("retention request unconfirmed: {e}");
            ExitCode::FAILURE
        }
    }
}
