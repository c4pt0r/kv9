fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/kv9.proto");
    tonic_prost_build::compile_protos("proto/kv9.proto")?;
    if std::env::var_os("CARGO_FEATURE_RPC_EXPERIMENT").is_some() {
        println!("cargo:rerun-if-changed=proto/rpc_experiment.proto");
        tonic_prost_build::configure()
            .skip_debug(["."])
            .compile_protos(&["proto/rpc_experiment.proto"], &["proto"])?;
    }
    Ok(())
}
