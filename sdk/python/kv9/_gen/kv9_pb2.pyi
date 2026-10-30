from google.protobuf.internal import containers as _containers
from google.protobuf.internal import enum_type_wrapper as _enum_type_wrapper
from google.protobuf import descriptor as _descriptor
from google.protobuf import message as _message
from collections.abc import Iterable as _Iterable, Mapping as _Mapping
from typing import ClassVar as _ClassVar, Optional as _Optional, Union as _Union

DESCRIPTOR: _descriptor.FileDescriptor

class EndpointRefusal(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    ENDPOINT_REFUSAL_UNSPECIFIED: _ClassVar[EndpointRefusal]
    ENDPOINT_REFUSAL_WRONG_CLUSTER: _ClassVar[EndpointRefusal]
    ENDPOINT_REFUSAL_MISSING_NODE: _ClassVar[EndpointRefusal]
    ENDPOINT_REFUSAL_INACTIVE_NODE: _ClassVar[EndpointRefusal]
    ENDPOINT_REFUSAL_INVALID_INCARNATION: _ClassVar[EndpointRefusal]
    ENDPOINT_REFUSAL_CONFLICT: _ClassVar[EndpointRefusal]
    ENDPOINT_REFUSAL_GENERATION_EXHAUSTED: _ClassVar[EndpointRefusal]

class ApiType(int, metaclass=_enum_type_wrapper.EnumTypeWrapper):
    __slots__ = ()
    API_TYPE_UNSPECIFIED: _ClassVar[ApiType]
    API_TYPE_TXN: _ClassVar[ApiType]
    API_TYPE_RAW: _ClassVar[ApiType]
ENDPOINT_REFUSAL_UNSPECIFIED: EndpointRefusal
ENDPOINT_REFUSAL_WRONG_CLUSTER: EndpointRefusal
ENDPOINT_REFUSAL_MISSING_NODE: EndpointRefusal
ENDPOINT_REFUSAL_INACTIVE_NODE: EndpointRefusal
ENDPOINT_REFUSAL_INVALID_INCARNATION: EndpointRefusal
ENDPOINT_REFUSAL_CONFLICT: EndpointRefusal
ENDPOINT_REFUSAL_GENERATION_EXHAUSTED: EndpointRefusal
API_TYPE_UNSPECIFIED: ApiType
API_TYPE_TXN: ApiType
API_TYPE_RAW: ApiType

class Empty(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class RawTtlRequest(_message.Message):
    __slots__ = ("binding", "protocol_version", "budget_micros", "capabilities", "get", "put", "delete_key", "scan", "delete_range")
    BINDING_FIELD_NUMBER: _ClassVar[int]
    PROTOCOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    BUDGET_MICROS_FIELD_NUMBER: _ClassVar[int]
    CAPABILITIES_FIELD_NUMBER: _ClassVar[int]
    GET_FIELD_NUMBER: _ClassVar[int]
    PUT_FIELD_NUMBER: _ClassVar[int]
    DELETE_KEY_FIELD_NUMBER: _ClassVar[int]
    SCAN_FIELD_NUMBER: _ClassVar[int]
    DELETE_RANGE_FIELD_NUMBER: _ClassVar[int]
    binding: bytes
    protocol_version: int
    budget_micros: int
    capabilities: Empty
    get: RoutedKeys
    put: RawTtlPut
    delete_key: bytes
    scan: RawTtlScan
    delete_range: RawTtlScan
    def __init__(self, binding: _Optional[bytes] = ..., protocol_version: _Optional[int] = ..., budget_micros: _Optional[int] = ..., capabilities: _Optional[_Union[Empty, _Mapping]] = ..., get: _Optional[_Union[RoutedKeys, _Mapping]] = ..., put: _Optional[_Union[RawTtlPut, _Mapping]] = ..., delete_key: _Optional[bytes] = ..., scan: _Optional[_Union[RawTtlScan, _Mapping]] = ..., delete_range: _Optional[_Union[RawTtlScan, _Mapping]] = ...) -> None: ...

class ActivateRawTtlRequest(_message.Message):
    __slots__ = ("binding", "protocol_version", "budget_micros")
    BINDING_FIELD_NUMBER: _ClassVar[int]
    PROTOCOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    BUDGET_MICROS_FIELD_NUMBER: _ClassVar[int]
    binding: bytes
    protocol_version: int
    budget_micros: int
    def __init__(self, binding: _Optional[bytes] = ..., protocol_version: _Optional[int] = ..., budget_micros: _Optional[int] = ...) -> None: ...

class RawTtlPut(_message.Message):
    __slots__ = ("pairs", "ttl_secs")
    PAIRS_FIELD_NUMBER: _ClassVar[int]
    TTL_SECS_FIELD_NUMBER: _ClassVar[int]
    pairs: _containers.RepeatedCompositeFieldContainer[KeyValue]
    ttl_secs: int
    def __init__(self, pairs: _Optional[_Iterable[_Union[KeyValue, _Mapping]]] = ..., ttl_secs: _Optional[int] = ...) -> None: ...

class RawTtlScan(_message.Message):
    __slots__ = ("start", "end", "limit")
    START_FIELD_NUMBER: _ClassVar[int]
    END_FIELD_NUMBER: _ClassVar[int]
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    start: bytes
    end: bytes
    limit: int
    def __init__(self, start: _Optional[bytes] = ..., end: _Optional[bytes] = ..., limit: _Optional[int] = ...) -> None: ...

class RawTtlPage(_message.Message):
    __slots__ = ("pairs", "next", "examined")
    PAIRS_FIELD_NUMBER: _ClassVar[int]
    NEXT_FIELD_NUMBER: _ClassVar[int]
    EXAMINED_FIELD_NUMBER: _ClassVar[int]
    pairs: _containers.RepeatedCompositeFieldContainer[KeyValue]
    next: bytes
    examined: int
    def __init__(self, pairs: _Optional[_Iterable[_Union[KeyValue, _Mapping]]] = ..., next: _Optional[bytes] = ..., examined: _Optional[int] = ...) -> None: ...

class RawTtlDeleted(_message.Message):
    __slots__ = ("processed_keys", "applied", "next")
    PROCESSED_KEYS_FIELD_NUMBER: _ClassVar[int]
    APPLIED_FIELD_NUMBER: _ClassVar[int]
    NEXT_FIELD_NUMBER: _ClassVar[int]
    processed_keys: int
    applied: RawWriteResponse
    next: bytes
    def __init__(self, processed_keys: _Optional[int] = ..., applied: _Optional[_Union[RawWriteResponse, _Mapping]] = ..., next: _Optional[bytes] = ...) -> None: ...

class RawTtlResponse(_message.Message):
    __slots__ = ("binding_digest", "protocol_version", "capabilities", "values", "applied", "page", "deleted")
    BINDING_DIGEST_FIELD_NUMBER: _ClassVar[int]
    PROTOCOL_VERSION_FIELD_NUMBER: _ClassVar[int]
    CAPABILITIES_FIELD_NUMBER: _ClassVar[int]
    VALUES_FIELD_NUMBER: _ClassVar[int]
    APPLIED_FIELD_NUMBER: _ClassVar[int]
    PAGE_FIELD_NUMBER: _ClassVar[int]
    DELETED_FIELD_NUMBER: _ClassVar[int]
    binding_digest: bytes
    protocol_version: int
    capabilities: Empty
    values: RawBatchGetResponse
    applied: RawWriteResponse
    page: RawTtlPage
    deleted: RawTtlDeleted
    def __init__(self, binding_digest: _Optional[bytes] = ..., protocol_version: _Optional[int] = ..., capabilities: _Optional[_Union[Empty, _Mapping]] = ..., values: _Optional[_Union[RawBatchGetResponse, _Mapping]] = ..., applied: _Optional[_Union[RawWriteResponse, _Mapping]] = ..., page: _Optional[_Union[RawTtlPage, _Mapping]] = ..., deleted: _Optional[_Union[RawTtlDeleted, _Mapping]] = ...) -> None: ...

class CreateDataKeyspaceRequest(_message.Message):
    __slots__ = ("root_digest", "creation_task", "name", "tenant_id")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    CREATION_TASK_FIELD_NUMBER: _ClassVar[int]
    NAME_FIELD_NUMBER: _ClassVar[int]
    TENANT_ID_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    creation_task: int
    name: str
    tenant_id: int
    def __init__(self, root_digest: _Optional[bytes] = ..., creation_task: _Optional[int] = ..., name: _Optional[str] = ..., tenant_id: _Optional[int] = ...) -> None: ...

class CreateDataKeyspaceResponse(_message.Message):
    __slots__ = ("binding", "changed", "applied_term", "applied_index", "keyspace_id", "region_id")
    BINDING_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    APPLIED_TERM_FIELD_NUMBER: _ClassVar[int]
    APPLIED_INDEX_FIELD_NUMBER: _ClassVar[int]
    KEYSPACE_ID_FIELD_NUMBER: _ClassVar[int]
    REGION_ID_FIELD_NUMBER: _ClassVar[int]
    binding: bytes
    changed: bool
    applied_term: int
    applied_index: int
    keyspace_id: int
    region_id: int
    def __init__(self, binding: _Optional[bytes] = ..., changed: _Optional[bool] = ..., applied_term: _Optional[int] = ..., applied_index: _Optional[int] = ..., keyspace_id: _Optional[int] = ..., region_id: _Optional[int] = ...) -> None: ...

class CreateDataGroupRequest(_message.Message):
    __slots__ = ("root_digest", "operation_id", "voters")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    VOTERS_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    operation_id: bytes
    voters: _containers.RepeatedScalarFieldContainer[int]
    def __init__(self, root_digest: _Optional[bytes] = ..., operation_id: _Optional[bytes] = ..., voters: _Optional[_Iterable[int]] = ...) -> None: ...

class CreateDataGroupResponse(_message.Message):
    __slots__ = ("creation_intent", "changed", "applied_term", "applied_index", "creation_task", "region_id")
    CREATION_INTENT_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    APPLIED_TERM_FIELD_NUMBER: _ClassVar[int]
    APPLIED_INDEX_FIELD_NUMBER: _ClassVar[int]
    CREATION_TASK_FIELD_NUMBER: _ClassVar[int]
    REGION_ID_FIELD_NUMBER: _ClassVar[int]
    creation_intent: bytes
    changed: bool
    applied_term: int
    applied_index: int
    creation_task: int
    region_id: int
    def __init__(self, creation_intent: _Optional[bytes] = ..., changed: _Optional[bool] = ..., applied_term: _Optional[int] = ..., applied_index: _Optional[int] = ..., creation_task: _Optional[int] = ..., region_id: _Optional[int] = ...) -> None: ...

class MigrateDataGroupRequest(_message.Message):
    __slots__ = ("root_digest", "operation_id", "creation_task", "destination_node")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    CREATION_TASK_FIELD_NUMBER: _ClassVar[int]
    DESTINATION_NODE_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    operation_id: bytes
    creation_task: int
    destination_node: int
    def __init__(self, root_digest: _Optional[bytes] = ..., operation_id: _Optional[bytes] = ..., creation_task: _Optional[int] = ..., destination_node: _Optional[int] = ...) -> None: ...

class MigrateDataGroupResponse(_message.Message):
    __slots__ = ("migration_intent", "changed", "applied_term", "applied_index")
    MIGRATION_INTENT_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    APPLIED_TERM_FIELD_NUMBER: _ClassVar[int]
    APPLIED_INDEX_FIELD_NUMBER: _ClassVar[int]
    migration_intent: bytes
    changed: bool
    applied_term: int
    applied_index: int
    def __init__(self, migration_intent: _Optional[bytes] = ..., changed: _Optional[bool] = ..., applied_term: _Optional[int] = ..., applied_index: _Optional[int] = ...) -> None: ...

class BindMigrationImageRequest(_message.Message):
    __slots__ = ("root_digest", "operation_id", "manifest")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    MANIFEST_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    operation_id: bytes
    manifest: bytes
    def __init__(self, root_digest: _Optional[bytes] = ..., operation_id: _Optional[bytes] = ..., manifest: _Optional[bytes] = ...) -> None: ...

class AttachMigrationLearnerRequest(_message.Message):
    __slots__ = ("root_digest", "operation_id")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    operation_id: bytes
    def __init__(self, root_digest: _Optional[bytes] = ..., operation_id: _Optional[bytes] = ...) -> None: ...

class AttachMigrationLearnerResponse(_message.Message):
    __slots__ = ("destination_node", "changed", "cut_term", "cut_index")
    DESTINATION_NODE_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    CUT_TERM_FIELD_NUMBER: _ClassVar[int]
    CUT_INDEX_FIELD_NUMBER: _ClassVar[int]
    destination_node: int
    changed: bool
    cut_term: int
    cut_index: int
    def __init__(self, destination_node: _Optional[int] = ..., changed: _Optional[bool] = ..., cut_term: _Optional[int] = ..., cut_index: _Optional[int] = ...) -> None: ...

class PlanMigrationImageRequest(_message.Message):
    __slots__ = ("root_digest", "operation_id")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    operation_id: bytes
    def __init__(self, root_digest: _Optional[bytes] = ..., operation_id: _Optional[bytes] = ...) -> None: ...

class PlanMigrationImageResponse(_message.Message):
    __slots__ = ("manifest", "cut_term", "cut_index")
    MANIFEST_FIELD_NUMBER: _ClassVar[int]
    CUT_TERM_FIELD_NUMBER: _ClassVar[int]
    CUT_INDEX_FIELD_NUMBER: _ClassVar[int]
    manifest: bytes
    cut_term: int
    cut_index: int
    def __init__(self, manifest: _Optional[bytes] = ..., cut_term: _Optional[int] = ..., cut_index: _Optional[int] = ...) -> None: ...

class CaptureMigrationImageRequest(_message.Message):
    __slots__ = ("root_digest", "operation_id")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    operation_id: bytes
    def __init__(self, root_digest: _Optional[bytes] = ..., operation_id: _Optional[bytes] = ...) -> None: ...

class CaptureMigrationImageResponse(_message.Message):
    __slots__ = ("record", "image_digest", "cut_term", "cut_index", "configuration_applied_index", "objects", "object_bytes", "source_owner", "destination_owner")
    RECORD_FIELD_NUMBER: _ClassVar[int]
    IMAGE_DIGEST_FIELD_NUMBER: _ClassVar[int]
    CUT_TERM_FIELD_NUMBER: _ClassVar[int]
    CUT_INDEX_FIELD_NUMBER: _ClassVar[int]
    CONFIGURATION_APPLIED_INDEX_FIELD_NUMBER: _ClassVar[int]
    OBJECTS_FIELD_NUMBER: _ClassVar[int]
    OBJECT_BYTES_FIELD_NUMBER: _ClassVar[int]
    SOURCE_OWNER_FIELD_NUMBER: _ClassVar[int]
    DESTINATION_OWNER_FIELD_NUMBER: _ClassVar[int]
    record: bytes
    image_digest: bytes
    cut_term: int
    cut_index: int
    configuration_applied_index: int
    objects: int
    object_bytes: int
    source_owner: bytes
    destination_owner: bytes
    def __init__(self, record: _Optional[bytes] = ..., image_digest: _Optional[bytes] = ..., cut_term: _Optional[int] = ..., cut_index: _Optional[int] = ..., configuration_applied_index: _Optional[int] = ..., objects: _Optional[int] = ..., object_bytes: _Optional[int] = ..., source_owner: _Optional[bytes] = ..., destination_owner: _Optional[bytes] = ...) -> None: ...

class EmitInstallEvidenceRequest(_message.Message):
    __slots__ = ("root_digest", "region")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    REGION_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    region: int
    def __init__(self, root_digest: _Optional[bytes] = ..., region: _Optional[int] = ...) -> None: ...

class EmitInstallEvidenceResponse(_message.Message):
    __slots__ = ("receipt", "image_digest", "cut_term", "cut_index")
    RECEIPT_FIELD_NUMBER: _ClassVar[int]
    IMAGE_DIGEST_FIELD_NUMBER: _ClassVar[int]
    CUT_TERM_FIELD_NUMBER: _ClassVar[int]
    CUT_INDEX_FIELD_NUMBER: _ClassVar[int]
    receipt: bytes
    image_digest: bytes
    cut_term: int
    cut_index: int
    def __init__(self, receipt: _Optional[bytes] = ..., image_digest: _Optional[bytes] = ..., cut_term: _Optional[int] = ..., cut_index: _Optional[int] = ...) -> None: ...

class RecordInstallEvidenceRequest(_message.Message):
    __slots__ = ("root_digest", "receipt")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    RECEIPT_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    receipt: bytes
    def __init__(self, root_digest: _Optional[bytes] = ..., receipt: _Optional[bytes] = ...) -> None: ...

class RecordInstallEvidenceResponse(_message.Message):
    __slots__ = ("task", "changed")
    TASK_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    task: int
    changed: bool
    def __init__(self, task: _Optional[int] = ..., changed: _Optional[bool] = ...) -> None: ...

class RecordMigrationAbortRequest(_message.Message):
    __slots__ = ("root_digest", "operation_id")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    operation_id: bytes
    def __init__(self, root_digest: _Optional[bytes] = ..., operation_id: _Optional[bytes] = ...) -> None: ...

class RecordMigrationAbortResponse(_message.Message):
    __slots__ = ("task", "changed")
    TASK_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    task: int
    changed: bool
    def __init__(self, task: _Optional[int] = ..., changed: _Optional[bool] = ...) -> None: ...

class DetachAbortedLearnerRequest(_message.Message):
    __slots__ = ("root_digest", "operation_id")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    operation_id: bytes
    def __init__(self, root_digest: _Optional[bytes] = ..., operation_id: _Optional[bytes] = ...) -> None: ...

class DetachAbortedLearnerResponse(_message.Message):
    __slots__ = ("detached_node", "changed", "voters")
    DETACHED_NODE_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    VOTERS_FIELD_NUMBER: _ClassVar[int]
    detached_node: int
    changed: bool
    voters: _containers.RepeatedScalarFieldContainer[int]
    def __init__(self, detached_node: _Optional[int] = ..., changed: _Optional[bool] = ..., voters: _Optional[_Iterable[int]] = ...) -> None: ...

class RecordGroupCompactionRequest(_message.Message):
    __slots__ = ("root_digest", "region_id", "floor_term", "floor_index")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    REGION_ID_FIELD_NUMBER: _ClassVar[int]
    FLOOR_TERM_FIELD_NUMBER: _ClassVar[int]
    FLOOR_INDEX_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    region_id: int
    floor_term: int
    floor_index: int
    def __init__(self, root_digest: _Optional[bytes] = ..., region_id: _Optional[int] = ..., floor_term: _Optional[int] = ..., floor_index: _Optional[int] = ...) -> None: ...

class RecordGroupCompactionResponse(_message.Message):
    __slots__ = ("task", "changed")
    TASK_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    task: int
    changed: bool
    def __init__(self, task: _Optional[int] = ..., changed: _Optional[bool] = ...) -> None: ...

class RecordSourceTruncationRequest(_message.Message):
    __slots__ = ("root_digest", "operation_id", "floor_term", "floor_index")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    FLOOR_TERM_FIELD_NUMBER: _ClassVar[int]
    FLOOR_INDEX_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    operation_id: bytes
    floor_term: int
    floor_index: int
    def __init__(self, root_digest: _Optional[bytes] = ..., operation_id: _Optional[bytes] = ..., floor_term: _Optional[int] = ..., floor_index: _Optional[int] = ...) -> None: ...

class RecordSourceTruncationResponse(_message.Message):
    __slots__ = ("task", "changed")
    TASK_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    task: int
    changed: bool
    def __init__(self, task: _Optional[int] = ..., changed: _Optional[bool] = ...) -> None: ...

class TruncateSourceLogRequest(_message.Message):
    __slots__ = ("root_digest", "operation_id")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    operation_id: bytes
    def __init__(self, root_digest: _Optional[bytes] = ..., operation_id: _Optional[bytes] = ...) -> None: ...

class TruncateSourceLogResponse(_message.Message):
    __slots__ = ("floor_term", "floor_index", "first_index")
    FLOOR_TERM_FIELD_NUMBER: _ClassVar[int]
    FLOOR_INDEX_FIELD_NUMBER: _ClassVar[int]
    FIRST_INDEX_FIELD_NUMBER: _ClassVar[int]
    floor_term: int
    floor_index: int
    first_index: int
    def __init__(self, floor_term: _Optional[int] = ..., floor_index: _Optional[int] = ..., first_index: _Optional[int] = ...) -> None: ...

class PromoteMigrationVoterRequest(_message.Message):
    __slots__ = ("root_digest", "operation_id")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    operation_id: bytes
    def __init__(self, root_digest: _Optional[bytes] = ..., operation_id: _Optional[bytes] = ...) -> None: ...

class PromoteMigrationVoterResponse(_message.Message):
    __slots__ = ("destination_node", "changed", "voters")
    DESTINATION_NODE_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    VOTERS_FIELD_NUMBER: _ClassVar[int]
    destination_node: int
    changed: bool
    voters: _containers.RepeatedScalarFieldContainer[int]
    def __init__(self, destination_node: _Optional[int] = ..., changed: _Optional[bool] = ..., voters: _Optional[_Iterable[int]] = ...) -> None: ...

class RecordSourceRemovalRequest(_message.Message):
    __slots__ = ("root_digest", "operation_id", "source_node", "source_incarnation")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    SOURCE_NODE_FIELD_NUMBER: _ClassVar[int]
    SOURCE_INCARNATION_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    operation_id: bytes
    source_node: int
    source_incarnation: bytes
    def __init__(self, root_digest: _Optional[bytes] = ..., operation_id: _Optional[bytes] = ..., source_node: _Optional[int] = ..., source_incarnation: _Optional[bytes] = ...) -> None: ...

class RecordSourceRemovalResponse(_message.Message):
    __slots__ = ("task", "changed")
    TASK_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    task: int
    changed: bool
    def __init__(self, task: _Optional[int] = ..., changed: _Optional[bool] = ...) -> None: ...

class RemoveSourceReplicaRequest(_message.Message):
    __slots__ = ("root_digest", "operation_id")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    operation_id: bytes
    def __init__(self, root_digest: _Optional[bytes] = ..., operation_id: _Optional[bytes] = ...) -> None: ...

class RemoveSourceReplicaResponse(_message.Message):
    __slots__ = ("removed_node", "changed", "voters")
    REMOVED_NODE_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    VOTERS_FIELD_NUMBER: _ClassVar[int]
    removed_node: int
    changed: bool
    voters: _containers.RepeatedScalarFieldContainer[int]
    def __init__(self, removed_node: _Optional[int] = ..., changed: _Optional[bool] = ..., voters: _Optional[_Iterable[int]] = ...) -> None: ...

class BindMigrationImageResponse(_message.Message):
    __slots__ = ("source_owner", "destination_owner")
    SOURCE_OWNER_FIELD_NUMBER: _ClassVar[int]
    DESTINATION_OWNER_FIELD_NUMBER: _ClassVar[int]
    source_owner: bytes
    destination_owner: bytes
    def __init__(self, source_owner: _Optional[bytes] = ..., destination_owner: _Optional[bytes] = ...) -> None: ...

class ApplyRetentionRequest(_message.Message):
    __slots__ = ("command",)
    COMMAND_FIELD_NUMBER: _ClassVar[int]
    command: bytes
    def __init__(self, command: _Optional[bytes] = ...) -> None: ...

class ApplyRetentionResponse(_message.Message):
    __slots__ = ("revision", "changed", "applied_term", "applied_index")
    REVISION_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    APPLIED_TERM_FIELD_NUMBER: _ClassVar[int]
    APPLIED_INDEX_FIELD_NUMBER: _ClassVar[int]
    revision: int
    changed: bool
    applied_term: int
    applied_index: int
    def __init__(self, revision: _Optional[int] = ..., changed: _Optional[bool] = ..., applied_term: _Optional[int] = ..., applied_index: _Optional[int] = ...) -> None: ...

class GetRetentionOwnerRequest(_message.Message):
    __slots__ = ("root_digest", "owner_id")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OWNER_ID_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    owner_id: bytes
    def __init__(self, root_digest: _Optional[bytes] = ..., owner_id: _Optional[bytes] = ...) -> None: ...

class GetRetentionOwnerResponse(_message.Message):
    __slots__ = ("found", "owner")
    FOUND_FIELD_NUMBER: _ClassVar[int]
    OWNER_FIELD_NUMBER: _ClassVar[int]
    found: bool
    owner: bytes
    def __init__(self, found: _Optional[bool] = ..., owner: _Optional[bytes] = ...) -> None: ...

class NodeEndpoint(_message.Message):
    __slots__ = ("node_id", "store_incarnation", "address", "generation", "previous_address", "active")
    NODE_ID_FIELD_NUMBER: _ClassVar[int]
    STORE_INCARNATION_FIELD_NUMBER: _ClassVar[int]
    ADDRESS_FIELD_NUMBER: _ClassVar[int]
    GENERATION_FIELD_NUMBER: _ClassVar[int]
    PREVIOUS_ADDRESS_FIELD_NUMBER: _ClassVar[int]
    ACTIVE_FIELD_NUMBER: _ClassVar[int]
    node_id: int
    store_incarnation: bytes
    address: str
    generation: int
    previous_address: str
    active: bool
    def __init__(self, node_id: _Optional[int] = ..., store_incarnation: _Optional[bytes] = ..., address: _Optional[str] = ..., generation: _Optional[int] = ..., previous_address: _Optional[str] = ..., active: _Optional[bool] = ...) -> None: ...

class GetNodeEndpointRequest(_message.Message):
    __slots__ = ("node_id",)
    NODE_ID_FIELD_NUMBER: _ClassVar[int]
    node_id: int
    def __init__(self, node_id: _Optional[int] = ...) -> None: ...

class NodeStatusRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class StatusField(_message.Message):
    __slots__ = ("name", "value")
    NAME_FIELD_NUMBER: _ClassVar[int]
    VALUE_FIELD_NUMBER: _ClassVar[int]
    name: str
    value: str
    def __init__(self, name: _Optional[str] = ..., value: _Optional[str] = ...) -> None: ...

class StatusPeer(_message.Message):
    __slots__ = ("node_id", "address")
    NODE_ID_FIELD_NUMBER: _ClassVar[int]
    ADDRESS_FIELD_NUMBER: _ClassVar[int]
    node_id: int
    address: str
    def __init__(self, node_id: _Optional[int] = ..., address: _Optional[str] = ...) -> None: ...

class NodeStatusResponse(_message.Message):
    __slots__ = ("fields", "peers")
    FIELDS_FIELD_NUMBER: _ClassVar[int]
    PEERS_FIELD_NUMBER: _ClassVar[int]
    fields: _containers.RepeatedCompositeFieldContainer[StatusField]
    peers: _containers.RepeatedCompositeFieldContainer[StatusPeer]
    def __init__(self, fields: _Optional[_Iterable[_Union[StatusField, _Mapping]]] = ..., peers: _Optional[_Iterable[_Union[StatusPeer, _Mapping]]] = ...) -> None: ...

class ListNodesRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class NodeRow(_message.Message):
    __slots__ = ("node_id", "address", "store_incarnation", "endpoint_generation", "active", "last_heartbeat_unix", "raft_role", "matched_index", "recent_active", "admission_state", "admission_expires_unix", "draining", "drained")
    NODE_ID_FIELD_NUMBER: _ClassVar[int]
    ADDRESS_FIELD_NUMBER: _ClassVar[int]
    STORE_INCARNATION_FIELD_NUMBER: _ClassVar[int]
    ENDPOINT_GENERATION_FIELD_NUMBER: _ClassVar[int]
    ACTIVE_FIELD_NUMBER: _ClassVar[int]
    LAST_HEARTBEAT_UNIX_FIELD_NUMBER: _ClassVar[int]
    RAFT_ROLE_FIELD_NUMBER: _ClassVar[int]
    MATCHED_INDEX_FIELD_NUMBER: _ClassVar[int]
    RECENT_ACTIVE_FIELD_NUMBER: _ClassVar[int]
    ADMISSION_STATE_FIELD_NUMBER: _ClassVar[int]
    ADMISSION_EXPIRES_UNIX_FIELD_NUMBER: _ClassVar[int]
    DRAINING_FIELD_NUMBER: _ClassVar[int]
    DRAINED_FIELD_NUMBER: _ClassVar[int]
    node_id: int
    address: str
    store_incarnation: bytes
    endpoint_generation: int
    active: bool
    last_heartbeat_unix: int
    raft_role: str
    matched_index: int
    recent_active: bool
    admission_state: str
    admission_expires_unix: int
    draining: bool
    drained: bool
    def __init__(self, node_id: _Optional[int] = ..., address: _Optional[str] = ..., store_incarnation: _Optional[bytes] = ..., endpoint_generation: _Optional[int] = ..., active: _Optional[bool] = ..., last_heartbeat_unix: _Optional[int] = ..., raft_role: _Optional[str] = ..., matched_index: _Optional[int] = ..., recent_active: _Optional[bool] = ..., admission_state: _Optional[str] = ..., admission_expires_unix: _Optional[int] = ..., draining: _Optional[bool] = ..., drained: _Optional[bool] = ...) -> None: ...

class ListNodesResponse(_message.Message):
    __slots__ = ("leader_node_id", "raft_committed", "nodes")
    LEADER_NODE_ID_FIELD_NUMBER: _ClassVar[int]
    RAFT_COMMITTED_FIELD_NUMBER: _ClassVar[int]
    NODES_FIELD_NUMBER: _ClassVar[int]
    leader_node_id: int
    raft_committed: int
    nodes: _containers.RepeatedCompositeFieldContainer[NodeRow]
    def __init__(self, leader_node_id: _Optional[int] = ..., raft_committed: _Optional[int] = ..., nodes: _Optional[_Iterable[_Union[NodeRow, _Mapping]]] = ...) -> None: ...

class ListDataGroupsRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class DataGroupReplica(_message.Message):
    __slots__ = ("node_id", "store_incarnation")
    NODE_ID_FIELD_NUMBER: _ClassVar[int]
    STORE_INCARNATION_FIELD_NUMBER: _ClassVar[int]
    node_id: int
    store_incarnation: bytes
    def __init__(self, node_id: _Optional[int] = ..., store_incarnation: _Optional[bytes] = ...) -> None: ...

class DataGroupKeyspace(_message.Message):
    __slots__ = ("keyspace_id", "tenant_id", "bind_task", "conf_ver", "version", "start", "end", "sealed")
    KEYSPACE_ID_FIELD_NUMBER: _ClassVar[int]
    TENANT_ID_FIELD_NUMBER: _ClassVar[int]
    BIND_TASK_FIELD_NUMBER: _ClassVar[int]
    CONF_VER_FIELD_NUMBER: _ClassVar[int]
    VERSION_FIELD_NUMBER: _ClassVar[int]
    START_FIELD_NUMBER: _ClassVar[int]
    END_FIELD_NUMBER: _ClassVar[int]
    SEALED_FIELD_NUMBER: _ClassVar[int]
    keyspace_id: int
    tenant_id: int
    bind_task: int
    conf_ver: int
    version: int
    start: bytes
    end: bytes
    sealed: bool
    def __init__(self, keyspace_id: _Optional[int] = ..., tenant_id: _Optional[int] = ..., bind_task: _Optional[int] = ..., conf_ver: _Optional[int] = ..., version: _Optional[int] = ..., start: _Optional[bytes] = ..., end: _Optional[bytes] = ..., sealed: _Optional[bool] = ...) -> None: ...

class DataGroupMigration(_message.Message):
    __slots__ = ("task", "operation", "destination", "evidence_task", "abort_task", "removal_task", "truncation_task")
    TASK_FIELD_NUMBER: _ClassVar[int]
    OPERATION_FIELD_NUMBER: _ClassVar[int]
    DESTINATION_FIELD_NUMBER: _ClassVar[int]
    EVIDENCE_TASK_FIELD_NUMBER: _ClassVar[int]
    ABORT_TASK_FIELD_NUMBER: _ClassVar[int]
    REMOVAL_TASK_FIELD_NUMBER: _ClassVar[int]
    TRUNCATION_TASK_FIELD_NUMBER: _ClassVar[int]
    task: int
    operation: bytes
    destination: DataGroupReplica
    evidence_task: int
    abort_task: int
    removal_task: int
    truncation_task: int
    def __init__(self, task: _Optional[int] = ..., operation: _Optional[bytes] = ..., destination: _Optional[_Union[DataGroupReplica, _Mapping]] = ..., evidence_task: _Optional[int] = ..., abort_task: _Optional[int] = ..., removal_task: _Optional[int] = ..., truncation_task: _Optional[int] = ...) -> None: ...

class DataGroupPlan(_message.Message):
    __slots__ = ("task", "operation", "migration_task", "mode", "destination", "source", "stage")
    TASK_FIELD_NUMBER: _ClassVar[int]
    OPERATION_FIELD_NUMBER: _ClassVar[int]
    MIGRATION_TASK_FIELD_NUMBER: _ClassVar[int]
    MODE_FIELD_NUMBER: _ClassVar[int]
    DESTINATION_FIELD_NUMBER: _ClassVar[int]
    SOURCE_FIELD_NUMBER: _ClassVar[int]
    STAGE_FIELD_NUMBER: _ClassVar[int]
    task: int
    operation: bytes
    migration_task: int
    mode: str
    destination: DataGroupReplica
    source: DataGroupReplica
    stage: str
    def __init__(self, task: _Optional[int] = ..., operation: _Optional[bytes] = ..., migration_task: _Optional[int] = ..., mode: _Optional[str] = ..., destination: _Optional[_Union[DataGroupReplica, _Mapping]] = ..., source: _Optional[_Union[DataGroupReplica, _Mapping]] = ..., stage: _Optional[str] = ...) -> None: ...

class DataGroupRow(_message.Message):
    __slots__ = ("creation_task", "region_id", "operation", "replicas", "activated", "keyspaces", "migrations", "compaction_floor_term", "compaction_floor_index", "plans", "members", "splits", "merges")
    CREATION_TASK_FIELD_NUMBER: _ClassVar[int]
    REGION_ID_FIELD_NUMBER: _ClassVar[int]
    OPERATION_FIELD_NUMBER: _ClassVar[int]
    REPLICAS_FIELD_NUMBER: _ClassVar[int]
    ACTIVATED_FIELD_NUMBER: _ClassVar[int]
    KEYSPACES_FIELD_NUMBER: _ClassVar[int]
    MIGRATIONS_FIELD_NUMBER: _ClassVar[int]
    COMPACTION_FLOOR_TERM_FIELD_NUMBER: _ClassVar[int]
    COMPACTION_FLOOR_INDEX_FIELD_NUMBER: _ClassVar[int]
    PLANS_FIELD_NUMBER: _ClassVar[int]
    MEMBERS_FIELD_NUMBER: _ClassVar[int]
    SPLITS_FIELD_NUMBER: _ClassVar[int]
    MERGES_FIELD_NUMBER: _ClassVar[int]
    creation_task: int
    region_id: int
    operation: bytes
    replicas: _containers.RepeatedCompositeFieldContainer[DataGroupReplica]
    activated: bool
    keyspaces: _containers.RepeatedCompositeFieldContainer[DataGroupKeyspace]
    migrations: _containers.RepeatedCompositeFieldContainer[DataGroupMigration]
    compaction_floor_term: int
    compaction_floor_index: int
    plans: _containers.RepeatedCompositeFieldContainer[DataGroupPlan]
    members: _containers.RepeatedCompositeFieldContainer[DataGroupReplica]
    splits: _containers.RepeatedCompositeFieldContainer[DataGroupSplit]
    merges: _containers.RepeatedCompositeFieldContainer[DataGroupMerge]
    def __init__(self, creation_task: _Optional[int] = ..., region_id: _Optional[int] = ..., operation: _Optional[bytes] = ..., replicas: _Optional[_Iterable[_Union[DataGroupReplica, _Mapping]]] = ..., activated: _Optional[bool] = ..., keyspaces: _Optional[_Iterable[_Union[DataGroupKeyspace, _Mapping]]] = ..., migrations: _Optional[_Iterable[_Union[DataGroupMigration, _Mapping]]] = ..., compaction_floor_term: _Optional[int] = ..., compaction_floor_index: _Optional[int] = ..., plans: _Optional[_Iterable[_Union[DataGroupPlan, _Mapping]]] = ..., members: _Optional[_Iterable[_Union[DataGroupReplica, _Mapping]]] = ..., splits: _Optional[_Iterable[_Union[DataGroupSplit, _Mapping]]] = ..., merges: _Optional[_Iterable[_Union[DataGroupMerge, _Mapping]]] = ...) -> None: ...

class AddReplicaRequest(_message.Message):
    __slots__ = ("root_digest", "operation_id", "creation_task", "destination_node", "mode", "source_node")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    CREATION_TASK_FIELD_NUMBER: _ClassVar[int]
    DESTINATION_NODE_FIELD_NUMBER: _ClassVar[int]
    MODE_FIELD_NUMBER: _ClassVar[int]
    SOURCE_NODE_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    operation_id: bytes
    creation_task: int
    destination_node: int
    mode: str
    source_node: int
    def __init__(self, root_digest: _Optional[bytes] = ..., operation_id: _Optional[bytes] = ..., creation_task: _Optional[int] = ..., destination_node: _Optional[int] = ..., mode: _Optional[str] = ..., source_node: _Optional[int] = ...) -> None: ...

class SplitRangeRequest(_message.Message):
    __slots__ = ("root_digest", "operation_id", "parent_region", "split_key")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    PARENT_REGION_FIELD_NUMBER: _ClassVar[int]
    SPLIT_KEY_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    operation_id: bytes
    parent_region: int
    split_key: bytes
    def __init__(self, root_digest: _Optional[bytes] = ..., operation_id: _Optional[bytes] = ..., parent_region: _Optional[int] = ..., split_key: _Optional[bytes] = ...) -> None: ...

class SplitRangeResponse(_message.Message):
    __slots__ = ("intent_task", "child_low_task", "child_high_task", "child_low_region", "child_high_region", "changed", "applied_term", "applied_index")
    INTENT_TASK_FIELD_NUMBER: _ClassVar[int]
    CHILD_LOW_TASK_FIELD_NUMBER: _ClassVar[int]
    CHILD_HIGH_TASK_FIELD_NUMBER: _ClassVar[int]
    CHILD_LOW_REGION_FIELD_NUMBER: _ClassVar[int]
    CHILD_HIGH_REGION_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    APPLIED_TERM_FIELD_NUMBER: _ClassVar[int]
    APPLIED_INDEX_FIELD_NUMBER: _ClassVar[int]
    intent_task: int
    child_low_task: int
    child_high_task: int
    child_low_region: int
    child_high_region: int
    changed: bool
    applied_term: int
    applied_index: int
    def __init__(self, intent_task: _Optional[int] = ..., child_low_task: _Optional[int] = ..., child_high_task: _Optional[int] = ..., child_low_region: _Optional[int] = ..., child_high_region: _Optional[int] = ..., changed: _Optional[bool] = ..., applied_term: _Optional[int] = ..., applied_index: _Optional[int] = ...) -> None: ...

class AbortTaskRequest(_message.Message):
    __slots__ = ("task",)
    TASK_FIELD_NUMBER: _ClassVar[int]
    task: int
    def __init__(self, task: _Optional[int] = ...) -> None: ...

class AbortTaskResponse(_message.Message):
    __slots__ = ("target", "target_task", "operation", "abort_task", "changed", "applied_term", "applied_index", "path")
    TARGET_FIELD_NUMBER: _ClassVar[int]
    TARGET_TASK_FIELD_NUMBER: _ClassVar[int]
    OPERATION_FIELD_NUMBER: _ClassVar[int]
    ABORT_TASK_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    APPLIED_TERM_FIELD_NUMBER: _ClassVar[int]
    APPLIED_INDEX_FIELD_NUMBER: _ClassVar[int]
    PATH_FIELD_NUMBER: _ClassVar[int]
    target: str
    target_task: int
    operation: bytes
    abort_task: int
    changed: bool
    applied_term: int
    applied_index: int
    path: str
    def __init__(self, target: _Optional[str] = ..., target_task: _Optional[int] = ..., operation: _Optional[bytes] = ..., abort_task: _Optional[int] = ..., changed: _Optional[bool] = ..., applied_term: _Optional[int] = ..., applied_index: _Optional[int] = ..., path: _Optional[str] = ...) -> None: ...

class MergeRangesRequest(_message.Message):
    __slots__ = ("root_digest", "operation_id", "low_region", "high_region")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OPERATION_ID_FIELD_NUMBER: _ClassVar[int]
    LOW_REGION_FIELD_NUMBER: _ClassVar[int]
    HIGH_REGION_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    operation_id: bytes
    low_region: int
    high_region: int
    def __init__(self, root_digest: _Optional[bytes] = ..., operation_id: _Optional[bytes] = ..., low_region: _Optional[int] = ..., high_region: _Optional[int] = ...) -> None: ...

class MergeRangesResponse(_message.Message):
    __slots__ = ("intent_task", "child_task", "child_region", "changed", "applied_term", "applied_index")
    INTENT_TASK_FIELD_NUMBER: _ClassVar[int]
    CHILD_TASK_FIELD_NUMBER: _ClassVar[int]
    CHILD_REGION_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    APPLIED_TERM_FIELD_NUMBER: _ClassVar[int]
    APPLIED_INDEX_FIELD_NUMBER: _ClassVar[int]
    intent_task: int
    child_task: int
    child_region: int
    changed: bool
    applied_term: int
    applied_index: int
    def __init__(self, intent_task: _Optional[int] = ..., child_task: _Optional[int] = ..., child_region: _Optional[int] = ..., changed: _Optional[bool] = ..., applied_term: _Optional[int] = ..., applied_index: _Optional[int] = ...) -> None: ...

class DataGroupMerge(_message.Message):
    __slots__ = ("task", "operation", "low_region", "high_region", "child_task", "child_region", "stage", "evidence_task", "evidence_digest", "evidence_rows")
    TASK_FIELD_NUMBER: _ClassVar[int]
    OPERATION_FIELD_NUMBER: _ClassVar[int]
    LOW_REGION_FIELD_NUMBER: _ClassVar[int]
    HIGH_REGION_FIELD_NUMBER: _ClassVar[int]
    CHILD_TASK_FIELD_NUMBER: _ClassVar[int]
    CHILD_REGION_FIELD_NUMBER: _ClassVar[int]
    STAGE_FIELD_NUMBER: _ClassVar[int]
    EVIDENCE_TASK_FIELD_NUMBER: _ClassVar[int]
    EVIDENCE_DIGEST_FIELD_NUMBER: _ClassVar[int]
    EVIDENCE_ROWS_FIELD_NUMBER: _ClassVar[int]
    task: int
    operation: bytes
    low_region: int
    high_region: int
    child_task: int
    child_region: int
    stage: str
    evidence_task: int
    evidence_digest: bytes
    evidence_rows: int
    def __init__(self, task: _Optional[int] = ..., operation: _Optional[bytes] = ..., low_region: _Optional[int] = ..., high_region: _Optional[int] = ..., child_task: _Optional[int] = ..., child_region: _Optional[int] = ..., stage: _Optional[str] = ..., evidence_task: _Optional[int] = ..., evidence_digest: _Optional[bytes] = ..., evidence_rows: _Optional[int] = ...) -> None: ...

class DataGroupSplit(_message.Message):
    __slots__ = ("task", "operation", "split_key", "child_low_task", "child_high_task", "child_low_region", "child_high_region", "stage", "evidence")
    TASK_FIELD_NUMBER: _ClassVar[int]
    OPERATION_FIELD_NUMBER: _ClassVar[int]
    SPLIT_KEY_FIELD_NUMBER: _ClassVar[int]
    CHILD_LOW_TASK_FIELD_NUMBER: _ClassVar[int]
    CHILD_HIGH_TASK_FIELD_NUMBER: _ClassVar[int]
    CHILD_LOW_REGION_FIELD_NUMBER: _ClassVar[int]
    CHILD_HIGH_REGION_FIELD_NUMBER: _ClassVar[int]
    STAGE_FIELD_NUMBER: _ClassVar[int]
    EVIDENCE_FIELD_NUMBER: _ClassVar[int]
    task: int
    operation: bytes
    split_key: bytes
    child_low_task: int
    child_high_task: int
    child_low_region: int
    child_high_region: int
    stage: str
    evidence: _containers.RepeatedCompositeFieldContainer[DataGroupSplitEvidence]
    def __init__(self, task: _Optional[int] = ..., operation: _Optional[bytes] = ..., split_key: _Optional[bytes] = ..., child_low_task: _Optional[int] = ..., child_high_task: _Optional[int] = ..., child_low_region: _Optional[int] = ..., child_high_region: _Optional[int] = ..., stage: _Optional[str] = ..., evidence: _Optional[_Iterable[_Union[DataGroupSplitEvidence, _Mapping]]] = ...) -> None: ...

class DataGroupSplitEvidence(_message.Message):
    __slots__ = ("task", "child_task", "digest", "rows")
    TASK_FIELD_NUMBER: _ClassVar[int]
    CHILD_TASK_FIELD_NUMBER: _ClassVar[int]
    DIGEST_FIELD_NUMBER: _ClassVar[int]
    ROWS_FIELD_NUMBER: _ClassVar[int]
    task: int
    child_task: int
    digest: bytes
    rows: int
    def __init__(self, task: _Optional[int] = ..., child_task: _Optional[int] = ..., digest: _Optional[bytes] = ..., rows: _Optional[int] = ...) -> None: ...

class BackupKeyspaceRequest(_message.Message):
    __slots__ = ("keyspace_id", "label")
    KEYSPACE_ID_FIELD_NUMBER: _ClassVar[int]
    LABEL_FIELD_NUMBER: _ClassVar[int]
    keyspace_id: int
    label: str
    def __init__(self, keyspace_id: _Optional[int] = ..., label: _Optional[str] = ...) -> None: ...

class BackupKeyspaceResponse(_message.Message):
    __slots__ = ("task", "operation", "changed", "applied_term", "applied_index", "sequence", "regions")
    TASK_FIELD_NUMBER: _ClassVar[int]
    OPERATION_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    APPLIED_TERM_FIELD_NUMBER: _ClassVar[int]
    APPLIED_INDEX_FIELD_NUMBER: _ClassVar[int]
    SEQUENCE_FIELD_NUMBER: _ClassVar[int]
    REGIONS_FIELD_NUMBER: _ClassVar[int]
    task: int
    operation: bytes
    changed: bool
    applied_term: int
    applied_index: int
    sequence: int
    regions: _containers.RepeatedScalarFieldContainer[int]
    def __init__(self, task: _Optional[int] = ..., operation: _Optional[bytes] = ..., changed: _Optional[bool] = ..., applied_term: _Optional[int] = ..., applied_index: _Optional[int] = ..., sequence: _Optional[int] = ..., regions: _Optional[_Iterable[int]] = ...) -> None: ...

class ListBackupsRequest(_message.Message):
    __slots__ = ("include_retired",)
    INCLUDE_RETIRED_FIELD_NUMBER: _ClassVar[int]
    include_retired: bool
    def __init__(self, include_retired: _Optional[bool] = ...) -> None: ...

class BackupAnchorRow(_message.Message):
    __slots__ = ("task", "cut_term", "cut_index", "image_digest", "objects", "object_bytes", "rows", "rows_digest", "restorable")
    TASK_FIELD_NUMBER: _ClassVar[int]
    CUT_TERM_FIELD_NUMBER: _ClassVar[int]
    CUT_INDEX_FIELD_NUMBER: _ClassVar[int]
    IMAGE_DIGEST_FIELD_NUMBER: _ClassVar[int]
    OBJECTS_FIELD_NUMBER: _ClassVar[int]
    OBJECT_BYTES_FIELD_NUMBER: _ClassVar[int]
    ROWS_FIELD_NUMBER: _ClassVar[int]
    ROWS_DIGEST_FIELD_NUMBER: _ClassVar[int]
    RESTORABLE_FIELD_NUMBER: _ClassVar[int]
    task: int
    cut_term: int
    cut_index: int
    image_digest: bytes
    objects: int
    object_bytes: int
    rows: int
    rows_digest: bytes
    restorable: bool
    def __init__(self, task: _Optional[int] = ..., cut_term: _Optional[int] = ..., cut_index: _Optional[int] = ..., image_digest: _Optional[bytes] = ..., objects: _Optional[int] = ..., object_bytes: _Optional[int] = ..., rows: _Optional[int] = ..., rows_digest: _Optional[bytes] = ..., restorable: _Optional[bool] = ...) -> None: ...

class BackupRangeRow(_message.Message):
    __slots__ = ("region", "creation_task", "bind_task", "operation", "anchor")
    REGION_FIELD_NUMBER: _ClassVar[int]
    CREATION_TASK_FIELD_NUMBER: _ClassVar[int]
    BIND_TASK_FIELD_NUMBER: _ClassVar[int]
    OPERATION_FIELD_NUMBER: _ClassVar[int]
    ANCHOR_FIELD_NUMBER: _ClassVar[int]
    region: int
    creation_task: int
    bind_task: int
    operation: bytes
    anchor: BackupAnchorRow
    def __init__(self, region: _Optional[int] = ..., creation_task: _Optional[int] = ..., bind_task: _Optional[int] = ..., operation: _Optional[bytes] = ..., anchor: _Optional[_Union[BackupAnchorRow, _Mapping]] = ...) -> None: ...

class BackupRow(_message.Message):
    __slots__ = ("task", "operation", "keyspace_id", "tenant_id", "label", "sequence", "requested_unix", "stage", "ranges", "bundle_key", "bundle_digest", "bundle_bytes", "completed_unix", "retired_unix", "restorable")
    TASK_FIELD_NUMBER: _ClassVar[int]
    OPERATION_FIELD_NUMBER: _ClassVar[int]
    KEYSPACE_ID_FIELD_NUMBER: _ClassVar[int]
    TENANT_ID_FIELD_NUMBER: _ClassVar[int]
    LABEL_FIELD_NUMBER: _ClassVar[int]
    SEQUENCE_FIELD_NUMBER: _ClassVar[int]
    REQUESTED_UNIX_FIELD_NUMBER: _ClassVar[int]
    STAGE_FIELD_NUMBER: _ClassVar[int]
    RANGES_FIELD_NUMBER: _ClassVar[int]
    BUNDLE_KEY_FIELD_NUMBER: _ClassVar[int]
    BUNDLE_DIGEST_FIELD_NUMBER: _ClassVar[int]
    BUNDLE_BYTES_FIELD_NUMBER: _ClassVar[int]
    COMPLETED_UNIX_FIELD_NUMBER: _ClassVar[int]
    RETIRED_UNIX_FIELD_NUMBER: _ClassVar[int]
    RESTORABLE_FIELD_NUMBER: _ClassVar[int]
    task: int
    operation: bytes
    keyspace_id: int
    tenant_id: int
    label: str
    sequence: int
    requested_unix: int
    stage: str
    ranges: _containers.RepeatedCompositeFieldContainer[BackupRangeRow]
    bundle_key: str
    bundle_digest: bytes
    bundle_bytes: int
    completed_unix: int
    retired_unix: int
    restorable: bool
    def __init__(self, task: _Optional[int] = ..., operation: _Optional[bytes] = ..., keyspace_id: _Optional[int] = ..., tenant_id: _Optional[int] = ..., label: _Optional[str] = ..., sequence: _Optional[int] = ..., requested_unix: _Optional[int] = ..., stage: _Optional[str] = ..., ranges: _Optional[_Iterable[_Union[BackupRangeRow, _Mapping]]] = ..., bundle_key: _Optional[str] = ..., bundle_digest: _Optional[bytes] = ..., bundle_bytes: _Optional[int] = ..., completed_unix: _Optional[int] = ..., retired_unix: _Optional[int] = ..., restorable: _Optional[bool] = ...) -> None: ...

class ListBackupsResponse(_message.Message):
    __slots__ = ("backups",)
    BACKUPS_FIELD_NUMBER: _ClassVar[int]
    backups: _containers.RepeatedCompositeFieldContainer[BackupRow]
    def __init__(self, backups: _Optional[_Iterable[_Union[BackupRow, _Mapping]]] = ...) -> None: ...

class RetireBackupRequest(_message.Message):
    __slots__ = ("task",)
    TASK_FIELD_NUMBER: _ClassVar[int]
    task: int
    def __init__(self, task: _Optional[int] = ...) -> None: ...

class RetireBackupResponse(_message.Message):
    __slots__ = ("retirement_task", "changed", "applied_term", "applied_index")
    RETIREMENT_TASK_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    APPLIED_TERM_FIELD_NUMBER: _ClassVar[int]
    APPLIED_INDEX_FIELD_NUMBER: _ClassVar[int]
    retirement_task: int
    changed: bool
    applied_term: int
    applied_index: int
    def __init__(self, retirement_task: _Optional[int] = ..., changed: _Optional[bool] = ..., applied_term: _Optional[int] = ..., applied_index: _Optional[int] = ...) -> None: ...

class VerifyBackupRequest(_message.Message):
    __slots__ = ("task",)
    TASK_FIELD_NUMBER: _ClassVar[int]
    task: int
    def __init__(self, task: _Optional[int] = ...) -> None: ...

class BackupRangeVerdict(_message.Message):
    __slots__ = ("region", "ok", "detail")
    REGION_FIELD_NUMBER: _ClassVar[int]
    OK_FIELD_NUMBER: _ClassVar[int]
    DETAIL_FIELD_NUMBER: _ClassVar[int]
    region: int
    ok: bool
    detail: str
    def __init__(self, region: _Optional[int] = ..., ok: _Optional[bool] = ..., detail: _Optional[str] = ...) -> None: ...

class VerifyBackupResponse(_message.Message):
    __slots__ = ("task", "bundle_key", "bundle_ok", "ranges", "ok")
    TASK_FIELD_NUMBER: _ClassVar[int]
    BUNDLE_KEY_FIELD_NUMBER: _ClassVar[int]
    BUNDLE_OK_FIELD_NUMBER: _ClassVar[int]
    RANGES_FIELD_NUMBER: _ClassVar[int]
    OK_FIELD_NUMBER: _ClassVar[int]
    task: int
    bundle_key: str
    bundle_ok: bool
    ranges: _containers.RepeatedCompositeFieldContainer[BackupRangeVerdict]
    ok: bool
    def __init__(self, task: _Optional[int] = ..., bundle_key: _Optional[str] = ..., bundle_ok: _Optional[bool] = ..., ranges: _Optional[_Iterable[_Union[BackupRangeVerdict, _Mapping]]] = ..., ok: _Optional[bool] = ...) -> None: ...

class DrainNodeRequest(_message.Message):
    __slots__ = ("node_id",)
    NODE_ID_FIELD_NUMBER: _ClassVar[int]
    node_id: int
    def __init__(self, node_id: _Optional[int] = ...) -> None: ...

class DrainNodeResponse(_message.Message):
    __slots__ = ("task", "operation", "changed", "applied_term", "applied_index")
    TASK_FIELD_NUMBER: _ClassVar[int]
    OPERATION_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    APPLIED_TERM_FIELD_NUMBER: _ClassVar[int]
    APPLIED_INDEX_FIELD_NUMBER: _ClassVar[int]
    task: int
    operation: bytes
    changed: bool
    applied_term: int
    applied_index: int
    def __init__(self, task: _Optional[int] = ..., operation: _Optional[bytes] = ..., changed: _Optional[bool] = ..., applied_term: _Optional[int] = ..., applied_index: _Optional[int] = ...) -> None: ...

class TransferLeaderRequest(_message.Message):
    __slots__ = ("region_id", "transferee")
    REGION_ID_FIELD_NUMBER: _ClassVar[int]
    TRANSFEREE_FIELD_NUMBER: _ClassVar[int]
    region_id: int
    transferee: int
    def __init__(self, region_id: _Optional[int] = ..., transferee: _Optional[int] = ...) -> None: ...

class TransferLeaderResponse(_message.Message):
    __slots__ = ("previous_leader", "leader", "term", "completed")
    PREVIOUS_LEADER_FIELD_NUMBER: _ClassVar[int]
    LEADER_FIELD_NUMBER: _ClassVar[int]
    TERM_FIELD_NUMBER: _ClassVar[int]
    COMPLETED_FIELD_NUMBER: _ClassVar[int]
    previous_leader: int
    leader: int
    term: int
    completed: bool
    def __init__(self, previous_leader: _Optional[int] = ..., leader: _Optional[int] = ..., term: _Optional[int] = ..., completed: _Optional[bool] = ...) -> None: ...

class AddReplicaResponse(_message.Message):
    __slots__ = ("plan_task", "migration_task", "region_id", "changed", "applied_term", "applied_index")
    PLAN_TASK_FIELD_NUMBER: _ClassVar[int]
    MIGRATION_TASK_FIELD_NUMBER: _ClassVar[int]
    REGION_ID_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    APPLIED_TERM_FIELD_NUMBER: _ClassVar[int]
    APPLIED_INDEX_FIELD_NUMBER: _ClassVar[int]
    plan_task: int
    migration_task: int
    region_id: int
    changed: bool
    applied_term: int
    applied_index: int
    def __init__(self, plan_task: _Optional[int] = ..., migration_task: _Optional[int] = ..., region_id: _Optional[int] = ..., changed: _Optional[bool] = ..., applied_term: _Optional[int] = ..., applied_index: _Optional[int] = ...) -> None: ...

class ListDataGroupsResponse(_message.Message):
    __slots__ = ("groups", "readback_errors")
    GROUPS_FIELD_NUMBER: _ClassVar[int]
    READBACK_ERRORS_FIELD_NUMBER: _ClassVar[int]
    groups: _containers.RepeatedCompositeFieldContainer[DataGroupRow]
    readback_errors: _containers.RepeatedScalarFieldContainer[str]
    def __init__(self, groups: _Optional[_Iterable[_Union[DataGroupRow, _Mapping]]] = ..., readback_errors: _Optional[_Iterable[str]] = ...) -> None: ...

class ListTasksRequest(_message.Message):
    __slots__ = ("after", "limit", "kind")
    AFTER_FIELD_NUMBER: _ClassVar[int]
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    after: int
    limit: int
    kind: int
    def __init__(self, after: _Optional[int] = ..., limit: _Optional[int] = ..., kind: _Optional[int] = ...) -> None: ...

class TaskRow(_message.Message):
    __slots__ = ("id", "kind", "kind_name", "state", "created_unix", "payload_bytes", "operation", "reference_task")
    ID_FIELD_NUMBER: _ClassVar[int]
    KIND_FIELD_NUMBER: _ClassVar[int]
    KIND_NAME_FIELD_NUMBER: _ClassVar[int]
    STATE_FIELD_NUMBER: _ClassVar[int]
    CREATED_UNIX_FIELD_NUMBER: _ClassVar[int]
    PAYLOAD_BYTES_FIELD_NUMBER: _ClassVar[int]
    OPERATION_FIELD_NUMBER: _ClassVar[int]
    REFERENCE_TASK_FIELD_NUMBER: _ClassVar[int]
    id: int
    kind: int
    kind_name: str
    state: int
    created_unix: int
    payload_bytes: int
    operation: bytes
    reference_task: int
    def __init__(self, id: _Optional[int] = ..., kind: _Optional[int] = ..., kind_name: _Optional[str] = ..., state: _Optional[int] = ..., created_unix: _Optional[int] = ..., payload_bytes: _Optional[int] = ..., operation: _Optional[bytes] = ..., reference_task: _Optional[int] = ...) -> None: ...

class ListTasksResponse(_message.Message):
    __slots__ = ("tasks", "next_after")
    TASKS_FIELD_NUMBER: _ClassVar[int]
    NEXT_AFTER_FIELD_NUMBER: _ClassVar[int]
    tasks: _containers.RepeatedCompositeFieldContainer[TaskRow]
    next_after: int
    def __init__(self, tasks: _Optional[_Iterable[_Union[TaskRow, _Mapping]]] = ..., next_after: _Optional[int] = ...) -> None: ...

class GetNodeEndpointResponse(_message.Message):
    __slots__ = ("cluster_id", "endpoint")
    CLUSTER_ID_FIELD_NUMBER: _ClassVar[int]
    ENDPOINT_FIELD_NUMBER: _ClassVar[int]
    cluster_id: bytes
    endpoint: NodeEndpoint
    def __init__(self, cluster_id: _Optional[bytes] = ..., endpoint: _Optional[_Union[NodeEndpoint, _Mapping]] = ...) -> None: ...

class ChangeNodeEndpointRequest(_message.Message):
    __slots__ = ("cluster_id", "node_id", "store_incarnation", "expected_address", "expected_generation", "new_address")
    CLUSTER_ID_FIELD_NUMBER: _ClassVar[int]
    NODE_ID_FIELD_NUMBER: _ClassVar[int]
    STORE_INCARNATION_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_ADDRESS_FIELD_NUMBER: _ClassVar[int]
    EXPECTED_GENERATION_FIELD_NUMBER: _ClassVar[int]
    NEW_ADDRESS_FIELD_NUMBER: _ClassVar[int]
    cluster_id: bytes
    node_id: int
    store_incarnation: bytes
    expected_address: str
    expected_generation: int
    new_address: str
    def __init__(self, cluster_id: _Optional[bytes] = ..., node_id: _Optional[int] = ..., store_incarnation: _Optional[bytes] = ..., expected_address: _Optional[str] = ..., expected_generation: _Optional[int] = ..., new_address: _Optional[str] = ...) -> None: ...

class EndpointReceipt(_message.Message):
    __slots__ = ("endpoint", "applied_term", "applied_index")
    ENDPOINT_FIELD_NUMBER: _ClassVar[int]
    APPLIED_TERM_FIELD_NUMBER: _ClassVar[int]
    APPLIED_INDEX_FIELD_NUMBER: _ClassVar[int]
    endpoint: NodeEndpoint
    applied_term: int
    applied_index: int
    def __init__(self, endpoint: _Optional[_Union[NodeEndpoint, _Mapping]] = ..., applied_term: _Optional[int] = ..., applied_index: _Optional[int] = ...) -> None: ...

class ChangeNodeEndpointResponse(_message.Message):
    __slots__ = ("changed", "confirmed", "refused")
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    CONFIRMED_FIELD_NUMBER: _ClassVar[int]
    REFUSED_FIELD_NUMBER: _ClassVar[int]
    changed: EndpointReceipt
    confirmed: EndpointReceipt
    refused: EndpointRefusal
    def __init__(self, changed: _Optional[_Union[EndpointReceipt, _Mapping]] = ..., confirmed: _Optional[_Union[EndpointReceipt, _Mapping]] = ..., refused: _Optional[_Union[EndpointRefusal, str]] = ...) -> None: ...

class AdmitNodeRequest(_message.Message):
    __slots__ = ("node_id", "addr", "ttl_seconds")
    NODE_ID_FIELD_NUMBER: _ClassVar[int]
    ADDR_FIELD_NUMBER: _ClassVar[int]
    TTL_SECONDS_FIELD_NUMBER: _ClassVar[int]
    node_id: int
    addr: str
    ttl_seconds: int
    def __init__(self, node_id: _Optional[int] = ..., addr: _Optional[str] = ..., ttl_seconds: _Optional[int] = ...) -> None: ...

class PromoteNodeRequest(_message.Message):
    __slots__ = ("node_id",)
    NODE_ID_FIELD_NUMBER: _ClassVar[int]
    node_id: int
    def __init__(self, node_id: _Optional[int] = ...) -> None: ...

class RemoveNodeRequest(_message.Message):
    __slots__ = ("node_id",)
    NODE_ID_FIELD_NUMBER: _ClassVar[int]
    node_id: int
    def __init__(self, node_id: _Optional[int] = ...) -> None: ...

class RevokeAdmissionRequest(_message.Message):
    __slots__ = ("node_id",)
    NODE_ID_FIELD_NUMBER: _ClassVar[int]
    node_id: int
    def __init__(self, node_id: _Optional[int] = ...) -> None: ...

class MembershipChangeResponse(_message.Message):
    __slots__ = ("applied_term", "applied_index", "voters", "learners", "join_ticket", "changed")
    APPLIED_TERM_FIELD_NUMBER: _ClassVar[int]
    APPLIED_INDEX_FIELD_NUMBER: _ClassVar[int]
    VOTERS_FIELD_NUMBER: _ClassVar[int]
    LEARNERS_FIELD_NUMBER: _ClassVar[int]
    JOIN_TICKET_FIELD_NUMBER: _ClassVar[int]
    CHANGED_FIELD_NUMBER: _ClassVar[int]
    applied_term: int
    applied_index: int
    voters: _containers.RepeatedScalarFieldContainer[int]
    learners: _containers.RepeatedScalarFieldContainer[int]
    join_ticket: str
    changed: bool
    def __init__(self, applied_term: _Optional[int] = ..., applied_index: _Optional[int] = ..., voters: _Optional[_Iterable[int]] = ..., learners: _Optional[_Iterable[int]] = ..., join_ticket: _Optional[str] = ..., changed: _Optional[bool] = ...) -> None: ...

class RawWriteResponse(_message.Message):
    __slots__ = ("applied_term", "applied_index")
    APPLIED_TERM_FIELD_NUMBER: _ClassVar[int]
    APPLIED_INDEX_FIELD_NUMBER: _ClassVar[int]
    applied_term: int
    applied_index: int
    def __init__(self, applied_term: _Optional[int] = ..., applied_index: _Optional[int] = ...) -> None: ...

class RawDeleteRangeResponse(_message.Message):
    __slots__ = ("committed_chunks", "last_applied_term", "last_applied_index", "resume_from")
    COMMITTED_CHUNKS_FIELD_NUMBER: _ClassVar[int]
    LAST_APPLIED_TERM_FIELD_NUMBER: _ClassVar[int]
    LAST_APPLIED_INDEX_FIELD_NUMBER: _ClassVar[int]
    RESUME_FROM_FIELD_NUMBER: _ClassVar[int]
    committed_chunks: int
    last_applied_term: int
    last_applied_index: int
    resume_from: bytes
    def __init__(self, committed_chunks: _Optional[int] = ..., last_applied_term: _Optional[int] = ..., last_applied_index: _Optional[int] = ..., resume_from: _Optional[bytes] = ...) -> None: ...

class RegionEpoch(_message.Message):
    __slots__ = ("conf_ver", "version")
    CONF_VER_FIELD_NUMBER: _ClassVar[int]
    VERSION_FIELD_NUMBER: _ClassVar[int]
    conf_ver: int
    version: int
    def __init__(self, conf_ver: _Optional[int] = ..., version: _Optional[int] = ...) -> None: ...

class RequestContext(_message.Message):
    __slots__ = ("keyspace_id", "region_epoch")
    KEYSPACE_ID_FIELD_NUMBER: _ClassVar[int]
    REGION_EPOCH_FIELD_NUMBER: _ClassVar[int]
    keyspace_id: int
    region_epoch: RegionEpoch
    def __init__(self, keyspace_id: _Optional[int] = ..., region_epoch: _Optional[_Union[RegionEpoch, _Mapping]] = ...) -> None: ...

class OptionalValue(_message.Message):
    __slots__ = ("found", "value")
    FOUND_FIELD_NUMBER: _ClassVar[int]
    VALUE_FIELD_NUMBER: _ClassVar[int]
    found: bool
    value: bytes
    def __init__(self, found: _Optional[bool] = ..., value: _Optional[bytes] = ...) -> None: ...

class KeyValue(_message.Message):
    __slots__ = ("key", "value")
    KEY_FIELD_NUMBER: _ClassVar[int]
    VALUE_FIELD_NUMBER: _ClassVar[int]
    key: bytes
    value: bytes
    def __init__(self, key: _Optional[bytes] = ..., value: _Optional[bytes] = ...) -> None: ...

class ScanResponse(_message.Message):
    __slots__ = ("pairs", "resume_from")
    PAIRS_FIELD_NUMBER: _ClassVar[int]
    RESUME_FROM_FIELD_NUMBER: _ClassVar[int]
    pairs: _containers.RepeatedCompositeFieldContainer[KeyValue]
    resume_from: bytes
    def __init__(self, pairs: _Optional[_Iterable[_Union[KeyValue, _Mapping]]] = ..., resume_from: _Optional[bytes] = ...) -> None: ...

class RawGetRequest(_message.Message):
    __slots__ = ("context", "key")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    KEY_FIELD_NUMBER: _ClassVar[int]
    context: RequestContext
    key: bytes
    def __init__(self, context: _Optional[_Union[RequestContext, _Mapping]] = ..., key: _Optional[bytes] = ...) -> None: ...

class RawGetResponse(_message.Message):
    __slots__ = ("value",)
    VALUE_FIELD_NUMBER: _ClassVar[int]
    value: OptionalValue
    def __init__(self, value: _Optional[_Union[OptionalValue, _Mapping]] = ...) -> None: ...

class RawBatchGetRequest(_message.Message):
    __slots__ = ("context", "keys")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    KEYS_FIELD_NUMBER: _ClassVar[int]
    context: RequestContext
    keys: _containers.RepeatedScalarFieldContainer[bytes]
    def __init__(self, context: _Optional[_Union[RequestContext, _Mapping]] = ..., keys: _Optional[_Iterable[bytes]] = ...) -> None: ...

class RawBatchGetResponse(_message.Message):
    __slots__ = ("values",)
    VALUES_FIELD_NUMBER: _ClassVar[int]
    values: _containers.RepeatedCompositeFieldContainer[OptionalValue]
    def __init__(self, values: _Optional[_Iterable[_Union[OptionalValue, _Mapping]]] = ...) -> None: ...

class RawPutRequest(_message.Message):
    __slots__ = ("context", "key", "value")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    KEY_FIELD_NUMBER: _ClassVar[int]
    VALUE_FIELD_NUMBER: _ClassVar[int]
    context: RequestContext
    key: bytes
    value: bytes
    def __init__(self, context: _Optional[_Union[RequestContext, _Mapping]] = ..., key: _Optional[bytes] = ..., value: _Optional[bytes] = ...) -> None: ...

class RawBatchPutRequest(_message.Message):
    __slots__ = ("context", "pairs")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    PAIRS_FIELD_NUMBER: _ClassVar[int]
    context: RequestContext
    pairs: _containers.RepeatedCompositeFieldContainer[KeyValue]
    def __init__(self, context: _Optional[_Union[RequestContext, _Mapping]] = ..., pairs: _Optional[_Iterable[_Union[KeyValue, _Mapping]]] = ...) -> None: ...

class RawMutation(_message.Message):
    __slots__ = ("put", "delete")
    PUT_FIELD_NUMBER: _ClassVar[int]
    DELETE_FIELD_NUMBER: _ClassVar[int]
    put: KeyValue
    delete: bytes
    def __init__(self, put: _Optional[_Union[KeyValue, _Mapping]] = ..., delete: _Optional[bytes] = ...) -> None: ...

class RawBatchRequest(_message.Message):
    __slots__ = ("context", "mutations")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    MUTATIONS_FIELD_NUMBER: _ClassVar[int]
    context: RequestContext
    mutations: _containers.RepeatedCompositeFieldContainer[RawMutation]
    def __init__(self, context: _Optional[_Union[RequestContext, _Mapping]] = ..., mutations: _Optional[_Iterable[_Union[RawMutation, _Mapping]]] = ...) -> None: ...

class RawDeleteRequest(_message.Message):
    __slots__ = ("context", "key")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    KEY_FIELD_NUMBER: _ClassVar[int]
    context: RequestContext
    key: bytes
    def __init__(self, context: _Optional[_Union[RequestContext, _Mapping]] = ..., key: _Optional[bytes] = ...) -> None: ...

class RawScanRequest(_message.Message):
    __slots__ = ("context", "start", "end", "limit")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    START_FIELD_NUMBER: _ClassVar[int]
    END_FIELD_NUMBER: _ClassVar[int]
    LIMIT_FIELD_NUMBER: _ClassVar[int]
    context: RequestContext
    start: bytes
    end: bytes
    limit: int
    def __init__(self, context: _Optional[_Union[RequestContext, _Mapping]] = ..., start: _Optional[bytes] = ..., end: _Optional[bytes] = ..., limit: _Optional[int] = ...) -> None: ...

class RawDeleteRangeRequest(_message.Message):
    __slots__ = ("context", "start", "end")
    CONTEXT_FIELD_NUMBER: _ClassVar[int]
    START_FIELD_NUMBER: _ClassVar[int]
    END_FIELD_NUMBER: _ClassVar[int]
    context: RequestContext
    start: bytes
    end: bytes
    def __init__(self, context: _Optional[_Union[RequestContext, _Mapping]] = ..., start: _Optional[bytes] = ..., end: _Optional[bytes] = ...) -> None: ...

class CreateKeyspaceRequest(_message.Message):
    __slots__ = ("name", "tenant_id", "api_type")
    NAME_FIELD_NUMBER: _ClassVar[int]
    TENANT_ID_FIELD_NUMBER: _ClassVar[int]
    API_TYPE_FIELD_NUMBER: _ClassVar[int]
    name: str
    tenant_id: int
    api_type: ApiType
    def __init__(self, name: _Optional[str] = ..., tenant_id: _Optional[int] = ..., api_type: _Optional[_Union[ApiType, str]] = ...) -> None: ...

class CreateKeyspaceResponse(_message.Message):
    __slots__ = ("keyspace_id", "proposed_term", "proposed_index")
    KEYSPACE_ID_FIELD_NUMBER: _ClassVar[int]
    PROPOSED_TERM_FIELD_NUMBER: _ClassVar[int]
    PROPOSED_INDEX_FIELD_NUMBER: _ClassVar[int]
    keyspace_id: int
    proposed_term: int
    proposed_index: int
    def __init__(self, keyspace_id: _Optional[int] = ..., proposed_term: _Optional[int] = ..., proposed_index: _Optional[int] = ...) -> None: ...

class ListKeyspacesRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class Keyspace(_message.Message):
    __slots__ = ("id", "name", "tenant_id", "api_type")
    ID_FIELD_NUMBER: _ClassVar[int]
    NAME_FIELD_NUMBER: _ClassVar[int]
    TENANT_ID_FIELD_NUMBER: _ClassVar[int]
    API_TYPE_FIELD_NUMBER: _ClassVar[int]
    id: int
    name: str
    tenant_id: int
    api_type: ApiType
    def __init__(self, id: _Optional[int] = ..., name: _Optional[str] = ..., tenant_id: _Optional[int] = ..., api_type: _Optional[_Union[ApiType, str]] = ...) -> None: ...

class ListKeyspacesResponse(_message.Message):
    __slots__ = ("keyspaces",)
    KEYSPACES_FIELD_NUMBER: _ClassVar[int]
    keyspaces: _containers.RepeatedCompositeFieldContainer[Keyspace]
    def __init__(self, keyspaces: _Optional[_Iterable[_Union[Keyspace, _Mapping]]] = ...) -> None: ...

class GetRegionRequest(_message.Message):
    __slots__ = ("keyspace_id", "key")
    KEYSPACE_ID_FIELD_NUMBER: _ClassVar[int]
    KEY_FIELD_NUMBER: _ClassVar[int]
    keyspace_id: int
    key: bytes
    def __init__(self, keyspace_id: _Optional[int] = ..., key: _Optional[bytes] = ...) -> None: ...

class RegionLocation(_message.Message):
    __slots__ = ("region_id", "epoch", "leader_node_id")
    REGION_ID_FIELD_NUMBER: _ClassVar[int]
    EPOCH_FIELD_NUMBER: _ClassVar[int]
    LEADER_NODE_ID_FIELD_NUMBER: _ClassVar[int]
    region_id: int
    epoch: RegionEpoch
    leader_node_id: int
    def __init__(self, region_id: _Optional[int] = ..., epoch: _Optional[_Union[RegionEpoch, _Mapping]] = ..., leader_node_id: _Optional[int] = ...) -> None: ...

class GetRegionResponse(_message.Message):
    __slots__ = ("region",)
    REGION_FIELD_NUMBER: _ClassVar[int]
    region: RegionLocation
    def __init__(self, region: _Optional[_Union[RegionLocation, _Mapping]] = ...) -> None: ...

class ClusterInfoRequest(_message.Message):
    __slots__ = ()
    def __init__(self) -> None: ...

class ClusterInfoResponse(_message.Message):
    __slots__ = ("node_count", "keyspace_count", "region_count")
    NODE_COUNT_FIELD_NUMBER: _ClassVar[int]
    KEYSPACE_COUNT_FIELD_NUMBER: _ClassVar[int]
    REGION_COUNT_FIELD_NUMBER: _ClassVar[int]
    node_count: int
    keyspace_count: int
    region_count: int
    def __init__(self, node_count: _Optional[int] = ..., keyspace_count: _Optional[int] = ..., region_count: _Optional[int] = ...) -> None: ...

class LookupRawRouteRequest(_message.Message):
    __slots__ = ("root_digest", "tenant_id", "keyspace_id", "key")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    TENANT_ID_FIELD_NUMBER: _ClassVar[int]
    KEYSPACE_ID_FIELD_NUMBER: _ClassVar[int]
    KEY_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    tenant_id: int
    keyspace_id: int
    key: bytes
    def __init__(self, root_digest: _Optional[bytes] = ..., tenant_id: _Optional[int] = ..., keyspace_id: _Optional[int] = ..., key: _Optional[bytes] = ...) -> None: ...

class LookupRawRouteResponse(_message.Message):
    __slots__ = ("root_digest", "metadata_peers", "metadata_leader", "binding", "replicas", "data_leader")
    ROOT_DIGEST_FIELD_NUMBER: _ClassVar[int]
    METADATA_PEERS_FIELD_NUMBER: _ClassVar[int]
    METADATA_LEADER_FIELD_NUMBER: _ClassVar[int]
    BINDING_FIELD_NUMBER: _ClassVar[int]
    REPLICAS_FIELD_NUMBER: _ClassVar[int]
    DATA_LEADER_FIELD_NUMBER: _ClassVar[int]
    root_digest: bytes
    metadata_peers: _containers.RepeatedCompositeFieldContainer[NodeEndpoint]
    metadata_leader: int
    binding: bytes
    replicas: _containers.RepeatedCompositeFieldContainer[NodeEndpoint]
    data_leader: int
    def __init__(self, root_digest: _Optional[bytes] = ..., metadata_peers: _Optional[_Iterable[_Union[NodeEndpoint, _Mapping]]] = ..., metadata_leader: _Optional[int] = ..., binding: _Optional[bytes] = ..., replicas: _Optional[_Iterable[_Union[NodeEndpoint, _Mapping]]] = ..., data_leader: _Optional[int] = ...) -> None: ...

class RoutedRawRequest(_message.Message):
    __slots__ = ("binding", "get", "put", "delete_key", "batch")
    BINDING_FIELD_NUMBER: _ClassVar[int]
    GET_FIELD_NUMBER: _ClassVar[int]
    PUT_FIELD_NUMBER: _ClassVar[int]
    DELETE_KEY_FIELD_NUMBER: _ClassVar[int]
    BATCH_FIELD_NUMBER: _ClassVar[int]
    binding: bytes
    get: RoutedKeys
    put: RoutedPairs
    delete_key: bytes
    batch: RoutedMutations
    def __init__(self, binding: _Optional[bytes] = ..., get: _Optional[_Union[RoutedKeys, _Mapping]] = ..., put: _Optional[_Union[RoutedPairs, _Mapping]] = ..., delete_key: _Optional[bytes] = ..., batch: _Optional[_Union[RoutedMutations, _Mapping]] = ...) -> None: ...

class RoutedKeys(_message.Message):
    __slots__ = ("keys",)
    KEYS_FIELD_NUMBER: _ClassVar[int]
    keys: _containers.RepeatedScalarFieldContainer[bytes]
    def __init__(self, keys: _Optional[_Iterable[bytes]] = ...) -> None: ...

class RoutedPairs(_message.Message):
    __slots__ = ("pairs",)
    PAIRS_FIELD_NUMBER: _ClassVar[int]
    pairs: _containers.RepeatedCompositeFieldContainer[KeyValue]
    def __init__(self, pairs: _Optional[_Iterable[_Union[KeyValue, _Mapping]]] = ...) -> None: ...

class RoutedMutations(_message.Message):
    __slots__ = ("mutations",)
    MUTATIONS_FIELD_NUMBER: _ClassVar[int]
    mutations: _containers.RepeatedCompositeFieldContainer[RawMutation]
    def __init__(self, mutations: _Optional[_Iterable[_Union[RawMutation, _Mapping]]] = ...) -> None: ...

class RoutedRawResponse(_message.Message):
    __slots__ = ("binding_digest", "values", "applied")
    BINDING_DIGEST_FIELD_NUMBER: _ClassVar[int]
    VALUES_FIELD_NUMBER: _ClassVar[int]
    APPLIED_FIELD_NUMBER: _ClassVar[int]
    binding_digest: bytes
    values: RawBatchGetResponse
    applied: RawWriteResponse
    def __init__(self, binding_digest: _Optional[bytes] = ..., values: _Optional[_Union[RawBatchGetResponse, _Mapping]] = ..., applied: _Optional[_Union[RawWriteResponse, _Mapping]] = ...) -> None: ...
