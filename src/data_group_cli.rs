use kv9_common::{NodeId, RootDigest};
use kv9_server::data_groups::{DataGroupClient, DataGroupRpcError};
use std::io::Read;
use std::{collections::HashMap, fs::File, process::ExitCode};

pub(super) fn run_attach_learner(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(flag.as_str(), "--addr" | "--root-digest" | "--operation-id") {
            return super::command_error(&format!("unknown attach flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate attach flag {flag}"));
        }
    }
    match execute_attach(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_attach(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.attach_learner(root, operation))
    {
        Ok(r) => {
            println!(
                "attach_outcome={}
destination_node={}
cut_term={}
cut_index={}
capability=learner_configuration_only",
                if r.changed { "attached" } else { "confirmed" },
                r.destination.0,
                r.cut.term,
                r.cut.index
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "attach_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("attach_outcome=unconfirmed");
            eprintln!("attach unconfirmed; retry reuses the same operation: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run_emit_evidence(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(
            flag.as_str(),
            "--addr" | "--root-digest" | "--region" | "--receipt-file"
        ) {
            return super::command_error(&format!("unknown evidence flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate evidence flag {flag}"));
        }
    }
    match execute_emit_evidence(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_emit_evidence(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let region: u64 = required("--region")?
        .parse()
        .map_err(|_| "--region must be a number")?;
    let receipt_path = required("--receipt-file")?.clone();
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.emit_install_evidence(root, kv9_common::RegionId(region)))
    {
        Ok(r) => {
            std::fs::write(&receipt_path, &r.receipt).map_err(|e| e.to_string())?;
            println!(
                "evidence_outcome=emitted
image_digest={}
cut_term={}
cut_index={}
receipt_file={receipt_path}
capability=description_only",
                r.image_digest, r.cut.term, r.cut.index
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "evidence_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("evidence_outcome=unconfirmed");
            eprintln!("emission is read-only and may retry: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run_record_evidence(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(flag.as_str(), "--addr" | "--root-digest" | "--receipt-file") {
            return super::command_error(&format!("unknown evidence flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate evidence flag {flag}"));
        }
    }
    match execute_record_evidence(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_record_evidence(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let mut receipt = Vec::new();
    File::open(required("--receipt-file")?)
        .and_then(|mut f| f.read_to_end(&mut receipt))
        .map_err(|e| format!("--receipt-file: {e}"))?;
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.record_install_evidence(root, &receipt))
    {
        Ok((task, changed)) => {
            println!(
                "evidence_outcome={}
evidence_task={task}
capability=committed_row_only",
                if changed { "recorded" } else { "confirmed" }
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "evidence_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("evidence_outcome=unconfirmed");
            eprintln!("evidence unconfirmed; the identical receipt may retry: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run_record_truncation(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(
            flag.as_str(),
            "--addr" | "--root-digest" | "--operation-id" | "--floor-term" | "--floor-index"
        ) {
            return super::command_error(&format!("unknown truncation flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate truncation flag {flag}"));
        }
    }
    match execute_record_truncation(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_record_truncation(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let floor = kv9_common::AppliedPosition {
        term: required("--floor-term")?
            .parse()
            .map_err(|_| "--floor-term must be a number")?,
        index: required("--floor-index")?
            .parse()
            .map_err(|_| "--floor-index must be a number")?,
    };
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.record_source_truncation(root, operation, floor))
    {
        Ok((task, changed)) => {
            println!(
                "truncation_outcome={}
truncation_task={task}
capability=committed_row_only",
                if changed { "recorded" } else { "confirmed" }
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "truncation_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("truncation_outcome=unconfirmed");
            eprintln!("truncation unconfirmed; the identical decision may retry: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run_truncate_log(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(flag.as_str(), "--addr" | "--root-digest" | "--operation-id") {
            return super::command_error(&format!("unknown truncation flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate truncation flag {flag}"));
        }
    }
    match execute_truncate_log(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_truncate_log(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.truncate_source_log(root, operation))
    {
        Ok(r) => {
            println!(
                "truncate_outcome=compacted
floor_term={}
floor_index={}
first_index={}
capability=local_log_prefix_only",
                r.floor.term, r.floor.index, r.first_index
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "truncate_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("truncate_outcome=unconfirmed");
            eprintln!("truncation is idempotent at the same floor and may retry: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run_promote_voter(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(flag.as_str(), "--addr" | "--root-digest" | "--operation-id") {
            return super::command_error(&format!("unknown promote flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate promote flag {flag}"));
        }
    }
    match execute_promote_voter(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_promote_voter(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.promote_migration_voter(root, operation))
    {
        Ok(r) => {
            println!(
                "promote_outcome={}
destination_node={}
voters={}
capability=voter_configuration_only",
                if r.changed { "promoted" } else { "confirmed" },
                r.destination.0,
                r.voters
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "promote_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("promote_outcome=unconfirmed");
            eprintln!("promotion unconfirmed; retry reuses the same operation: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run_record_removal(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(
            flag.as_str(),
            "--addr"
                | "--root-digest"
                | "--operation-id"
                | "--source-node"
                | "--source-incarnation"
        ) {
            return super::command_error(&format!("unknown removal flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate removal flag {flag}"));
        }
    }
    match execute_record_removal(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_record_removal(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let source_node: u64 = required("--source-node")?
        .parse()
        .map_err(|_| "--source-node must be a number")?;
    let incarnation: [u8; 16] =
        super::decode_hex("--source-incarnation", required("--source-incarnation")?)?
            .try_into()
            .map_err(|_| "--source-incarnation must contain 16 bytes")?;
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token).and_then(|mut c| {
        c.record_source_removal(
            root,
            operation,
            kv9_common::NodeId(source_node),
            incarnation,
        )
    }) {
        Ok((task, changed)) => {
            println!(
                "removal_outcome={}
removal_task={task}
capability=committed_row_only",
                if changed { "recorded" } else { "confirmed" }
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "removal_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("removal_outcome=unconfirmed");
            eprintln!("removal unconfirmed; the identical decision may retry: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run_remove_replica(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(flag.as_str(), "--addr" | "--root-digest" | "--operation-id") {
            return super::command_error(&format!("unknown removal flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate removal flag {flag}"));
        }
    }
    match execute_remove_replica(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_remove_replica(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.remove_source_replica(root, operation))
    {
        Ok(r) => {
            println!(
                "remove_outcome={}
removed_node={}
voters={}
capability=voter_configuration_only",
                if r.changed { "removed" } else { "confirmed" },
                r.removed.0,
                r.voters
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "remove_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("remove_outcome=unconfirmed");
            eprintln!("removal unconfirmed; retry reuses the same operation: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run_record_split(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(
            flag.as_str(),
            "--addr"
                | "--root-digest"
                | "--operation-id"
                | "--parent-region"
                | "--split-key-hex"
                | "--child-low-task"
                | "--child-high-task"
        ) {
            return super::command_error(&format!("unknown split flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate split flag {flag}"));
        }
    }
    match execute_record_split(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_record_split(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let parent: u64 = required("--parent-region")?
        .parse()
        .map_err(|_| "--parent-region must be a number")?;
    let split_key = super::decode_hex("--split-key-hex", required("--split-key-hex")?)?;
    let child_low: u64 = required("--child-low-task")?
        .parse()
        .map_err(|_| "--child-low-task must be a number")?;
    let child_high: u64 = required("--child-high-task")?
        .parse()
        .map_err(|_| "--child-high-task must be a number")?;
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token).and_then(|mut c| {
        c.record_split_intent(
            root,
            operation,
            kv9_common::RegionId(parent),
            &split_key,
            child_low,
            child_high,
        )
    }) {
        Ok((task, changed)) => {
            println!(
                "split_outcome={}
split_task={task}
capability=committed_row_only",
                if changed { "recorded" } else { "confirmed" }
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "split_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("split_outcome=unconfirmed");
            eprintln!("split unconfirmed; the identical intent may retry: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run_seal_split(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(flag.as_str(), "--addr" | "--root-digest" | "--operation-id") {
            return super::command_error(&format!("unknown seal flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate seal flag {flag}"));
        }
    }
    match execute_seal_split(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_seal_split(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.seal_split_parent(root, operation))
    {
        Ok(r) => {
            println!(
                "seal_outcome={}
sealed_version={}
cut_term={}
cut_index={}
capability=group_write_fence_only",
                if r.changed { "sealed" } else { "confirmed" },
                r.sealed_version,
                r.cut.term,
                r.cut.index
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "seal_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("seal_outcome=unconfirmed");
            eprintln!("seal unconfirmed; retry reuses the same operation: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run_populate_child(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(
            flag.as_str(),
            "--addr" | "--root-digest" | "--operation-id" | "--half"
        ) {
            return super::command_error(&format!("unknown populate flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate populate flag {flag}"));
        }
    }
    match execute_populate_child(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_populate_child(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let high = match required("--half")?.as_str() {
        "low" => false,
        "high" => true,
        _ => return Err("--half must be low or high".into()),
    };
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.populate_split_child(root, operation, high))
    {
        Ok(r) => {
            println!(
                "populate_outcome=copied
rows_copied={}
parent_half_digest={}
child_digest={}
capability=committed_child_rows_only",
                r.rows_copied, r.parent_half_digest, r.child_digest
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "populate_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("populate_outcome=unconfirmed");
            eprintln!("population is idempotent and may retry: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run_publish_split(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(flag.as_str(), "--addr" | "--root-digest" | "--operation-id") {
            return super::command_error(&format!("unknown publish flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate publish flag {flag}"));
        }
    }
    match execute_publish_split(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_publish_split(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.publish_split(root, operation))
    {
        Ok(r) => {
            println!(
                "publish_outcome={}
parent_region={}
sealed_version={}
low_region={}
high_region={}
capability=atomic_directory_transaction",
                if r.changed { "published" } else { "confirmed" },
                r.publication.parent_region.0,
                r.publication.sealed_version,
                r.publication.low_region.0,
                r.publication.high_region.0
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "publish_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("publish_outcome=unconfirmed");
            eprintln!("publication is idempotent and may retry: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run_plan_image(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(
            flag.as_str(),
            "--addr" | "--root-digest" | "--operation-id" | "--manifest-file"
        ) {
            return super::command_error(&format!("unknown plan flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate plan flag {flag}"));
        }
    }
    match execute_plan_image(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_plan_image(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let manifest_path = required("--manifest-file")?.clone();
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.plan_image(root, operation))
    {
        Ok(r) => {
            std::fs::write(&manifest_path, &r.manifest).map_err(|e| e.to_string())?;
            println!(
                "plan_outcome=planned
cut_term={}
cut_index={}
manifest_file={manifest_path}
capability=description_only",
                r.cut.term, r.cut.index
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "plan_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("plan_outcome=unconfirmed");
            eprintln!("plan unconfirmed; planning is read-only and may retry: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run_capture_image(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(
            flag.as_str(),
            "--addr" | "--root-digest" | "--operation-id" | "--record-file"
        ) {
            return super::command_error(&format!("unknown capture flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate capture flag {flag}"));
        }
    }
    match execute_capture(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_capture(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let record_path = required("--record-file")?.clone();
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.capture_image(root, operation))
    {
        Ok(r) => {
            std::fs::write(&record_path, &r.record).map_err(|e| e.to_string())?;
            println!(
                "capture_outcome=captured
image_digest={}
cut_term={}
cut_index={}
configuration_applied_index={}
objects={}
object_bytes={}
source_owner={}
destination_owner={}
record_file={record_path}
capability=description_only",
                r.image_digest,
                r.cut.term,
                r.cut.index,
                r.configuration_applied_at.map_or(0, |p| p.index),
                r.objects,
                r.object_bytes,
                super::encode_hex(r.source_owner.as_bytes()),
                super::encode_hex(r.destination_owner.as_bytes())
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "capture_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("capture_outcome=unconfirmed");
            eprintln!("capture unconfirmed; the operation may retry safely: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

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

pub(super) fn run_record_abort(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(flag.as_str(), "--addr" | "--root-digest" | "--operation-id") {
            return super::command_error(&format!("unknown abort flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate abort flag {flag}"));
        }
    }
    match execute_record_abort(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_record_abort(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.record_migration_abort(root, operation))
    {
        Ok((task, changed)) => {
            println!(
                "abort_outcome={}
abort_task={task}
capability=settlement_row_only",
                if changed { "committed" } else { "confirmed" }
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "abort_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("abort_outcome=unconfirmed");
            eprintln!("the abort is idempotent and may retry: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run_detach_learner(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(flag.as_str(), "--addr" | "--root-digest" | "--operation-id") {
            return super::command_error(&format!("unknown detach flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate detach flag {flag}"));
        }
    }
    match execute_detach_learner(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_detach_learner(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let operation: [u8; 16] = super::decode_hex("--operation-id", required("--operation-id")?)?
        .try_into()
        .map_err(|_| "--operation-id must contain 16 bytes")?;
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.detach_aborted_learner(root, operation))
    {
        Ok((node, changed, voters)) => {
            println!(
                "detach_outcome={}
detached_node={node}
voters={}",
                if changed { "detached" } else { "confirmed" },
                voters
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "detach_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("detach_outcome=unconfirmed");
            eprintln!("the detach is idempotent and may retry: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub(super) fn run_record_compaction(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        if !matches!(
            flag.as_str(),
            "--addr" | "--root-digest" | "--region" | "--floor-term" | "--floor-index"
        ) {
            return super::command_error(&format!("unknown compaction flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate compaction flag {flag}"));
        }
    }
    match execute_record_compaction(values) {
        Ok(code) => code,
        Err(e) => super::command_error(&e),
    }
}

fn execute_record_compaction(values: HashMap<String, String>) -> Result<ExitCode, String> {
    let required = |key: &str| values.get(key).ok_or_else(|| format!("{key} is required"));
    let root = RootDigest::from_bytes(
        super::decode_hex("--root-digest", required("--root-digest")?)?
            .try_into()
            .map_err(|_| "--root-digest must contain 32 bytes")?,
    );
    let region = kv9_common::RegionId(
        required("--region")?
            .parse()
            .map_err(|_| "--region must be a number")?,
    );
    let floor = kv9_common::AppliedPosition {
        term: required("--floor-term")?
            .parse()
            .map_err(|_| "--floor-term must be a number")?,
        index: required("--floor-index")?
            .parse()
            .map_err(|_| "--floor-index must be a number")?,
    };
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    match DataGroupClient::connect(required("--addr")?, &token)
        .and_then(|mut c| c.record_group_compaction(root, region, floor))
    {
        Ok((task, changed)) => {
            println!(
                "compaction_outcome={}
compaction_task={task}
capability=committed_floor_only",
                if changed { "recorded" } else { "confirmed" }
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(DataGroupRpcError::Local(e)) => Err(e),
        Err(DataGroupRpcError::NotLeader { leader }) => Ok(super::print_not_leader(leader)),
        Err(DataGroupRpcError::AdmissionRefused { reason }) => {
            println!(
                "compaction_outcome=refused
admission_refused={reason}"
            );
            Ok(ExitCode::FAILURE)
        }
        Err(DataGroupRpcError::Unconfirmed(e)) => {
            println!("compaction_outcome=unconfirmed");
            eprintln!("the identical floor may retry: {e}");
            Ok(ExitCode::FAILURE)
        }
    }
}
