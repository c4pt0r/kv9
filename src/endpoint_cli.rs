use std::{collections::HashMap, fmt::Display, process::ExitCode, str::FromStr};

use kv9_common::NodeId;
use kv9_server::api::{EndpointChange, EndpointRefusal, EndpointUpdateResult, NodeEndpoint};
use kv9_server::endpoints::{EndpointClient, EndpointRpcError};

fn required<T: FromStr>(values: &HashMap<String, String>, key: &str) -> Result<T, String>
where
    T::Err: Display,
{
    values
        .get(key)
        .ok_or_else(|| format!("{key} is required"))?
        .parse()
        .map_err(|error| format!("invalid {key}: {error}"))
}

pub(super) fn run(mut args: impl Iterator<Item = String>, change: bool) -> ExitCode {
    let mut values = HashMap::new();
    while let Some(flag) = args.next() {
        let allowed = matches!(flag.as_str(), "--addr" | "--node-id")
            || change
                && matches!(
                    flag.as_str(),
                    "--cluster-id"
                        | "--store-incarnation"
                        | "--expected-address"
                        | "--expected-generation"
                        | "--new-address"
                );
        if !allowed {
            return super::command_error(&format!("unknown endpoint flag {flag}"));
        }
        let Some(value) = args.next() else {
            return super::command_error(&format!("{flag} needs a value"));
        };
        if values.insert(flag.clone(), value).is_some() {
            return super::command_error(&format!("duplicate endpoint flag {flag}"));
        }
    }
    match execute(values, change) {
        Ok(code) => code,
        Err(error) => super::command_error(&error),
    }
}

fn execute(values: HashMap<String, String>, change: bool) -> Result<ExitCode, String> {
    let address: String = required(&values, "--addr")?;
    let node = NodeId(required(&values, "--node-id")?);
    if node.0 == 0 {
        return Err("--node-id must be non-zero".into());
    }
    let request = if change {
        Some(EndpointChange {
            cluster: required(&values, "--cluster-id")?,
            node,
            incarnation: required(&values, "--store-incarnation")?,
            expected_address: required(&values, "--expected-address")?,
            expected_generation: required(&values, "--expected-generation")?,
            new_address: required(&values, "--new-address")?,
        })
    } else {
        None
    };
    let Some(token) = super::client_token() else {
        return Ok(ExitCode::FAILURE);
    };
    let mut client = match EndpointClient::connect(&address, &token) {
        Ok(client) => client,
        Err(error) => return Ok(rpc_error(error)),
    };
    if let Some(request) = request {
        match client.change(request) {
            Ok(EndpointUpdateResult::Changed { endpoint, applied }) => {
                println!(
                    "endpoint_outcome=changed\nmutation_term={}\nmutation_index={}",
                    applied.term, applied.index
                );
                print_endpoint(endpoint);
            }
            Ok(EndpointUpdateResult::Confirmed {
                endpoint,
                confirmation,
            }) => {
                println!(
                    "endpoint_outcome=confirmed\nconfirmation_term={}\nconfirmation_index={}",
                    confirmation.term, confirmation.index
                );
                print_endpoint(endpoint);
            }
            Ok(EndpointUpdateResult::Refused(reason)) => {
                let reason = match reason {
                    EndpointRefusal::WrongCluster => "wrong_cluster",
                    EndpointRefusal::MissingNode => "missing_node",
                    EndpointRefusal::InactiveNode => "inactive_node",
                    EndpointRefusal::InvalidIncarnation => "invalid_incarnation",
                    EndpointRefusal::Conflict => "conflict",
                    EndpointRefusal::GenerationExhausted => "generation_exhausted",
                };
                println!("endpoint_outcome=refused\nendpoint_refusal={reason}");
                return Ok(ExitCode::FAILURE);
            }
            Err(error) => return Ok(rpc_error(error)),
        }
    } else {
        match client.get(node) {
            Ok(result) => {
                println!(
                    "cluster_id={}\nfound={}",
                    result.cluster,
                    result.endpoint.is_some()
                );
                if let Some(endpoint) = result.endpoint {
                    print_endpoint(endpoint);
                }
            }
            Err(error) => return Ok(rpc_error(error)),
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn print_endpoint(endpoint: NodeEndpoint) {
    println!("node_id={}\nstore_incarnation={}\naddress={}\ngeneration={}\nprevious_address={}\nactive={}",
        endpoint.node.0, endpoint.incarnation, endpoint.address, endpoint.generation,
        endpoint.previous_address.map(|address| address.to_string()).unwrap_or_default(), endpoint.active);
}

fn rpc_error(error: EndpointRpcError) -> ExitCode {
    match error {
        EndpointRpcError::NotLeader { leader } => super::print_not_leader(leader),
        EndpointRpcError::Local(detail) => super::command_error(&detail),
        EndpointRpcError::AdmissionRefused { reason } => {
            println!("endpoint_outcome=refused\nadmission_refused={reason}");
            ExitCode::FAILURE
        }
        EndpointRpcError::Unconfirmed(detail) => {
            println!("endpoint_outcome=unconfirmed");
            eprintln!("endpoint request unconfirmed: {detail}");
            ExitCode::FAILURE
        }
    }
}
