import HeapEntryLocations

set_option autoImplicit false

namespace Kv9.Radix

-- Only occupied, nonzero-sized Arc<Node> payload and Box<Branch> object
-- regions appear here. Arc counter/header bytes are outside the Node region;
-- empty Vec buffer sentinels are deliberately not object blocks.
inductive ResidentBlockKind where
  | arcNode
  | branchBox
deriving DecidableEq, Repr

structure ResidentBlockId where
  node : NodeId
  kind : ResidentBlockKind
deriving DecidableEq, Repr

structure ObjectBlock where
  base : Nat
  bytes : Nat
deriving DecidableEq, Repr

def ObjectBlock.Within (block : ObjectBlock) (limit : Nat) : Prop :=
  0 < block.base ∧ 0 < block.bytes ∧ block.base + block.bytes ≤ limit

def ObjectBlock.Disjoint (first second : ObjectBlock) : Prop :=
  first.base + first.bytes ≤ second.base ∨ second.base + second.bytes ≤ first.base

def residentEntryOwner : StoredNode → ResidentBlockKind
  | .leaf _ => .arcNode
  | .branch _ _ _ => .branchBox

-- This image records native object placement independently of graph identity.
-- Relating actual Rust allocations, layout and moves to the image is required
-- at the native boundary; no cardinality bound is assumed in the relation.
structure EntryStorageImage where
  blocks : ResidentBlockId → Option ObjectBlock
  entryOffset : NodeId → Nat
  entryBytes : Nat

structure EntryStorageRep (heap : NodeHeap) (image : EntryStorageImage) (limit : Nat) : Prop where
  nonzeroEntry : 0 < image.entryBytes
  blocksWithin : ∀ owner block, image.blocks owner = some block → block.Within limit
  blocksSeparate : ∀ first second a b, first ≠ second → image.blocks first = some a → image.blocks second = some b → a.Disjoint b
  entryInside : ∀ address node entry, heapRead heap address = some node → storedEntry node = some entry →
    ∃ block, image.blocks ⟨address, residentEntryOwner node⟩ = some block ∧
      image.entryOffset address + image.entryBytes ≤ block.bytes

def nativeEntryAddress (heap : NodeHeap) (image : EntryStorageImage) (address : NodeId) : Option Nat := do
  let node ← heapRead heap address
  let _ ← storedEntry node
  let block ← image.blocks ⟨address, residentEntryOwner node⟩
  some (block.base + image.entryOffset address)

theorem heap_entry_cell (heap : NodeHeap) (address : NodeId) (entry : Entry)
    (read : heapEntry heap address = some entry) :
    ∃ node, heapRead heap address = some node ∧ storedEntry node = some entry := by
  cases cell : heapRead heap address with
  | none => simp [heapEntry, cell] at read
  | some node => exact ⟨node, rfl, by simpa only [heapEntry, cell, Option.bind_some] using read⟩

theorem native_entry_address_bounds (heap : NodeHeap) (image : EntryStorageImage) (limit : Nat)
    (storage : EntryStorageRep heap image limit) (address : NodeId) (entry : Entry)
    (read : heapEntry heap address = some entry) :
    ∃ native, nativeEntryAddress heap image address = some native ∧ 0 < native ∧ native < limit := by
  obtain ⟨node, cell, payload⟩ := heap_entry_cell heap address entry read
  obtain ⟨block, owner, inside⟩ := storage.entryInside address node entry cell payload
  obtain ⟨basePositive, bytesPositive, endWithin⟩ := storage.blocksWithin _ block owner
  have nonzero := storage.nonzeroEntry
  exact ⟨block.base + image.entryOffset address, by simp [nativeEntryAddress, cell, payload, owner], by omega, by omega⟩

theorem native_entry_address_injective (heap : NodeHeap) (image : EntryStorageImage) (limit : Nat)
    (storage : EntryStorageRep heap image limit) (first second : NodeId) (a b : Entry) (native : Nat)
    (firstEntry : heapEntry heap first = some a) (secondEntry : heapEntry heap second = some b)
    (firstAddress : nativeEntryAddress heap image first = some native)
    (secondAddress : nativeEntryAddress heap image second = some native) : first = second := by
  by_cases same : first = second
  · exact same
  · obtain ⟨firstNode, firstRead, firstPayload⟩ := heap_entry_cell heap first a firstEntry
    obtain ⟨secondNode, secondRead, secondPayload⟩ := heap_entry_cell heap second b secondEntry
    obtain ⟨firstBlock, firstOwner, firstInside⟩ := storage.entryInside first firstNode a firstRead firstPayload
    obtain ⟨secondBlock, secondOwner, secondInside⟩ := storage.entryInside second secondNode b secondRead secondPayload
    have ownerDifferent : (⟨first, residentEntryOwner firstNode⟩ : ResidentBlockId) ≠ ⟨second, residentEntryOwner secondNode⟩ :=
      fun equal => same (congrArg ResidentBlockId.node equal)
    have separate := storage.blocksSeparate _ _ firstBlock secondBlock ownerDifferent firstOwner secondOwner
    have firstEq : firstBlock.base + image.entryOffset first = native := by
      simpa only [nativeEntryAddress, firstRead, firstPayload, firstOwner, bind, Option.bind_some, Option.some.injEq] using firstAddress
    have secondEq : secondBlock.base + image.entryOffset second = native := by
      simpa only [nativeEntryAddress, secondRead, secondPayload, secondOwner, bind, Option.bind_some, Option.some.injEq] using secondAddress
    have nonzero := storage.nonzeroEntry
    cases separate <;> omega

inductive EntryAddresses (heap : NodeHeap) (image : EntryStorageImage) (limit : Nat) :
    List (NodeId × Entry) → List Nat → Prop where
  | nil : EntryAddresses heap image limit [] []
  | cons (location : NodeId × Entry) (native : Nat) (locations : List (NodeId × Entry)) (addresses : List Nat)
      (related : nativeEntryAddress heap image location.1 = some native ∧ 0 < native ∧ native < limit)
      (rest : EntryAddresses heap image limit locations addresses) :
      EntryAddresses heap image limit (location :: locations) (native :: addresses)

theorem entry_addresses_member (heap : NodeHeap) (image : EntryStorageImage) (limit : Nat)
    (locations : List (NodeId × Entry)) (addresses : List Nat) (paired : EntryAddresses heap image limit locations addresses)
    (native : Nat) (member : native ∈ addresses) :
    ∃ location ∈ locations, nativeEntryAddress heap image location.1 = some native := by
  induction paired with
  | nil => cases member
  | cons location address locations addresses related rest ih =>
      rcases List.mem_cons.mp member with same | later
      · exact ⟨location, by simp, by simpa only [same] using related.1⟩
      · obtain ⟨found, included, result⟩ := ih later
        exact ⟨found, by simp [included], result⟩

theorem entry_addresses_bounded (heap : NodeHeap) (image : EntryStorageImage) (limit : Nat)
    (locations : List (NodeId × Entry)) (addresses : List Nat) (paired : EntryAddresses heap image limit locations addresses) :
    ∀ address ∈ addresses, 0 < address ∧ address < limit := by
  induction paired with
  | nil => intro address member; cases member
  | cons location native locations addresses related rest ih =>
      intro address member
      rcases List.mem_cons.mp member with same | later
      · simpa only [same] using related.2
      · exact ih address later

theorem resident_entry_addresses (heap : NodeHeap) (image : EntryStorageImage) (limit : Nat)
    (storage : EntryStorageRep heap image limit) (locations : List (NodeId × Entry))
    (resident : EntryLocations heap locations) (unique : (locations.map Prod.fst).Nodup) :
    ∃ addresses : List Nat, addresses.length = locations.length ∧ addresses.Nodup ∧ EntryAddresses heap image limit locations addresses := by
  induction locations with
  | nil => exact ⟨[], rfl, List.nodup_nil, .nil⟩
  | cons location tail ih =>
      have split : location.1 ∉ tail.map Prod.fst ∧ (tail.map Prod.fst).Nodup := List.nodup_cons.mp unique
      obtain ⟨native, actual, positive, bound⟩ := native_entry_address_bounds heap image limit storage location.1 location.2
        (resident location (by simp))
      obtain ⟨addresses, lengthEq, distinct, paired⟩ := ih (fun item member => resident item (by simp [member])) split.2
      refine ⟨native :: addresses, by simp [lengthEq], List.nodup_cons.mpr ⟨?_, distinct⟩,
        .cons location native tail addresses ⟨actual, positive, bound⟩ paired⟩
      intro member
      obtain ⟨other, otherMember, sameAddress⟩ := entry_addresses_member heap image limit tail addresses paired native member
      have same := native_entry_address_injective heap image limit storage location.1 other.1 location.2 other.2 native
        (resident location (by simp)) (resident other (by simp [otherMember])) actual sameAddress
      exact split.1 (List.mem_map.mpr ⟨other, otherMember, same.symm⟩)

theorem positive_addresses_count_bound (addresses : List Nat) (limit : Nat)
    (positiveLimit : 0 < limit) (unique : addresses.Nodup)
    (within : ∀ address ∈ addresses, 0 < address ∧ address < limit) : addresses.length < limit := by
  have allDistinct : (0 :: addresses).Nodup := List.nodup_cons.mpr ⟨by
    intro member
    have positive := (within 0 member).1
    omega, unique⟩
  have subset : (0 :: addresses) ⊆ List.range limit := by
    intro address member
    apply List.mem_range.mpr
    rcases List.mem_cons.mp member with zero | other
    · simpa only [zero] using positiveLimit
    · exact (within address other).2
  have count := allDistinct.length_le_of_subset subset
  simp only [List.length_cons, List.length_range] at count
  omega

theorem native_resident_cardinality (heap : NodeHeap) (image : EntryStorageImage) (limit : Nat)
    (storage : EntryStorageRep heap image limit) (positiveLimit : 0 < limit) (root : Option NodeId) (tree : Option Tree)
    (rep : RootRep heap root tree) (valid : ValidOptional [] tree) : (optionalEntries tree).length < limit := by
  obtain ⟨locations, values, resident, unique⟩ := valid_root_entry_locations heap root tree rep valid
  obtain ⟨addresses, lengths, distinct, paired⟩ := resident_entry_addresses heap image limit storage locations resident unique
  have within := entry_addresses_bounded heap image limit locations addresses paired
  have bound := positive_addresses_count_bound addresses limit positiveLimit distinct within
  have count := congrArg List.length values
  simp only [List.length_map] at count
  omega

end Kv9.Radix
