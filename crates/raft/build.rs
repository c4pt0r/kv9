fn main() {
    // Internal node-to-node service (task #19). protoc is required and pinned
    // in CI (EdHuang's ruling relaxed the protoc-free constraint for gRPC).
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_protos(&["../../proto/kv9_raft.proto"], &["../../proto"])
        .expect("compile kv9_raft.proto (is a working protoc installed?)");
    println!("cargo:rerun-if-changed=../../proto/kv9_raft.proto");
    // Same V11 service before the read-only membership method; isolate its
    // generated filename from production. Never register a compatibility path.
    let old = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("pre_membership");
    std::fs::create_dir_all(&old).unwrap();
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .out_dir(old)
        .compile_protos(
            &["src/grpc/data_membership/pre_membership_wire.proto"],
            &["src/grpc/data_membership"],
        )
        .expect("compile frozen pre-membership V11 fixture");
    println!("cargo:rerun-if-changed=src/grpc/data_membership/pre_membership_wire.proto");
    // Frozen V7 dispatch/client fixture, included only under #[cfg(test)]. The
    // production service exposes V11 exclusively; no compatibility registration.
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_protos(
            &["src/grpc/ttl_format/pre_capture_wire.proto"],
            &["src/grpc/ttl_format"],
        )
        .expect("compile frozen V7 wire fixture");
    println!("cargo:rerun-if-changed=src/grpc/ttl_format/pre_capture_wire.proto");
    // Frozen V8 reader-era predecessor; test-only include, never a service fallback.
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_protos(
            &["src/grpc/ttl_format/pre_readers_wire.proto"],
            &["src/grpc/ttl_format"],
        )
        .expect("compile frozen V8 reader wire fixture");
    println!("cargo:rerun-if-changed=src/grpc/ttl_format/pre_readers_wire.proto");
    // Frozen V9 DATA read-floor predecessor, test-only include.
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_protos(
            &["src/grpc/ttl_format/pre_data_reads_wire.proto"],
            &["src/grpc/ttl_format"],
        )
        .expect("compile frozen V9 DATA read-floor wire fixture");
    println!("cargo:rerun-if-changed=src/grpc/ttl_format/pre_data_reads_wire.proto");
    // Frozen V10 task-census predecessor, test-only include.
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_protos(
            &["src/grpc/ttl_format/pre_store_history_wire.proto"],
            &["src/grpc/ttl_format"],
        )
        .expect("compile frozen V10 task-history wire fixture");
    println!("cargo:rerun-if-changed=src/grpc/ttl_format/pre_store_history_wire.proto");
}
