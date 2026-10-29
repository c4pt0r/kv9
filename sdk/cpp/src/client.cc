#include "kv9/client.h"

#include <grpcpp/grpcpp.h>

#include <algorithm>
#include <mutex>
#include <string_view>

#include "kv9.grpc.pb.h"

namespace kv9 {
namespace {

namespace pb = ::kv9::v1;

constexpr std::string_view kNotLeaderKey = "kv9-not-leader";
constexpr std::string_view kLeaderHintKey = "kv9-leader-node-id";
constexpr std::string_view kReadUnconfirmedKey = "kv9-read-unconfirmed";
constexpr std::string_view kAdmissionRefusedKey = "kv9-admission-refused";
constexpr std::string_view kPartialWriteKey = "kv9-partial-write";
constexpr std::string_view kCommittedChunksKey = "kv9-committed-chunks";
constexpr std::string_view kLastAppliedTermKey = "kv9-last-applied-term";
constexpr std::string_view kLastAppliedIndexKey = "kv9-last-applied-index";

using Trailing = std::multimap<grpc::string_ref, grpc::string_ref>;

std::string_view View(grpc::string_ref ref) { return {ref.data(), ref.size()}; }

Error Protocol(std::string message) {
  Error error;
  error.kind = ErrorKind::kProtocol;
  error.message = std::move(message);
  return error;
}

Error Bounds(std::string message) {
  Error error;
  error.kind = ErrorKind::kBounds;
  error.message = std::move(message);
  return error;
}

struct Marker {
  bool present = false;
  bool repeated = false;
  std::string value;
};

Marker Single(const Trailing& trailing, std::string_view key) {
  Marker marker;
  for (const auto& [k, v] : trailing) {
    if (View(k) != key) continue;
    if (marker.present) marker.repeated = true;
    marker.present = true;
    marker.value = std::string(View(v));
  }
  return marker;
}

bool Canonical(const std::string& text, std::uint64_t* out) {
  if (text.empty() || text.size() > 20) return false;
  if (text.size() > 1 && text[0] == '0') return false;
  std::uint64_t value = 0;
  for (char c : text) {
    if (c < '0' || c > '9') return false;
    const std::uint64_t digit = static_cast<std::uint64_t>(c - '0');
    if (value > (UINT64_MAX - digit) / 10) return false;
    value = value * 10 + digit;
  }
  *out = value;
  return true;
}

bool Integer(const Trailing& trailing, std::string_view key, std::uint64_t* out, Error* error) {
  const Marker marker = Single(trailing, key);
  if (!marker.present || marker.repeated || !Canonical(marker.value, out)) {
    *error = Protocol("marker " + std::string(key) + " is not a canonical integer");
    return false;
  }
  return true;
}

}  // namespace

Error Classify(const grpc::Status& status, const Trailing& trailing, bool read) {
  Error error;
  error.code = status.error_code();
  error.message = status.error_message();

  if (Marker m = Single(trailing, kNotLeaderKey); m.present) {
    if (m.repeated || m.value != "true" || status.error_code() != grpc::StatusCode::FAILED_PRECONDITION) {
      return Protocol("malformed not-leader marker");
    }
    error.kind = ErrorKind::kNotLeader;
    if (Marker hint = Single(trailing, kLeaderHintKey); hint.present) {
      std::uint64_t id = 0;
      Error bad;
      if (!Integer(trailing, kLeaderHintKey, &id, &bad)) return bad;
      if (id == 0) return Protocol("leader hint names node 0");
      error.leader_node_id = id;
    }
    return error;
  }
  if (Marker m = Single(trailing, kAdmissionRefusedKey); m.present) {
    if (m.repeated || (m.value != "request_count" && m.value != "encoded_bytes" && m.value != "request_too_large")) {
      return Protocol("unknown admission refusal");
    }
    error.kind = ErrorKind::kAdmissionRefused;
    error.reason = m.value;
    return error;
  }
  if (Marker m = Single(trailing, kPartialWriteKey); m.present) {
    if (m.repeated || m.value != "true") return Protocol("malformed partial-write marker");
    error.kind = ErrorKind::kPartialWrite;
    Error bad;
    if (!Integer(trailing, kCommittedChunksKey, &error.committed_chunks, &bad) ||
        !Integer(trailing, kLastAppliedTermKey, &error.last_applied.term, &bad) ||
        !Integer(trailing, kLastAppliedIndexKey, &error.last_applied.index, &bad)) {
      return bad;
    }
    return error;
  }
  if (Marker m = Single(trailing, kReadUnconfirmedKey); m.present) {
    if (!read || m.repeated || status.error_code() != grpc::StatusCode::UNAVAILABLE ||
        (m.value != "quorum" && m.value != "apply")) {
      return Protocol("malformed read-unconfirmed marker");
    }
    error.kind = ErrorKind::kReadUnconfirmed;
    error.phase = m.value;
    return error;
  }
  for (const auto& [k, _] : trailing) {
    if (View(k).substr(0, 4) == "kv9-") return Protocol("unexpected control marker " + std::string(View(k)));
  }
  error.kind = ErrorKind::kRpc;
  return error;
}

// ----------------------------------------------------------------------------- Client

struct Client::Impl {
  std::mutex mu;
  std::string address;
  std::string authorization;
  ClientOptions options;
  std::map<std::string, std::shared_ptr<grpc::Channel>> channels;

  std::shared_ptr<grpc::Channel> ChannelFor(const std::string& target) {
    std::lock_guard<std::mutex> lock(mu);
    auto it = channels.find(target);
    if (it != channels.end()) return it->second;
    grpc::ChannelArguments args;
    args.SetMaxReceiveMessageSize(static_cast<int>(kMaxMessageBytes));
    args.SetMaxSendMessageSize(static_cast<int>(kMaxMessageBytes));
    auto channel = grpc::CreateCustomChannel(target, grpc::InsecureChannelCredentials(), args);
    channels.emplace(target, channel);
    return channel;
  }

  std::string Address() {
    std::lock_guard<std::mutex> lock(mu);
    return address;
  }

  // One logical call: authenticate, bound the deadline, follow NotLeader hints.
  template <typename Req, typename Resp, typename Call>
  Expected<Resp> Invoke(bool read, const Req& request, Call call) {
    int hops = 0;
    for (;;) {
      const std::string target = Address();
      auto stub = pb::Kv9::NewStub(ChannelFor(target));
      grpc::ClientContext context;
      context.AddMetadata("authorization", authorization);
      context.set_deadline(std::chrono::system_clock::now() + options.timeout);
      Resp response;
      const grpc::Status status = call(*stub, context, request, &response);
      if (status.ok()) return response;
      Error error = Classify(status, context.GetServerTrailingMetadata(), read);
      if (error.kind == ErrorKind::kNotLeader && error.leader_node_id && hops < options.max_redirects) {
        auto peer = options.peers.find(*error.leader_node_id);
        if (peer != options.peers.end() && peer->second != target) {
          std::lock_guard<std::mutex> lock(mu);
          address = peer->second;
          ++hops;
          continue;
        }
      }
      return error;
    }
  }
};

Client::Client(std::string address, std::string token, ClientOptions options) : impl_(std::make_shared<Impl>()) {
  impl_->address = std::move(address);
  impl_->authorization = "Bearer " + token;
  impl_->options = std::move(options);
  impl_->options.timeout = std::min(impl_->options.timeout, kMaxDeadline);
}

std::string Client::address() const { return impl_->Address(); }

Expected<std::uint32_t> Client::CreateKeyspace(const std::string& name, std::uint64_t tenant_id) {
  pb::CreateKeyspaceRequest request;
  request.set_name(name);
  request.set_tenant_id(tenant_id);
  request.set_api_type(pb::API_TYPE_RAW);
  auto result = impl_->Invoke<pb::CreateKeyspaceRequest, pb::CreateKeyspaceResponse>(
      false, request, [](pb::Kv9::Stub& s, grpc::ClientContext& c, const auto& q, auto* r) {
        return s.CreateKeyspace(&c, q, r);
      });
  if (!result) return result.error();
  return result->keyspace_id();
}

Expected<std::vector<KeyspaceInfo>> Client::ListKeyspaces() {
  auto result = impl_->Invoke<pb::ListKeyspacesRequest, pb::ListKeyspacesResponse>(
      true, pb::ListKeyspacesRequest{}, [](pb::Kv9::Stub& s, grpc::ClientContext& c, const auto& q, auto* r) {
        return s.ListKeyspaces(&c, q, r);
      });
  if (!result) return result.error();
  std::vector<KeyspaceInfo> out;
  for (const auto& k : result->keyspaces()) out.push_back({k.id(), k.name(), k.tenant_id()});
  return out;
}

Expected<ClusterInfo> Client::GetClusterInfo() {
  auto result = impl_->Invoke<pb::ClusterInfoRequest, pb::ClusterInfoResponse>(
      true, pb::ClusterInfoRequest{}, [](pb::Kv9::Stub& s, grpc::ClientContext& c, const auto& q, auto* r) {
        return s.ClusterInfo(&c, q, r);
      });
  if (!result) return result.error();
  return ClusterInfo{result->node_count(), result->keyspace_count(), result->region_count()};
}

namespace {

Expected<RegionLocation> GetRegionWith(Client::Impl& impl, std::uint32_t keyspace_id, const std::string& key) {
  if (key.size() > kMaxKeyBytes) return Bounds("key exceeds 4096 bytes");
  pb::GetRegionRequest request;
  request.set_keyspace_id(keyspace_id);
  request.set_key(key);
  auto result = impl.Invoke<pb::GetRegionRequest, pb::GetRegionResponse>(
      true, request, [](pb::Kv9::Stub& s, grpc::ClientContext& c, const auto& q, auto* r) {
        return s.GetRegion(&c, q, r);
      });
  if (!result) return result.error();
  if (!result->has_region() || !result->region().has_epoch()) return Protocol("GetRegion answered without a region");
  const auto& region = result->region();
  RegionLocation location;
  location.region_id = region.region_id();
  location.epoch = {region.epoch().conf_ver(), region.epoch().version()};
  if (region.has_leader_node_id()) location.leader_node_id = region.leader_node_id();
  return location;
}

}  // namespace

Expected<RegionLocation> Client::GetRegion(std::uint32_t keyspace_id, const std::string& key) {
  return GetRegionWith(*impl_, keyspace_id, key);
}

kv9::Keyspace Client::Keyspace(std::uint32_t keyspace_id, Epoch epoch, bool auto_refresh) {
  return kv9::Keyspace(impl_, keyspace_id, epoch, auto_refresh);
}

Expected<kv9::Keyspace> Client::KeyspaceNamed(const std::string& name, Epoch epoch,
                                              bool auto_refresh) {
  auto keyspaces = ListKeyspaces();
  if (!keyspaces) return keyspaces.error();
  for (const auto& info : *keyspaces) {
    if (info.name == name) {
      kv9::Keyspace ks(impl_, info.id, epoch, auto_refresh);
      ks.name_ = info.name;
      return ks;
    }
  }
  Error error;
  error.kind = ErrorKind::kRpc;
  error.code = grpc::StatusCode::NOT_FOUND;
  error.message = "no keyspace named \"" + name + "\"";
  return error;
}

// --------------------------------------------------------------------------- Keyspace

namespace {

Expected<Applied> AppliedOf(const pb::RawWriteResponse& r) {
  if (r.applied_term() == 0 || r.applied_index() == 0) {
    return Protocol("write acknowledged without an applied position");
  }
  return Applied{r.applied_term(), r.applied_index()};
}

std::optional<Error> CheckKey(const std::string& key, const char* what = "key") {
  if (key.size() > kMaxKeyBytes) return Bounds(std::string(what) + " exceeds 4096 bytes");
  return std::nullopt;
}

std::optional<Error> CheckValue(const std::string& value) {
  if (value.size() > kMaxValueBytes) return Bounds("value exceeds 65536 bytes");
  return std::nullopt;
}

std::optional<Error> CheckCount(std::size_t count) {
  if (count < 1 || count > kMaxBatchItems) return Bounds("a batch carries 1..=256 items");
  return std::nullopt;
}

}  // namespace

template <typename Req, typename Resp, typename Call>
Expected<Resp> Keyspace::CallWithContext(bool read, const std::string& anchor, Req request, Call call) {
  auto stamp = [&](Req& r) {
    auto* context = r.mutable_context();
    context->set_keyspace_id(id_);
    context->mutable_region_epoch()->set_conf_ver(epoch_.conf_ver);
    context->mutable_region_epoch()->set_version(epoch_.version);
  };
  stamp(request);
  if (request.ByteSizeLong() > kMaxMessageBytes) return Bounds("encoded request exceeds 1 MiB");
  auto result = impl_->Invoke<Req, Resp>(read, request, call);
  if (result || !result.error().precondition() || !auto_refresh_) return result;
  const Epoch before = epoch_;
  auto refreshed = RefreshEpoch(anchor);
  if (!refreshed) return refreshed.error();
  if (*refreshed == before) return result;
  stamp(request);
  return impl_->Invoke<Req, Resp>(read, request, call);
}

Expected<Epoch> Keyspace::RefreshEpoch(const std::string& key) {
  auto location = GetRegionWith(*impl_, id_, key);
  if (!location) return location.error();
  epoch_ = location->epoch;
  return epoch_;
}

Expected<std::optional<std::string>> Keyspace::Get(const std::string& key) {
  if (auto bad = CheckKey(key)) return *bad;
  pb::RawGetRequest request;
  request.set_key(key);
  auto result = CallWithContext<pb::RawGetRequest, pb::RawGetResponse>(
      true, key, std::move(request), [](pb::Kv9::Stub& s, grpc::ClientContext& c, const auto& q, auto* r) {
        return s.RawGet(&c, q, r);
      });
  if (!result) return result.error();
  if (result->has_value() && result->value().found()) return std::optional<std::string>(result->value().value());
  return std::optional<std::string>();
}

Expected<std::vector<std::optional<std::string>>> Keyspace::BatchGet(const std::vector<std::string>& keys) {
  if (auto bad = CheckCount(keys.size())) return *bad;
  pb::RawBatchGetRequest request;
  for (const auto& key : keys) {
    if (auto bad = CheckKey(key)) return *bad;
    request.add_keys(key);
  }
  auto result = CallWithContext<pb::RawBatchGetRequest, pb::RawBatchGetResponse>(
      true, keys.front(), std::move(request), [](pb::Kv9::Stub& s, grpc::ClientContext& c, const auto& q, auto* r) {
        return s.RawBatchGet(&c, q, r);
      });
  if (!result) return result.error();
  if (static_cast<std::size_t>(result->values_size()) != keys.size()) {
    return Protocol("batch_get returned a different cardinality");
  }
  std::vector<std::optional<std::string>> out;
  out.reserve(keys.size());
  for (const auto& v : result->values()) {
    out.push_back(v.found() ? std::optional<std::string>(v.value()) : std::nullopt);
  }
  return out;
}

Expected<ScanPage> Keyspace::Scan(const std::string& start, const std::string& end, std::uint32_t limit) {
  if (auto bad = CheckKey(start, "start")) return *bad;
  if (auto bad = CheckKey(end, "end")) return *bad;
  if (limit == 0) return Bounds("scan limit must be positive");
  pb::RawScanRequest request;
  request.set_start(start);
  request.set_end(end);
  request.set_limit(limit);
  auto result = CallWithContext<pb::RawScanRequest, pb::ScanResponse>(
      true, start, std::move(request), [](pb::Kv9::Stub& s, grpc::ClientContext& c, const auto& q, auto* r) {
        return s.RawScan(&c, q, r);
      });
  if (!result) return result.error();
  ScanPage page;
  for (const auto& pair : result->pairs()) page.pairs.emplace_back(pair.key(), pair.value());
  page.resume_from = result->resume_from();
  return page;
}

Expected<std::vector<std::pair<std::string, std::string>>> Keyspace::ScanAll(const std::string& start,
                                                                             const std::string& end,
                                                                             std::uint32_t page) {
  std::vector<std::pair<std::string, std::string>> out;
  std::string cursor = start;
  for (;;) {
    auto result = Scan(cursor, end, page);
    if (!result) return result.error();
    const std::size_t count = result->pairs.size();
    out.insert(out.end(), result->pairs.begin(), result->pairs.end());
    if (!result->resume_from.empty()) {
      cursor = result->resume_from;
    } else if (count < page) {
      return out;
    } else {
      cursor = out.back().first + std::string(1, '\0');
    }
  }
}

Expected<Applied> Keyspace::Put(const std::string& key, const std::string& value) {
  if (auto bad = CheckKey(key)) return *bad;
  if (auto bad = CheckValue(value)) return *bad;
  pb::RawPutRequest request;
  request.set_key(key);
  request.set_value(value);
  auto result = CallWithContext<pb::RawPutRequest, pb::RawWriteResponse>(
      false, key, std::move(request), [](pb::Kv9::Stub& s, grpc::ClientContext& c, const auto& q, auto* r) {
        return s.RawPut(&c, q, r);
      });
  if (!result) return result.error();
  return AppliedOf(*result);
}

Expected<Applied> Keyspace::Delete(const std::string& key) {
  if (auto bad = CheckKey(key)) return *bad;
  pb::RawDeleteRequest request;
  request.set_key(key);
  auto result = CallWithContext<pb::RawDeleteRequest, pb::RawWriteResponse>(
      false, key, std::move(request), [](pb::Kv9::Stub& s, grpc::ClientContext& c, const auto& q, auto* r) {
        return s.RawDelete(&c, q, r);
      });
  if (!result) return result.error();
  return AppliedOf(*result);
}

Expected<Applied> Keyspace::BatchPut(const std::vector<std::pair<std::string, std::string>>& pairs) {
  if (auto bad = CheckCount(pairs.size())) return *bad;
  pb::RawBatchPutRequest request;
  for (const auto& [key, value] : pairs) {
    if (auto bad = CheckKey(key)) return *bad;
    if (auto bad = CheckValue(value)) return *bad;
    auto* pair = request.add_pairs();
    pair->set_key(key);
    pair->set_value(value);
  }
  auto result = CallWithContext<pb::RawBatchPutRequest, pb::RawWriteResponse>(
      false, pairs.front().first, std::move(request),
      [](pb::Kv9::Stub& s, grpc::ClientContext& c, const auto& q, auto* r) { return s.RawBatchPut(&c, q, r); });
  if (!result) return result.error();
  return AppliedOf(*result);
}

Expected<Applied> Keyspace::Batch(const std::vector<Mutation>& mutations) {
  if (auto bad = CheckCount(mutations.size())) return *bad;
  pb::RawBatchRequest request;
  for (const auto& mutation : mutations) {
    if (auto bad = CheckKey(mutation.key)) return *bad;
    auto* wire = request.add_mutations();
    if (mutation.value) {
      if (auto bad = CheckValue(*mutation.value)) return *bad;
      wire->mutable_put()->set_key(mutation.key);
      wire->mutable_put()->set_value(*mutation.value);
    } else {
      wire->set_delete_(mutation.key);
    }
  }
  auto result = CallWithContext<pb::RawBatchRequest, pb::RawWriteResponse>(
      false, mutations.front().key, std::move(request),
      [](pb::Kv9::Stub& s, grpc::ClientContext& c, const auto& q, auto* r) { return s.RawBatch(&c, q, r); });
  if (!result) return result.error();
  return AppliedOf(*result);
}

Expected<DeleteRangeResult> Keyspace::DeleteRange(const std::string& start, const std::string& end) {
  if (auto bad = CheckKey(start, "start")) return *bad;
  if (auto bad = CheckKey(end, "end")) return *bad;
  pb::RawDeleteRangeRequest request;
  request.set_start(start);
  request.set_end(end);
  auto result = CallWithContext<pb::RawDeleteRangeRequest, pb::RawDeleteRangeResponse>(
      false, start, std::move(request),
      [](pb::Kv9::Stub& s, grpc::ClientContext& c, const auto& q, auto* r) { return s.RawDeleteRange(&c, q, r); });
  if (!result) return result.error();
  DeleteRangeResult out;
  out.committed_chunks = result->committed_chunks();
  out.last_applied = {result->last_applied_term(), result->last_applied_index()};
  out.resume_from = result->resume_from();
  return out;
}

}  // namespace kv9
