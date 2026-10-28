// kv9 client SDK for C++17 on grpc++.
//
//   kv9::Client client("127.0.0.1:20161", "token", {.peers = {{1, "127.0.0.1:20161"}}});
//   auto ks = client.Keyspace(*client.CreateKeyspace("app"));
//   auto applied = ks.Put("k", "v");            // kv9::Expected<kv9::Applied>
//   ks.Batch({kv9::Mutation::Put("a", "1"), kv9::Mutation::Delete("k")});
//
// The contract shared by every kv9 SDK (auth header, request context and epoch
// refresh, typed refusal markers, bounds, redirect policy) is in sdk/README.md.
#pragma once

#include <grpcpp/support/status_code_enum.h>
#include <grpcpp/support/string_ref.h>

#include <chrono>
#include <cstdint>
#include <map>
#include <memory>
#include <optional>
#include <string>
#include <utility>
#include <vector>

namespace grpc {
class Status;
}

namespace kv9 {

constexpr std::size_t kMaxKeyBytes = 4096;
constexpr std::size_t kMaxValueBytes = 65'536;
constexpr std::size_t kMaxBatchItems = 256;
constexpr std::size_t kMaxMessageBytes = 1'048'576;
constexpr std::chrono::milliseconds kMaxDeadline{30'000};

// The region epoch a request believes in. Metadata-group keyspaces live at (1, 1).
struct Epoch {
  std::uint64_t conf_ver = 1;
  std::uint64_t version = 1;
  friend bool operator==(const Epoch& a, const Epoch& b) {
    return a.conf_ver == b.conf_ver && a.version == b.version;
  }
  friend bool operator!=(const Epoch& a, const Epoch& b) { return !(a == b); }
};

// The exact applied position of a write. Compare both fields, never the index alone.
struct Applied {
  std::uint64_t term = 0;
  std::uint64_t index = 0;
};

struct RegionLocation {
  std::uint64_t region_id = 0;
  Epoch epoch;
  std::optional<std::uint64_t> leader_node_id;
};

struct KeyspaceInfo {
  std::uint32_t id = 0;
  std::string name;
  std::uint64_t tenant_id = 0;
};

struct ClusterInfo {
  std::uint64_t node_count = 0;
  std::uint64_t keyspace_count = 0;
  std::uint64_t region_count = 0;
};

struct ScanPage {
  std::vector<std::pair<std::string, std::string>> pairs;
  // Nonempty when the walk paused at a foreign-leader chunk: continue exactly here.
  std::string resume_from;
};

struct DeleteRangeResult {
  std::uint64_t committed_chunks = 0;
  Applied last_applied;
  std::string resume_from;
};

// One entry of an atomic range batch; no value means delete.
struct Mutation {
  std::string key;
  std::optional<std::string> value;
  static Mutation Put(std::string key, std::string value) { return {std::move(key), std::move(value)}; }
  static Mutation Delete(std::string key) { return {std::move(key), std::nullopt}; }
};

enum class ErrorKind {
  kBounds,            // a local bound was violated; nothing was sent
  kNotLeader,         // not the leader; nothing executed (see leader_node_id)
  kReadUnconfirmed,   // the read barrier did not establish (phase: "quorum" | "apply")
  kAdmissionRefused,  // refused before execution (reason)
  kPartialWrite,      // delete_range committed some chunks, then failed
  kRpc,               // unmarked status; a write's outcome is unknown
  kProtocol,          // the server sent a marker the SDK cannot interpret
};

struct Error {
  ErrorKind kind = ErrorKind::kRpc;
  std::string message;
  grpc::StatusCode code = grpc::StatusCode::OK;
  std::optional<std::uint64_t> leader_node_id;  // kNotLeader
  std::string phase;                            // kReadUnconfirmed
  std::string reason;                           // kAdmissionRefused
  std::uint64_t committed_chunks = 0;           // kPartialWrite
  Applied last_applied;                         // kPartialWrite

  // FAILED_PRECONDITION without a marker: stale epoch, cross-region batch or
  // API-type mismatch. The keyspace handle refreshes its epoch once on this.
  bool precondition() const {
    return kind == ErrorKind::kRpc && code == grpc::StatusCode::FAILED_PRECONDITION;
  }
};

// A value or an Error. `ok()` tells which; `*` / `->` read the value.
template <typename T>
class Expected {
 public:
  Expected(T value) : value_(std::move(value)) {}  // NOLINT(google-explicit-constructor)
  Expected(Error error) : error_(std::move(error)) {}  // NOLINT(google-explicit-constructor)
  bool ok() const { return value_.has_value(); }
  explicit operator bool() const { return ok(); }
  const T& value() const { return *value_; }
  T& value() { return *value_; }
  const T& operator*() const { return *value_; }
  T& operator*() { return *value_; }
  const T* operator->() const { return &*value_; }
  T* operator->() { return &*value_; }
  const Error& error() const { return *error_; }

 private:
  std::optional<T> value_;
  std::optional<Error> error_;
};

// Decode a failed call into the typed error its trailing markers describe.
Error Classify(const grpc::Status& status,
               const std::multimap<grpc::string_ref, grpc::string_ref>& trailing, bool read);

struct ClientOptions {
  std::chrono::milliseconds timeout{5000};    // per call, at most 30 s
  std::map<std::uint64_t, std::string> peers;  // node_id -> address, to follow NotLeader hints
  int max_redirects = 3;
};

class Keyspace;

// An authenticated client for one kv9 cluster. Copyable; copies share the connections.
class Client {
 public:
  Client(std::string address, std::string token, ClientOptions options = {});

  Expected<std::uint32_t> CreateKeyspace(const std::string& name, std::uint64_t tenant_id = 0);
  Expected<std::vector<KeyspaceInfo>> ListKeyspaces();
  Expected<ClusterInfo> GetClusterInfo();
  Expected<RegionLocation> GetRegion(std::uint32_t keyspace_id, const std::string& key = "");

  // A handle for raw operations, starting at epoch (1, 1) with one-shot auto refresh.
  kv9::Keyspace Keyspace(std::uint32_t keyspace_id, Epoch epoch = {}, bool auto_refresh = true);

  // The node currently addressed (moves with followed hints).
  std::string address() const;

  struct Impl;  // opaque; shared between a Client and the Keyspace handles it hands out

 private:
  friend class Keyspace;
  std::shared_ptr<Impl> impl_;
};

// Raw operations on one keyspace under one region epoch.
class Keyspace {
 public:
  std::uint32_t id() const { return id_; }
  Epoch epoch() const { return epoch_; }
  Expected<Epoch> RefreshEpoch(const std::string& key = "");

  Expected<std::optional<std::string>> Get(const std::string& key);
  // One quorum-confirmed view; results align with `keys`, duplicates included.
  Expected<std::vector<std::optional<std::string>>> BatchGet(const std::vector<std::string>& keys);
  // One page of [start, end); an empty `end` means "to the keyspace end".
  Expected<ScanPage> Scan(const std::string& start, const std::string& end, std::uint32_t limit = 1024);
  // Walk [start, end) page by page, following resume_from.
  Expected<std::vector<std::pair<std::string, std::string>>> ScanAll(const std::string& start,
                                                                     const std::string& end,
                                                                     std::uint32_t page = 1024);

  Expected<Applied> Put(const std::string& key, const std::string& value);
  Expected<Applied> Delete(const std::string& key);
  // Put-only atomic batch: every pair lands at one applied position.
  Expected<Applied> BatchPut(const std::vector<std::pair<std::string, std::string>>& pairs);
  // The atomic range batch: ordered puts and deletes applied as ONE Raft entry. Every
  // key must lie in the same region; a batch crossing a region is refused whole and
  // never split by the SDK. A later mutation for the same key wins.
  Expected<Applied> Batch(const std::vector<Mutation>& mutations);
  // Delete [start, end) in bounded chunks; not atomic as a whole (see kPartialWrite).
  Expected<DeleteRangeResult> DeleteRange(const std::string& start, const std::string& end);

 private:
  friend class Client;
  Keyspace(std::shared_ptr<Client::Impl> impl, std::uint32_t id, Epoch epoch, bool auto_refresh)
      : impl_(std::move(impl)), id_(id), epoch_(epoch), auto_refresh_(auto_refresh) {}

  template <typename Req, typename Resp, typename Call>
  Expected<Resp> CallWithContext(bool read, const std::string& anchor, Req request, Call call);

  std::shared_ptr<Client::Impl> impl_;
  std::uint32_t id_;
  Epoch epoch_;
  bool auto_refresh_;
};

}  // namespace kv9
