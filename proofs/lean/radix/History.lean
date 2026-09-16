import Insert

set_option autoImplicit false

namespace Kv9.Radix

def Good (root : Option Tree) : Prop := ValidOptional [] root ∧
  (∀ tree, root = some tree → Canonical tree ∧ OrderedTree tree)

theorem empty_good : Good none := by simp [Good, ValidOptional]

theorem put_root_good (fresh : Entry) (root : Option Tree) (good : Good root) :
    Good (some (putRoot fresh root).1) := by
  refine ⟨put_root_valid fresh root good.1 (fun tree eq => (good.2 tree eq).2), ?_⟩
  intro result eq
  have resultEq : (putRoot fresh root).1 = result := Option.some.inj eq
  subst result
  cases root with
  | none => exact ⟨trivial, trivial⟩
  | some tree => exact ⟨insert_tree_canonical [] fresh tree (good.2 tree rfl).1,
      insert_tree_ordered [] fresh tree (good.2 tree rfl).2⟩

theorem erase_root_good (removed : Key) (root : Option Tree) (good : Good root) :
    Good (eraseRoot removed root) := by
  refine ⟨erase_root_valid removed root good.1, ?_⟩
  cases root with
  | none => simp [eraseRoot]
  | some tree =>
      by_cases absent : treeLookup removed tree removed = none
      · simpa [eraseRoot, absent] using good.2
      · intro result eq
        simp only [eraseRoot, if_neg absent] at eq
        exact ⟨delete_tree_canonical tree removed removed (good.2 tree rfl).1 result eq,
          delete_tree_ordered tree removed removed (good.2 tree rfl).2 result eq⟩

inductive Mutation where
  | put (entry : Entry)
  | erase (key : Key)

def applyMutation (root : Option Tree) : Mutation → Option Tree
  | .put entry => some (putRoot entry root).1
  | .erase key => eraseRoot key root

def specMutation (state : Map) : Mutation → Map
  | .put entry => put entry.key entry.value state
  | .erase key => erase key state

theorem mutation_good (root : Option Tree) (op : Mutation) (good : Good root) : Good (applyMutation root op) := by
  cases op with
  | put entry => exact put_root_good entry root good
  | erase key => exact erase_root_good key root good

theorem mutation_refines (root : Option Tree) (op : Mutation) (query : Key) (good : Good root) :
    optionalLookup query (applyMutation root op) = specMutation (fun key => optionalLookup key root) op query := by
  cases op with
  | put entry => exact put_root_lookup entry root query good.1 (fun tree eq => (good.2 tree eq).2)
  | erase key => exact erase_root_lookup query key root good.1

def run (root : Option Tree) (history : List Mutation) : Option Tree := history.foldl applyMutation root
def specRun (state : Map) (history : List Mutation) : Map := history.foldl specMutation state

theorem history_good (root : Option Tree) (history : List Mutation) (good : Good root) : Good (run root history) := by
  induction history generalizing root with
  | nil => exact good
  | cons op tail ih => exact ih (applyMutation root op) (mutation_good root op good)

theorem history_refines (root : Option Tree) (history : List Mutation) (query : Key) (good : Good root) :
    optionalLookup query (run root history) = specRun (fun key => optionalLookup key root) history query := by
  induction history generalizing root with
  | nil => rfl
  | cons op tail ih =>
      have first : (fun key => optionalLookup key (applyMutation root op)) =
          specMutation (fun key => optionalLookup key root) op := by
        funext key
        exact mutation_refines root op key good
      simpa only [run, specRun, List.foldl_cons, first] using
        ih (applyMutation root op) (mutation_good root op good)

-- A pure ownership abstraction. Rust correspondence additionally requires the
-- Arc/COW invariants; this is not a proof of reference counting or concurrency.
structure Snapshots where
  live : Option Tree
  saved : List (Option Tree)

def remember (state : Snapshots) : Snapshots := ⟨state.live, state.live :: state.saved⟩
def applySaved (state : Snapshots) (op : Mutation) : Snapshots :=
  ⟨applyMutation state.live op, state.saved⟩

theorem remembered_root (state : Snapshots) : (remember state).saved.head? = some state.live := rfl

theorem mutation_preserves_saved_roots (state : Snapshots) (op : Mutation) :
    (applySaved state op).saved = state.saved := rfl

theorem history_preserves_saved_roots (state : Snapshots) (history : List Mutation) :
    (history.foldl applySaved state).saved = state.saved := by
  induction history generalizing state with
  | nil => rfl
  | cons op tail ih => exact ih (applySaved state op)

end Kv9.Radix
