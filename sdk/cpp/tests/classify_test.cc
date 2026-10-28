// Marker classification without a server. Plain asserts: no test framework dependency.
#include <grpcpp/grpcpp.h>

#include <cassert>
#include <cstdio>
#include <map>
#include <string>
#include <vector>

#include "kv9/client.h"

namespace {

using Trailing = std::multimap<grpc::string_ref, grpc::string_ref>;

// grpc::string_ref does not own; keep the strings alive for the test's duration.
struct Fixture {
  std::vector<std::pair<std::string, std::string>> storage;
  Trailing trailing;
  explicit Fixture(std::vector<std::pair<std::string, std::string>> markers) : storage(std::move(markers)) {
    for (const auto& [k, v] : storage) trailing.emplace(grpc::string_ref(k), grpc::string_ref(v));
  }
};

kv9::Error Run(grpc::StatusCode code, std::vector<std::pair<std::string, std::string>> markers, bool read,
               const std::string& message = "message") {
  Fixture fixture(std::move(markers));
  return kv9::Classify(grpc::Status(code, message), fixture.trailing, read);
}

}  // namespace

int main() {
  using grpc::StatusCode;
  using kv9::ErrorKind;

  auto leader = Run(StatusCode::FAILED_PRECONDITION, {{"kv9-not-leader", "true"}, {"kv9-leader-node-id", "3"}}, false);
  assert(leader.kind == ErrorKind::kNotLeader && leader.leader_node_id && *leader.leader_node_id == 3);
  auto bare = Run(StatusCode::FAILED_PRECONDITION, {{"kv9-not-leader", "true"}}, true);
  assert(bare.kind == ErrorKind::kNotLeader && !bare.leader_node_id);

  auto read = Run(StatusCode::UNAVAILABLE, {{"kv9-read-unconfirmed", "apply"}}, true);
  assert(read.kind == ErrorKind::kReadUnconfirmed && read.phase == "apply");

  auto refused = Run(StatusCode::RESOURCE_EXHAUSTED, {{"kv9-admission-refused", "request_too_large"}}, false);
  assert(refused.kind == ErrorKind::kAdmissionRefused && refused.reason == "request_too_large");

  auto partial = Run(StatusCode::UNAVAILABLE,
                     {{"kv9-partial-write", "true"},
                      {"kv9-committed-chunks", "2"},
                      {"kv9-last-applied-term", "4"},
                      {"kv9-last-applied-index", "77"}},
                     false, "leader changed");
  assert(partial.kind == ErrorKind::kPartialWrite && partial.committed_chunks == 2 &&
         partial.last_applied.term == 4 && partial.last_applied.index == 77 && partial.message == "leader changed");

  struct Bad {
    StatusCode code;
    std::vector<std::pair<std::string, std::string>> markers;
    bool read;
  };
  const std::vector<Bad> bad = {
      {StatusCode::FAILED_PRECONDITION, {{"kv9-not-leader", "yes"}}, false},
      {StatusCode::UNAVAILABLE, {{"kv9-not-leader", "true"}}, false},
      {StatusCode::FAILED_PRECONDITION, {{"kv9-not-leader", "true"}, {"kv9-leader-node-id", "0"}}, false},
      {StatusCode::FAILED_PRECONDITION, {{"kv9-not-leader", "true"}, {"kv9-leader-node-id", "03"}}, false},
      {StatusCode::UNAVAILABLE, {{"kv9-read-unconfirmed", "quorum"}}, false},
      {StatusCode::UNAVAILABLE, {{"kv9-read-unconfirmed", "later"}}, true},
      {StatusCode::RESOURCE_EXHAUSTED, {{"kv9-admission-refused", "mood"}}, false},
      {StatusCode::INTERNAL, {{"kv9-something-new", "1"}}, false},
  };
  for (const auto& b : bad) {
    auto error = Run(b.code, b.markers, b.read);
    assert(error.kind == ErrorKind::kProtocol);
  }

  auto stale = Run(StatusCode::FAILED_PRECONDITION, {}, false);
  assert(stale.kind == ErrorKind::kRpc && stale.precondition());
  auto deadline = Run(StatusCode::DEADLINE_EXCEEDED, {}, false);
  assert(deadline.kind == ErrorKind::kRpc && !deadline.precondition());

  // Local bounds refuse before any dial.
  kv9::Client client("127.0.0.1:1", "t");
  auto ks = client.Keyspace(7);
  auto too_long = ks.Put(std::string(kv9::kMaxKeyBytes + 1, 'x'), "");
  assert(!too_long && too_long.error().kind == ErrorKind::kBounds);
  auto empty = ks.Batch({});
  assert(!empty && empty.error().kind == ErrorKind::kBounds);

  std::puts("kv9 c++ classify test: OK");
  return 0;
}
