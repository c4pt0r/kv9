cmake_minimum_required(VERSION 3.16)
project(kv9_client CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
option(KV9_BUILD_TESTS "Build the SDK unit tests" ON)
option(KV9_BUILD_EXAMPLES "Build the smoke example" ON)

# The SDK speaks exactly the proto the server in this repository serves.
set(KV9_PROTO_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../crates/server/proto" CACHE PATH "Directory holding kv9.proto")
# protoc matches --proto_path as an exact string prefix, so hand it a normalized path.
get_filename_component(KV9_PROTO_DIR "${KV9_PROTO_DIR}" REALPATH)
set(KV9_PROTO "${KV9_PROTO_DIR}/kv9.proto")

find_package(Protobuf REQUIRED)
find_package(gRPC CONFIG QUIET)
if(gRPC_FOUND)
  set(KV9_GRPC_LIBS gRPC::grpc++)
  if(TARGET gRPC::grpc_cpp_plugin)
    get_target_property(KV9_GRPC_PLUGIN gRPC::grpc_cpp_plugin IMPORTED_LOCATION)
  endif()
else()
  find_package(PkgConfig REQUIRED)
  pkg_check_modules(GRPCPP REQUIRED IMPORTED_TARGET grpc++)
  set(KV9_GRPC_LIBS PkgConfig::GRPCPP)
endif()
if(NOT KV9_GRPC_PLUGIN)
  find_program(KV9_GRPC_PLUGIN grpc_cpp_plugin REQUIRED)
endif()

set(KV9_GEN "${CMAKE_CURRENT_BINARY_DIR}/gen")
file(MAKE_DIRECTORY "${KV9_GEN}")
add_custom_command(
  OUTPUT "${KV9_GEN}/kv9.pb.cc" "${KV9_GEN}/kv9.pb.h" "${KV9_GEN}/kv9.grpc.pb.cc" "${KV9_GEN}/kv9.grpc.pb.h"
  COMMAND protobuf::protoc
          "--proto_path=${KV9_PROTO_DIR}"
          "--cpp_out=${KV9_GEN}"
          "--grpc_out=${KV9_GEN}"
          "--plugin=protoc-gen-grpc=${KV9_GRPC_PLUGIN}"
          "${KV9_PROTO}"
  DEPENDS "${KV9_PROTO}"
  COMMENT "Generating kv9.v1 protobuf and gRPC sources"
  VERBATIM)

add_library(kv9_client
  src/client.cc
  "${KV9_GEN}/kv9.pb.cc"
  "${KV9_GEN}/kv9.grpc.pb.cc")
target_include_directories(kv9_client
  PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}/include"
  PRIVATE "${KV9_GEN}")
target_link_libraries(kv9_client PUBLIC ${KV9_GRPC_LIBS} protobuf::libprotobuf)

if(KV9_BUILD_EXAMPLES)
  add_executable(kv9_smoke examples/smoke.cc)
  target_link_libraries(kv9_smoke PRIVATE kv9_client)
endif()

if(KV9_BUILD_TESTS)
  enable_testing()
  add_executable(kv9_classify_test tests/classify_test.cc)
  target_link_libraries(kv9_classify_test PRIVATE kv9_client)
  add_test(NAME classify COMMAND kv9_classify_test)
endif()
