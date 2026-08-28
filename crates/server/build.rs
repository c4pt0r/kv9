fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/kv9.proto");
    tonic_prost_build::compile_protos("proto/kv9.proto")?;
    Ok(())
}
