fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The SDK speaks exactly the proto the server in this repository serves.
    let dir = "../../../crates/server/proto";
    let proto = format!("{dir}/kv9.proto");
    println!("cargo:rerun-if-changed={proto}");
    tonic_prost_build::configure()
        .build_server(false)
        .compile_protos(&[proto.as_str()], &[dir])?;
    Ok(())
}
