use kv9_common::{NodeId, RootDigest};
use kv9_server::data_groups::{DataGroupClient, DataGroupRpcError};
use std::{collections::HashMap, process::ExitCode};

pub(super) fn run(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(
            flag.as_str(),
            "--addr" | "--root-digest" | "--operation-id" | "--voters"
        ) {
            return super::command_error(&format!("unknown data group flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate data group flag {flag}"));
        }
    }
    match execute(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let voters: Vec<NodeId> = required("--voters")?
        .split(',')
        .map(|value| {
            value
                .parse::<u64>()
                .map(NodeId)
                .map_err(|_| "--voters must contain comma-separated node IDs".to_string())
        })
        .collect::<Result<_, _>>()?;
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    let result = DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut client| client.create(root, operation, &voters));
    match result {
        Ok(result) => {
            let kind = if result.changed {
                "mutation"
            } else {
                "confirmation"
            };
            println!("group_outcome={}\ntask_id={}\nregion_id={}\nintent_digest={}\n{kind}_term={}\n{kind}_index={}\nreadiness=not_asserted",
                if result.changed { "requested" } else { "confirmed" }, result.intent.task(), result.intent.region().0,
                result.intent.digest(), result.applied.term, result.applied.index);
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!("group_outcome=refused\nadmission_refused={reason}");
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("group_outcome=unconfirmed");
            eprintln!(
                "data group request unconfirmed; retain and reuse the exact operation ID: {e}"
            );
            Ok(ExitCode::FAILURE)
        }
    }
}
