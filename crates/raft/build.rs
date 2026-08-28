fn main() {
    // Internal node-to-node service (task #19). protoc is required and pinned
    // in CI (EdHuang's ruling relaxed the protoc-free constraint for gRPC).
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_protos(&["../../proto/kv9_raft.proto"], &["../../proto"])
        .expect("compile kv9_raft.proto (is a working protoc installed?)");
    println!("cargo:rerun-if-changed=../../proto/kv9_raft.proto");
}
