use kv9_server::workload::{run, RunOptions, WorkloadConfig};
use std::path::PathBuf;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    if let Err(error) = execute().await {
        eprintln!("FAIL: {error}");
        std::process::exit(1);
    }
}

async fn execute() -> Result<(), String> {
    let mut config = None;
    let mut output = None;
    let mut build_manifest = None;
    let mut stop_file = None;
    let mut phase_file = None;
    let mut arguments = std::env::args_os().skip(1);
    while let Some(key) = arguments.next() {
        let target = match key.to_str() {
            Some("--config") => &mut config,
            Some("--output") => &mut output,
            Some("--build-manifest") => &mut build_manifest,
            Some("--stop-file") => &mut stop_file,
            Some("--phase-file") => &mut phase_file,
            _ => return Err("usage: kv9-workload --config FILE --output NEW_DIRECTORY --build-manifest FILE [--stop-file FILE] [--phase-file FILE]".into()),
        };
        if target.is_some() {
            return Err("duplicate workload argument".into());
        }
        *target = Some(PathBuf::from(
            arguments.next().ok_or("missing workload argument value")?,
        ));
    }
    let configuration = WorkloadConfig::read(&config.ok_or("--config is required")?)?;
    let options = RunOptions {
        output: output.ok_or("--output is required")?,
        build_manifest: build_manifest.ok_or("--build-manifest is required")?,
        stop_file,
        phase_file,
    };
    let token = std::env::var("KV9_CLIENT_TOKEN").map_err(|_| "KV9_CLIENT_TOKEN is required")?;
    let report = run(configuration, &token, options).await?;
    if !report.complete {
        return Err(report.failure.unwrap_or("incomplete workload").into());
    }
    println!(
        "PASS: workload drained {} operations; independent artifact verification is required",
        report.history.terminal
    );
    Ok(())
}
