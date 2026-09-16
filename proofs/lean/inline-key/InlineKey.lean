import Std

/- This is a representation-refinement proof, not verified Rust extraction.
   Rust's safe slice copy, Vec ownership/Clone, byte-slice comparison, compiler,
   and rpds' ordered-map/persistent-snapshot contracts remain explicit premises.
   The list state below models a finite key/value association sequence. Mutation
   theorems preserve its contents; ordered-access theorems preserve the order of
   the supplied traversal. That traversal being sorted is an rpds premise, not
   a property of the prepend-based abstract put. This proof does not model
   balancing, allocation, panics, locks, or Raft. -/
namespace Kv9.InlineKey

abbrev Bytes := List UInt8
def capacity : Nat := 40

inductive Key where
  | inline (len : Nat) (storage : Bytes)
  | heap (bytes : Bytes)
deriving DecidableEq

def encode (bytes : Bytes) : Key :=
  if bytes.length ≤ capacity then
    .inline bytes.length (bytes ++ List.replicate (capacity - bytes.length) 0)
  else .heap bytes

def decode : Key → Bytes
  | .inline len storage => storage.take len
  | .heap bytes => bytes

def valid : Key → Prop
  | .inline len storage => len ≤ capacity ∧ storage.length = capacity
  | .heap _ => True

theorem padding_unobservable (bytes padding : Bytes) :
    decode (.inline bytes.length (bytes ++ padding)) = bytes := by
  simp [decode]

theorem decode_encode (bytes : Bytes) : decode (encode bytes) = bytes := by
  unfold encode
  split <;> simp [decode]

theorem encoded_valid (bytes : Bytes) : valid (encode bytes) := by
  unfold encode
  split
  · rename_i h
    simp [valid, List.length_append, List.length_replicate]
    omega
  · trivial

theorem inline_length_fits_u8 (len : Nat) (h : len ≤ capacity) : len < 256 := by
  unfold capacity at h
  omega

theorem inline_length_cast (len : Nat) (h : len ≤ capacity) :
    (UInt8.ofNat len).toNat = len := by
  exact UInt8.toNat_ofNat_of_lt' (inline_length_fits_u8 len h)

theorem encode_injective (a b : Bytes) (h : encode a = encode b) : a = b := by
  have eq := congrArg decode h
  simpa only [decode_encode] using eq

def keyEq (a b : Key) : Bool := decode a == decode b
def keyCompare (a b : Key) : Ordering := compare (decode a) (decode b)
def borrowed (a : Key) : Bytes := decode a

theorem encoded_equality (a b : Bytes) : keyEq (encode a) (encode b) = (a == b) := by
  simp [keyEq, decode_encode]

theorem encoded_order (a b : Bytes) : keyCompare (encode a) (encode b) = compare a b := by
  simp [keyCompare, decode_encode]

theorem borrowed_order (a b : Key) : keyCompare a b = compare (borrowed a) (borrowed b) := by
  rfl

theorem representation_independent (a b c : Key) (h : decode a = decode b) :
    keyEq a c = keyEq b c ∧ keyCompare a c = keyCompare b c := by
  simp [keyEq, keyCompare, h]

abbrev State (V : Type) := List (Bytes × V)
abbrev Stored (V : Type) := List (Key × V)
def project (state : Stored V) : State V := state.map fun (k, v) => (decode k, v)

def lookup (query : Bytes) : State V → Option V
  | [] => none
  | (key, value) :: tail => if query == key then some value else lookup query tail
def storedLookup (query : Bytes) : Stored V → Option V
  | [] => none
  | (key, value) :: tail => if query == borrowed key then some value else storedLookup query tail

theorem lookup_preserved (query : Bytes) (state : Stored V) :
    storedLookup query state = lookup query (project state) := by
  induction state with
  | nil => rfl
  | cons entry tail ih =>
    rcases entry with ⟨key, value⟩
    simp [storedLookup, lookup, project, borrowed, ih]

def erase (query : Bytes) (state : State V) : State V :=
  state.filter fun entry => !(query == entry.1)
def storedErase (query : Bytes) (state : Stored V) : Stored V :=
  state.filter fun entry => !(query == borrowed entry.1)

theorem erase_preserved (query : Bytes) (state : Stored V) :
    project (storedErase query state) = erase query (project state) := by
  induction state with
  | nil => rfl
  | cons entry tail ih =>
    rcases entry with ⟨key, value⟩
    simp only [storedErase, erase, project, borrowed, List.filter_cons, List.map_cons] at ih ⊢
    split <;> simp_all

def put (key : Bytes) (value : V) (state : State V) : State V :=
  (key, value) :: erase key state
def storedPut (key : Bytes) (value : V) (state : Stored V) : Stored V :=
  (encode key, value) :: storedErase key state

theorem put_preserved (key : Bytes) (value : V) (state : Stored V) :
    project (storedPut key value state) = put key value (project state) := by
  change (decode (encode key), value) :: project (storedErase key state) =
    (key, value) :: erase key (project state)
  rw [decode_encode, erase_preserved]

def inRange (start stop key : Bytes) : Bool :=
  compare key start != .lt && compare key stop == .lt
def range (start stop : Bytes) (state : State V) : State V :=
  state.filter fun entry => inRange start stop entry.1
def storedRange (start stop : Bytes) (state : Stored V) : Stored V :=
  state.filter fun entry => inRange start stop (borrowed entry.1)

theorem range_preserved (start stop : Bytes) (state : Stored V) :
    project (storedRange start stop state) = range start stop (project state) := by
  induction state with
  | nil => rfl
  | cons entry tail ih =>
    rcases entry with ⟨key, value⟩
    simp only [storedRange, range, project, borrowed, List.filter_cons, List.map_cons] at ih ⊢
    split <;> simp_all

theorem reverse_preserved (state : Stored V) :
    project state.reverse = (project state).reverse := by
  simp [project]

theorem limit_preserved (limit : Nat) (state : Stored V) :
    project (state.take limit) = (project state).take limit := by
  simp [project]

inductive Mutation (V : Type) where
  | put (key : Bytes) (value : V)
  | delete (key : Bytes)
def apply (state : State V) : Mutation V → State V
  | .put key value => put key value state
  | .delete key => erase key state
def storedApply (state : Stored V) : Mutation V → Stored V
  | .put key value => storedPut key value state
  | .delete key => storedErase key state

theorem mutation_preserved (state : Stored V) (mutation : Mutation V) :
    project (storedApply state mutation) = apply (project state) mutation := by
  cases mutation with
  | put key value => exact put_preserved key value state
  | delete key => exact erase_preserved key state

theorem history_preserved (history : List (Mutation V)) (state : Stored V) :
    project (history.foldl storedApply state) = history.foldl apply (project state) := by
  induction history generalizing state with
  | nil => rfl
  | cons head tail ih => simp [List.foldl_cons, ih, mutation_preserved]

theorem lookup_after_history (history : List (Mutation V)) (state : Stored V) (query : Bytes) :
    storedLookup query (history.foldl storedApply state) =
      lookup query (history.foldl apply (project state)) := by
  rw [lookup_preserved, history_preserved]

end Kv9.InlineKey
