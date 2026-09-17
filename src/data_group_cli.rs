use kv9_common::{NodeId, RootDigest};
use kv9_server::data_groups::{DataGroupClient, DataGroupRpcError};
use std::io::Read;
use std::{collections::HashMap, fs::File, process::ExitCode};

pub(super) fn run_bind_image(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(
            flag.as_str(),
            "--addr" | "--root-digest" | "--operation-id" | "--manifest-file"
        ) {
            return super::command_error(&format!("unknown image binding flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate image binding flag {flag}"));
        }
    }
    match execute_bind_image(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_bind_image(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let mut manifest = Vec::new();
    File::open(required("--manifest-file")?)
        .map_err(|e| e.to_string())?
        .take(4 * 1024 * 1024)
        .read_to_end(&mut manifest)
        .map_err(|e| e.to_string())?;
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.bind_image(root, operation, &manifest))
    {
        Ok(r) => {
            println!(
                "binding_outcome=bound\nsource_owner={}\ndestination_owner={}\ncapability=tracking_only_pins",
                super::encode_hex(r.source_owner.as_bytes()),
                super::encode_hex(r.destination_owner.as_bytes())
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!("binding_outcome=refused\nadmission_refused={reason}");
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("binding_outcome=unconfirmed");
            eprintln!("image binding unconfirmed; the same operation binds the same image: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run_migrate(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(
            flag.as_str(),
            "--addr"
                | "--root-digest"
                | "--operation-id"
                | "--creation-task"
                | "--destination-node"
        ) {
            return super::command_error(&format!("unknown migration flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate migration flag {flag}"));
        }
    }
    match execute_migrate(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_migrate(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let task = required("--creation-task")?
        .parse::<u64>()
        .map_err(|_| "invalid --creation-task")?;
    let destination = required("--destination-node")?
        .parse::<u64>()
        .map(NodeId)
        .map_err(|_| "invalid --destination-node")?;
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.migrate(root, operation, task, destination))
    {
        Ok(r) => {
            let kind = if r.changed {
                "mutation"
            } else {
                "confirmation"
            };
            println!("migration_outcome={}\ntask_id={}\nregion_id={}\ndestination_node={}\ndestination_incarnation={}\n{kind}_term={}\n{kind}_index={}\ncapability=image_owner_binding_only",
                if r.changed { "requested" } else { "confirmed" }, r.intent.task(), r.intent.region().0,
                r.intent.destination().node.0, r.intent.destination().incarnation, r.applied.term, r.applied.index);
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!("migration_outcome=refused\nadmission_refused={reason}");
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("migration_outcome=unconfirmed");
            eprintln!(
                "migration request unconfirmed; retain and reuse the exact operation ID: {e}"
            );
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run(mut args: impl Iterator<Item = String>, keyspace: bool) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !(matches!(flag.as_str(), "--addr" | "--root-digest")
            || (keyspace && matches!(flag.as_str(), "--creation-task" | "--name" | "--tenant-id"))
            || (!keyspace && matches!(flag.as_str(), "--operation-id" | "--voters")))
        {
            return super::command_error(&format!("unknown data group flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate data group flag {flag}"));
        }
    }
    match if keyspace {
        execute_keyspace(values)
    } else {
        execute(values)
    } {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_keyspace(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let task = required("--creation-task")?
        .parse::<u64>()
        .map_err(|_| "invalid --creation-task")?;
    let tenant = values
        .get("--tenant-id")
        .map(|v| v.parse::<u64>().map(kv9_common::TenantId))
        .transpose()
        .map_err(|_| "invalid --tenant-id")?
        .unwrap_or(kv9_common::TenantId::DEFAULT);
    let name = required("--name")?;
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.create_keyspace(root, task, name, tenant))
    {
        Ok(r) => {
            let kind = if r.changed {
                "mutation"
            } else {
                "confirmation"
            };
            println!("keyspace_outcome={}\nkeyspace_id={}\nregion_id={}\n{kind}_term={}\n{kind}_index={}\nreadiness=not_asserted",if r.changed {"requested"}else{"confirmed"},r.range.keyspace.0,r.range.region.0,r.applied.term,r.applied.index);
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(e) => {
            println!("keyspace_outcome=unconfirmed");
            eprintln!("{e}");
            Ok(ExitCode::FAILURE)
        }
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
