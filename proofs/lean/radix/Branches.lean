import Std

/- Local semantic rewrites for the actual radix split/collapse and path frames.
   This module alone is NOT whole-tree or Rust implementation refinement.
   Byte paths use UInt8. Subtree interpretations are pointwise finite-map views;
   the tree invariant and operational correspondence are separate obligations. -/
set_option autoImplicit false

namespace Kv9.Radix

abbrev Key := List UInt8
abbrev Value := List UInt8
abbrev Map := Key → Option Value
abbrev Edges := UInt8 → Map

def strip : Key → Key → Option Key
  | [], query => some query
  | _ :: _, [] => none
  | byte :: pfx, head :: tail => if byte = head then strip pfx tail else none

theorem strip_some_iff (pfx query suffix : Key) :
    strip pfx query = some suffix ↔ query = pfx ++ suffix := by
  induction pfx generalizing query with
  | nil => simp [strip]
  | cons byte pfx ih =>
      cases query with
      | nil => simp [strip]
      | cons head tail =>
          by_cases same : byte = head
          · subst head
            simp [strip, ih]
          · simp [strip, same, Ne.symm same]

theorem strip_append (pfx suffix : Key) :
    strip pfx (pfx ++ suffix) = some suffix :=
  (strip_some_iff pfx _ suffix).mpr rfl

theorem strip_empty_suffix (pfx query : Key) :
    strip pfx query = some [] ↔ query = pfx := by
  simpa using strip_some_iff pfx query []

theorem strip_consumed_bound (pfx query suffix : Key)
    (h : strip pfx query = some suffix) :
    pfx.length ≤ query.length ∧ query.length = pfx.length + suffix.length := by
  have eq := (strip_some_iff _ _ _).mp h
  simp [eq]

theorem strip_composed (a b query : Key) :
    strip (a ++ b) query = (strip a query).bind (strip b) := by
  induction a generalizing query with
  | nil => simp [strip]
  | cons head tail ih =>
      cases query with
      | nil => simp [strip]
      | cons byte rest =>
          by_cases same : head = byte <;> simp [strip, same, ih]

def empty : Map := fun _ => none
def singleton (key : Key) (value : Value) : Map :=
  fun query => if query = key then some value else none
def put (key : Key) (value : Value) (state : Map) : Map :=
  fun query => if query = key then some value else state query
def erase (key : Key) (state : Map) : Map :=
  fun query => if query = key then none else state query
def under (pfx : Key) (state : Map) : Map :=
  fun query => (strip pfx query).bind state
def route (terminal : Option Value) (edges : Edges) : Map
  | [] => terminal
  | byte :: suffix => edges byte suffix
def noEdges : Edges := fun _ => empty
def replaceEdge (byte : UInt8) (child : Map) (edges : Edges) : Edges :=
  fun query => if query = byte then child else edges query
def oneEdge (byte : UInt8) (child : Map) : Edges := replaceEdge byte child noEdges

theorem under_nil (state : Map) : under [] state = state := by
  funext query
  simp [under, strip]

theorem under_composed (a b : Key) (state : Map) :
    under a (under b state) = under (a ++ b) state := by
  funext query
  change (strip a query).bind (fun suffix => (strip b suffix).bind state) =
    (strip (a ++ b) query).bind state
  rw [strip_composed, Option.bind_assoc]

theorem under_empty (pfx : Key) : under pfx empty = empty := by
  funext query
  simp [under, empty]

theorem under_singleton (pfx key : Key) (value : Value) :
    under pfx (singleton key value) = singleton (pfx ++ key) value := by
  funext query
  cases h : strip pfx query with
  | none =>
      have absent : query ≠ pfx ++ key := by
        intro eq
        simp [eq, strip_append] at h
      simp [under, singleton, h, absent]
  | some suffix =>
      have eq := (strip_some_iff _ _ _).mp h
      simp [under, singleton, eq, strip_append]

theorem under_put (pfx key : Key) (value : Value) (state : Map) :
    under pfx (put key value state) = put (pfx ++ key) value (under pfx state) := by
  funext query
  cases h : strip pfx query with
  | none =>
      have absent : query ≠ pfx ++ key := by
        intro eq
        simp [eq, strip_append] at h
      simp [under, put, h, absent]
  | some suffix =>
      have eq := (strip_some_iff _ _ _).mp h
      simp [under, put, eq, strip_append]

theorem under_erase (pfx key : Key) (state : Map) :
    under pfx (erase key state) = erase (pfx ++ key) (under pfx state) := by
  funext query
  cases h : strip pfx query with
  | none =>
      have absent : query ≠ pfx ++ key := by
        intro eq
        simp [eq, strip_append] at h
      simp [under, erase, h, absent]
  | some suffix =>
      have eq := (strip_some_iff _ _ _).mp h
      simp [under, erase, eq, strip_append]

theorem route_empty : route none noEdges = empty := by
  funext query
  cases query <;> rfl

theorem route_only_terminal (value : Value) :
    route (some value) noEdges = singleton [] value := by
  funext query
  cases query <;> simp [route, noEdges, empty, singleton]

theorem route_one_edge (byte : UInt8) (child : Map) :
    route none (oneEdge byte child) = under [byte] child := by
  funext query
  cases query with
  | nil => rfl
  | cons head tail =>
      by_cases same : head = byte
      · subst head
        simp [route, oneEdge, replaceEdge, under, strip]
      · simp [route, oneEdge, replaceEdge, noEdges, empty, under, strip, same, Ne.symm same]

theorem terminal_put (terminal : Option Value) (edges : Edges) (value : Value) :
    route (some value) edges = put [] value (route terminal edges) := by
  funext query
  cases query <;> simp [route, put]

theorem terminal_erase (terminal : Option Value) (edges : Edges) :
    route none edges = erase [] (route terminal edges) := by
  funext query
  cases query <;> simp [route, erase]

theorem child_put (terminal : Option Value) (edges : Edges)
    (byte : UInt8) (key : Key) (value : Value) :
    route terminal (replaceEdge byte (put key value (edges byte)) edges) =
      put (byte :: key) value (route terminal edges) := by
  funext query
  cases query with
  | nil => simp [route, put]
  | cons head tail =>
      by_cases same : head = byte
      · subst head
        simp [route, replaceEdge, put]
      · simp [route, replaceEdge, put, same]

theorem child_erase (terminal : Option Value) (edges : Edges)
    (byte : UInt8) (key : Key) :
    route terminal (replaceEdge byte (erase key (edges byte)) edges) =
      erase (byte :: key) (route terminal edges) := by
  funext query
  cases query with
  | nil => simp [route, erase]
  | cons head tail =>
      by_cases same : head = byte
      · subst head
        simp [route, replaceEdge, erase]
      · simp [route, replaceEdge, erase, same]

theorem new_child_put (pfx : Key) (terminal : Option Value) (edges : Edges)
    (byte : UInt8) (key : Key) (value : Value) (absent : edges byte = empty) :
    under pfx (route terminal (replaceEdge byte (singleton key value) edges)) =
      put (pfx ++ byte :: key) value (under pfx (route terminal edges)) := by
  have replacement : singleton key value = put key value (edges byte) := by
    rw [absent]
    rfl
  rw [replacement, child_put, under_put]

theorem terminal_put_under (pfx : Key) (terminal : Option Value)
    (edges : Edges) (value : Value) :
    under pfx (route (some value) edges) =
      put pfx value (under pfx (route terminal edges)) := by
  rw [terminal_put terminal, under_put]
  simp

theorem terminal_erase_under (pfx : Key) (terminal : Option Value) (edges : Edges) :
    under pfx (route none edges) = erase pfx (under pfx (route terminal edges)) := by
  rw [terminal_erase terminal, under_erase]
  simp

theorem collapse_empty (pfx : Key) : under pfx (route none noEdges) = empty := by
  rw [route_empty, under_empty]

theorem collapse_terminal (pfx : Key) (value : Value) :
    under pfx (route (some value) noEdges) = singleton pfx value := by
  rw [route_only_terminal, under_singleton]
  simp

theorem collapse_branch (pfx childPrefix : Key) (byte : UInt8) (child : Map) :
    under pfx (route none (oneEdge byte (under childPrefix child))) =
      under (pfx ++ byte :: childPrefix) child := by
  rw [route_one_edge, under_composed, under_composed]
  simp

theorem collapse_leaf (pfx suffix : Key) (byte : UInt8) (value : Value) :
    under pfx (route none (oneEdge byte (singleton suffix value))) =
      singleton (pfx ++ byte :: suffix) value := by
  rw [route_one_edge, under_singleton, under_singleton]
  rfl

theorem split_branch (pfx rest : Key) (byte : UInt8) (body : Map) :
    under (pfx ++ byte :: rest) body =
      under pfx (route none (oneEdge byte (under rest body))) :=
  (collapse_branch pfx rest byte body).symm

theorem split_leaf_old_terminal (pfx suffix : Key) (byte : UInt8) (old fresh : Value) :
    under pfx (route (some old) (oneEdge byte (singleton suffix fresh))) =
      put (pfx ++ byte :: suffix) fresh (singleton pfx old) := by
  unfold oneEdge
  rw [new_child_put pfx (some old) noEdges byte suffix fresh (by rfl), collapse_terminal]

theorem split_leaf_new_terminal (pfx suffix : Key) (byte : UInt8) (old fresh : Value) :
    under pfx (route (some fresh) (oneEdge byte (singleton suffix old))) =
      put pfx fresh (singleton (pfx ++ byte :: suffix) old) := by
  rw [terminal_put_under pfx none, collapse_leaf]

theorem split_leaf_different_edges (pfx oldSuffix newSuffix : Key)
    (oldByte newByte : UInt8) (old fresh : Value) (different : newByte ≠ oldByte) :
    under pfx (route none
      (replaceEdge newByte (singleton newSuffix fresh) (oneEdge oldByte (singleton oldSuffix old)))) =
      put (pfx ++ newByte :: newSuffix) fresh (singleton (pfx ++ oldByte :: oldSuffix) old) := by
  rw [new_child_put pfx none _ newByte newSuffix fresh (by
    simp [oneEdge, replaceEdge, noEdges, different]), collapse_leaf]

structure Frame where
  pfx : Key
  terminal : Option Value
  byte : UInt8
  otherEdges : Edges

def plug (frame : Frame) (child : Map) : Map :=
  under frame.pfx (route frame.terminal (replaceEdge frame.byte child frame.otherEdges))

def liftKey (frame : Frame) (key : Key) : Key := frame.pfx ++ frame.byte :: key

theorem replace_edge_twice (byte : UInt8) (a b : Map) (edges : Edges) :
    replaceEdge byte a (replaceEdge byte b edges) = replaceEdge byte a edges := by
  funext query
  by_cases same : query = byte <;> simp [replaceEdge, same]

theorem replace_edge_self (byte : UInt8) (child : Map) (edges : Edges) :
    replaceEdge byte child edges byte = child := by
  simp [replaceEdge]

theorem plug_put (frame : Frame) (key : Key) (value : Value) (child : Map) :
    plug frame (put key value child) = put (liftKey frame key) value (plug frame child) := by
  have h := child_put frame.terminal (replaceEdge frame.byte child frame.otherEdges) frame.byte key value
  rw [replace_edge_self] at h
  have h' := congrArg (under frame.pfx) h
  simpa only [replace_edge_twice, under_put, plug, liftKey] using h'

theorem plug_erase (frame : Frame) (key : Key) (child : Map) :
    plug frame (erase key child) = erase (liftKey frame key) (plug frame child) := by
  have h := child_erase frame.terminal (replaceEdge frame.byte child frame.otherEdges) frame.byte key
  rw [replace_edge_self] at h
  have h' := congrArg (under frame.pfx) h
  simpa only [replace_edge_twice, under_erase, plug, liftKey] using h'

-- Frames are in pop order: immediate parent first, root last.
def restore (frames : List Frame) (child : Map) : Map :=
  frames.foldl (fun state frame => plug frame state) child
def restoreKey (frames : List Frame) (key : Key) : Key :=
  frames.foldl (fun suffix frame => liftKey frame suffix) key

theorem restore_put (frames : List Frame) (key : Key) (value : Value) (child : Map) :
    restore frames (put key value child) = put (restoreKey frames key) value (restore frames child) := by
  induction frames generalizing key child with
  | nil => rfl
  | cons frame tail ih =>
      simp only [restore, restoreKey, List.foldl_cons]
      rw [plug_put]
      exact ih (liftKey frame key) (plug frame child)

theorem restore_erase (frames : List Frame) (key : Key) (child : Map) :
    restore frames (erase key child) = erase (restoreKey frames key) (restore frames child) := by
  induction frames generalizing key child with
  | nil => rfl
  | cons frame tail ih =>
      simp only [restore, restoreKey, List.foldl_cons]
      rw [plug_erase]
      exact ih (liftKey frame key) (plug frame child)

end Kv9.Radix
