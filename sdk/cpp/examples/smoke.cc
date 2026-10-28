// End-to-end smoke used by scripts/sdk-e2e.sh. Reads KV9_ADDR, KV9_TOKEN, KV9_KEYSPACE
// and optionally KV9_PEERS (`1=host:port,2=…`).
#include <cstdio>
#include <cstdlib>
#include <sstream>
#include <string>

#include "kv9/client.h"

namespace {

std::string Env(const char* name) {
  const char* value = std::getenv(name);
  if (!value || !*value) {
    std::fprintf(stderr, "%s must be set\n", name);
    std::exit(2);
  }
  return value;
}

template <typename T>
const T& Must(const kv9::Expected<T>& result, const char* what) {
  if (!result) {
    std::fprintf(stderr, "%s failed: kind=%d %s\n", what, static_cast<int>(result.error().kind),
                 result.error().message.c_str());
    std::exit(1);
  }
  return *result;
}

void Check(bool condition, const char* what) {
  if (!condition) {
    std::fprintf(stderr, "check failed: %s\n", what);
    std::exit(1);
  }
}

}  // namespace

int main() {
  kv9::ClientOptions options;
  if (const char* peers = std::getenv("KV9_PEERS")) {
    std::stringstream stream(peers);
    std::string item;
    while (std::getline(stream, item, ',')) {
      const auto eq = item.find('=');
      if (eq != std::string::npos) options.peers[std::stoull(item.substr(0, eq))] = item.substr(eq + 1);
    }
  }
  kv9::Client client(Env("KV9_ADDR"), Env("KV9_TOKEN"), options);
  Check(Must(client.GetClusterInfo(), "ClusterInfo").node_count >= 1, "node_count");
  auto ks = client.Keyspace(static_cast<std::uint32_t>(std::stoul(Env("KV9_KEYSPACE"))));
  auto p = [](const char* suffix) { return std::string("cpp-smoke/") + suffix; };

  const auto first = Must(ks.Put(p("k1"), "v1"), "Put");
  Check(first.term > 0 && first.index > 0, "applied position");
  Check(Must(ks.Get(p("k1")), "Get") == std::optional<std::string>("v1"), "get k1");
  Check(!Must(ks.Get(p("absent")), "Get absent").has_value(), "absent");

  const auto second = Must(ks.Batch({kv9::Mutation::Put(p("k2"), "v2"), kv9::Mutation::Put(p("k3"), "v3"),
                                     kv9::Mutation::Delete(p("k1"))}),
                           "Batch");
  Check(second.term > first.term || second.index > first.index, "batch advanced");
  const auto values = Must(ks.BatchGet({p("k1"), p("k2"), p("k3"), p("k2")}), "BatchGet");
  Check(!values[0] && values[1] == "v2" && values[2] == "v3" && values[3] == "v2", "batch_get alignment");
  const auto rows = Must(ks.ScanAll(p(""), p("\x7f")), "ScanAll");
  Check(rows.size() == 2 && rows[0].first == p("k2") && rows[1].first == p("k3"), "scan order");

  Must(ks.BatchPut({{p("k4"), "v4"}, {p("k5"), "v5"}}), "BatchPut");
  Check(Must(ks.Scan(p(""), p("\x7f"), 2), "Scan").pairs.size() == 2, "scan limit");

  const auto deleted = Must(ks.DeleteRange(p(""), p("\x7f")), "DeleteRange");
  Check(deleted.committed_chunks >= 1, "committed chunks");
  Check(Must(ks.ScanAll(p(""), p("\x7f")), "ScanAll after delete").empty(), "range emptied");

  auto too_long = ks.Put(std::string(kv9::kMaxKeyBytes + 1, 'x'), "v");
  Check(!too_long && too_long.error().kind == kv9::ErrorKind::kBounds, "bounds");
  std::printf("c++ sdk smoke: OK (leader %s)\n", client.address().c_str());
  return 0;
}
