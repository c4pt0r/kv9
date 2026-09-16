import Std

/- Representation refinement, not verified Rust extraction. The source mapping
   assumes Rust safe copy/slice/Vec/Box behavior, byte comparison, compilation,
   and rpds whole-entry replacement and persistent snapshots. Association-list
   mutations describe map contents; traversal ordering is an rpds premise.
   Allocation failure, tree balancing, locks and Raft are not modeled here. -/
namespace Kv9.KeyClamp

abbrev Bytes := List UInt8
structure Buffer where
  bytes : Bytes
  keyLength : Nat
deriving DecidableEq

def encode (key value : Bytes) : Buffer := ⟨key ++ value, key.length⟩
def key (entry : Buffer) : Bytes := entry.bytes.take (min entry.keyLength entry.bytes.length)
def value (entry : Buffer) : Bytes := entry.bytes.drop entry.keyLength
def pair (entry : Buffer) : Bytes × Bytes := (key entry, value entry)
def valid (entry : Buffer) : Prop := entry.keyLength ≤ entry.bytes.length

theorem key_round_trip (k v : Bytes) : key (encode k v) = k := by
  simp [key, encode]
theorem value_round_trip (k v : Bytes) : value (encode k v) = v := by
  simp [value, encode]
theorem pair_round_trip (k v : Bytes) : pair (encode k v) = (k, v) := by
  simp [pair, key_round_trip, value_round_trip]
theorem encoded_valid (k v : Bytes) : valid (encode k v) := by
  simp [valid, encode]
theorem split_preserves_all_bytes (entry : Buffer) : key entry ++ value entry = entry.bytes := by
  simp only [key, value, ← List.take_eq_take_min, List.take_append_drop]
theorem encode_injective (k v k' v' : Bytes) (h : encode k v = encode k' v') :
    k = k' ∧ v = v' := by
  have h' := congrArg pair h
  simpa only [pair_round_trip, Prod.mk.injEq] using h'

def checkedLength (wordMax cap k v : Nat) : Option Nat :=
  if k + v ≤ wordMax then if k + v ≤ cap then some (k + v) else none else none

theorem length_check_exact (wordMax cap k v : Nat) (h : cap ≤ wordMax) :
    checkedLength wordMax cap k v = if k + v ≤ cap then some (k + v) else none := by
  unfold checkedLength
  split
  · rfl
  · rename_i overflow
    have tooLarge : ¬ k + v ≤ cap := by omega
    simp [tooLarge]

theorem accepted_length_is_exact (wordMax cap k v total : Nat)
    (h : checkedLength wordMax cap k v = some total) :
    total = k + v ∧ total ≤ cap ∧ total ≤ wordMax ∧ k ≤ total := by
  unfold checkedLength at h
  split at h <;> simp_all
  omega

def eqKey (a b : Buffer) : Bool := key a == key b
def compareKey (a b : Buffer) : Ordering := compare (key a) (key b)
def borrow (entry : Buffer) : Bytes := key entry

theorem values_do_not_affect_equality (k v v' : Bytes) :
    eqKey (encode k v) (encode k v') = true := by
  simp [eqKey, key_round_trip]
theorem compare_preserved (a av b bv : Bytes) :
    compareKey (encode a av) (encode b bv) = compare a b := by
  simp [compareKey, key_round_trip]
theorem equality_preserved (a av b bv : Bytes) :
    eqKey (encode a av) (encode b bv) = (a == b) := by
  simp [eqKey, key_round_trip]
theorem borrowed_order (a b : Buffer) : compareKey a b = compare (borrow a) (borrow b) := by
  rfl

abbrev State := List (Bytes × Bytes)
abbrev Stored := List Buffer
def project (state : Stored) : State := state.map pair
def lookup (query : Bytes) : State → Option Bytes
  | [] => none
  | (k, v) :: tail => if query == k then some v else lookup query tail
def storedLookup (query : Bytes) : Stored → Option Bytes
  | [] => none
  | entry :: tail => if query == borrow entry then some (value entry) else storedLookup query tail

theorem lookup_preserved (query : Bytes) (state : Stored) :
    storedLookup query state = lookup query (project state) := by
  induction state with
  | nil => rfl
  | cons entry tail ih => simp [storedLookup, lookup, project, pair, borrow, ih]

def erase (query : Bytes) (state : State) : State := state.filter fun entry => !(query == entry.1)
def storedErase (query : Bytes) (state : Stored) : Stored := state.filter fun entry => !(query == borrow entry)

theorem erase_preserved (query : Bytes) (state : Stored) :
    project (storedErase query state) = erase query (project state) := by
  induction state with
  | nil => rfl
  | cons entry tail ih =>
    simp only [storedErase, erase, project, borrow, pair, List.filter_cons, List.map_cons] at ih ⊢
    split <;> simp_all [pair]

def put (k v : Bytes) (state : State) : State := (k, v) :: erase k state
def storedPut (k v : Bytes) (state : Stored) : Stored := encode k v :: storedErase k state

theorem put_preserved (k v : Bytes) (state : Stored) :
    project (storedPut k v state) = put k v (project state) := by
  change pair (encode k v) :: project (storedErase k state) = (k, v) :: erase k (project state)
  rw [pair_round_trip, erase_preserved]

theorem replacement_returns_new_value (k v : Bytes) (state : Stored) :
    storedLookup k (storedPut k v state) = some v := by
  simp [storedLookup, storedPut, borrow, key_round_trip, value_round_trip]

def inRange (start stop k : Bytes) : Bool := compare k start != .lt && compare k stop == .lt
def range (start stop : Bytes) (state : State) : State := state.filter fun entry => inRange start stop entry.1
def storedRange (start stop : Bytes) (state : Stored) : Stored := state.filter fun entry => inRange start stop (borrow entry)

theorem range_preserved (start stop : Bytes) (state : Stored) :
    project (storedRange start stop state) = range start stop (project state) := by
  induction state with
  | nil => rfl
  | cons entry tail ih =>
    simp only [storedRange, range, project, borrow, pair, List.filter_cons, List.map_cons] at ih ⊢
    split <;> simp_all [pair]
theorem reverse_preserved (state : Stored) : project state.reverse = (project state).reverse := by
  simp [project]
theorem limit_preserved (limit : Nat) (state : Stored) : project (state.take limit) = (project state).take limit := by
  simp [project]

inductive Mutation where
  | put (k v : Bytes)
  | delete (k : Bytes)
def apply (state : State) : Mutation → State
  | .put k v => put k v state
  | .delete k => erase k state
def storedApply (state : Stored) : Mutation → Stored
  | .put k v => storedPut k v state
  | .delete k => storedErase k state
theorem mutation_preserved (state : Stored) (mutation : Mutation) :
    project (storedApply state mutation) = apply (project state) mutation := by
  cases mutation with
  | put k v => exact put_preserved k v state
  | delete k => exact erase_preserved k state
theorem history_preserved (history : List Mutation) (state : Stored) :
    project (history.foldl storedApply state) = history.foldl apply (project state) := by
  induction history generalizing state with
  | nil => rfl
  | cons head tail ih => simp [List.foldl_cons, ih, mutation_preserved]

def fits (wordMax cap : Nat) : Mutation → Bool
  | .put k v => (checkedLength wordMax cap k.length v.length).isSome
  | .delete _ => true
def checkedBatch (wordMax cap : Nat) (history : List Mutation) (state : Stored) : Bool × Stored :=
  if history.all (fits wordMax cap) then (true, history.foldl storedApply state) else (false, state)

theorem rejected_batch_preserves_state (wordMax cap : Nat) (history : List Mutation) (state : Stored)
    (h : history.all (fits wordMax cap) = false) :
    checkedBatch wordMax cap history state = (false, state) := by
  simp [checkedBatch, h]
theorem accepted_batch_refines_history (wordMax cap : Nat) (history : List Mutation) (state : Stored)
    (h : history.all (fits wordMax cap) = true) :
    project (checkedBatch wordMax cap history state).2 = history.foldl apply (project state) := by
  simp [checkedBatch, h, history_preserved]

theorem valid_clamp_identity (entry : Buffer) (h : valid entry) :
    min entry.keyLength entry.bytes.length = entry.keyLength := by
  exact Nat.min_eq_left h

theorem valid_key_matches_original (entry : Buffer) (h : valid entry) :
    key entry = entry.bytes.take entry.keyLength := by
  simp only [key, valid_clamp_identity entry h]

theorem cloned_entry_valid (entry : Buffer) (h : valid entry) :
    valid { bytes := entry.bytes, keyLength := entry.keyLength } := by
  exact h

end Kv9.KeyClamp
