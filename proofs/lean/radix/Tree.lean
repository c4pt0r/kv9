import Branches

set_option autoImplicit false

namespace Kv9.Radix

structure Entry where
  key : Key
  value : Value
deriving DecidableEq, Repr

-- Unlike a specification-only trie, leaves and terminals retain the full keys
-- stored in Rust. Forest is the ordered Vec<Edge> abstraction, with no sharing
-- or reference-count implementation hidden in its constructors.
mutual
  inductive Tree where
    | leaf (entry : Entry)
    | branch (pfx : Key) (terminal : Option Entry) (children : Forest)
  inductive Forest where
    | nil
    | cons (byte : UInt8) (child : Tree) (tail : Forest)
end

mutual
  def entries : Tree → List Entry
    | .leaf entry => [entry]
    | .branch _ terminal children => terminal.toList ++ forestEntries children
  def forestEntries : Forest → List Entry
    | .nil => []
    | .cons _ child tail => entries child ++ forestEntries tail
end

def linearLookup (query : Key) : List Entry → Option Value
  | [] => none
  | entry :: tail => if query = entry.key then some entry.value else linearLookup query tail

mutual
  def treeLookup (query : Key) : Tree → Key → Option Value
    | .leaf entry, _ => if query = entry.key then some entry.value else none
    | .branch pfx terminal children, remaining =>
        match strip pfx remaining with
        | none => none
        | some [] => terminal.map Entry.value
        | some (byte :: suffix) => forestLookup query children byte suffix
  def forestLookup (query : Key) : Forest → UInt8 → Key → Option Value
    | .nil, _, _ => none
    | .cons byte child tail, target, suffix =>
        if target = byte then treeLookup query child suffix else forestLookup query tail target suffix
end

def HasPrefix (pfx key : Key) : Prop := ∃ suffix, key = pfx ++ suffix

def MissingEdge (target : UInt8) : Forest → Prop
  | .nil => True
  | .cons byte _ tail => target ≠ byte ∧ MissingEdge target tail

mutual
  def Valid (path : Key) : Tree → Prop
    | .leaf entry => HasPrefix path entry.key
    | .branch pfx terminal children =>
        (∀ entry, terminal = some entry → entry.key = path ++ pfx) ∧ ValidForest (path ++ pfx) children
  def ValidForest (path : Key) : Forest → Prop
    | .nil => True
    | .cons byte child tail =>
        Valid (path ++ [byte]) child ∧ ValidForest path tail ∧ MissingEdge byte tail
end

def KeysExtend (path : Key) (rows : List Entry) : Prop :=
  ∀ entry ∈ rows, HasPrefix path entry.key

theorem prefix_refl (key : Key) : HasPrefix key key := by
  exact ⟨[], by simp⟩

theorem prefix_trans (a b c : Key) (ab : HasPrefix a b) (bc : HasPrefix b c) : HasPrefix a c := by
  obtain ⟨x, hx⟩ := ab
  obtain ⟨y, hy⟩ := bc
  exact ⟨x ++ y, by simp [hy, hx, List.append_assoc]⟩

theorem prefix_cancel (path pfx query : Key) :
    HasPrefix (path ++ pfx) (path ++ query) ↔ HasPrefix pfx query := by
  constructor
  · rintro ⟨suffix, eq⟩
    exact ⟨suffix, by simpa [List.append_assoc] using eq⟩
  · rintro ⟨suffix, rfl⟩
    exact ⟨suffix, by simp [List.append_assoc]⟩

theorem strip_none_iff (pfx query : Key) : strip pfx query = none ↔ ¬ HasPrefix pfx query := by
  constructor
  · intro h ⟨suffix, eq⟩
    simp [eq, strip_append] at h
  · intro absent
    cases h : strip pfx query with
    | none => rfl
    | some suffix => exact False.elim (absent ⟨suffix, (strip_some_iff _ _ _).mp h⟩)

theorem different_edge_prefix (path suffix : Key) (a b : UInt8) (different : a ≠ b) :
    ¬ HasPrefix (path ++ [a]) (path ++ b :: suffix) := by
  rw [prefix_cancel]
  rintro ⟨tail, eq⟩
  have same : b = a := by simpa using congrArg List.head? eq
  exact different same.symm

theorem extension_not_terminal (path suffix : Key) (byte : UInt8) :
    path ++ byte :: suffix ≠ path := by
  intro eq
  have lengths := congrArg List.length eq
  simp at lengths

theorem terminal_not_edge_prefix (path : Key) (byte : UInt8) :
    ¬ HasPrefix (path ++ [byte]) path := by
  rintro ⟨suffix, eq⟩
  exact extension_not_terminal path suffix byte (by simpa [List.append_assoc] using eq.symm)

theorem linear_append (query : Key) (a b : List Entry) :
    linearLookup query (a ++ b) = (linearLookup query a).orElse (fun _ => linearLookup query b) := by
  induction a with
  | nil => rfl
  | cons entry tail ih =>
      by_cases same : query = entry.key <;> simp [linearLookup, same, ih]

theorem linear_outside_prefix (path query : Key) (rows : List Entry)
    (valid : KeysExtend path rows) (outside : ¬ HasPrefix path query) :
    linearLookup query rows = none := by
  induction rows with
  | nil => rfl
  | cons entry tail ih =>
      have different : query ≠ entry.key := by
        intro eq
        exact outside (eq ▸ valid entry (by simp))
      simp only [linearLookup, if_neg different]
      exact ih (fun e member => valid e (by simp [member]))

-- Both proofs recurse structurally over Tree/Forest, not over a finite sample.
mutual
  theorem valid_tree_prefix (path : Key) (tree : Tree) (valid : Valid path tree) :
      KeysExtend path (entries tree) := by
    cases tree with
    | leaf entry =>
        intro e member
        have eq : e = entry := by simpa [entries] using member
        simpa [eq, Valid] using valid
    | branch pfx terminal children =>
        obtain ⟨terminalValid, childrenValid⟩ := valid
        intro entry member
        simp only [entries, List.mem_append] at member
        rcases member with terminalMember | childMember
        · have eq : terminal = some entry := by simpa using terminalMember
          exact ⟨pfx, terminalValid entry eq⟩
        · exact prefix_trans path (path ++ pfx) entry.key ⟨pfx, rfl⟩
            (valid_forest_prefix (path ++ pfx) children childrenValid entry childMember)
  theorem valid_forest_prefix (path : Key) (forest : Forest) (valid : ValidForest path forest) :
      KeysExtend path (forestEntries forest) := by
    cases forest with
    | nil => simp [KeysExtend, forestEntries]
    | cons byte child tail =>
        obtain ⟨childValid, tailValid, _⟩ := valid
        intro entry member
        simp only [forestEntries, List.mem_append] at member
        rcases member with childMember | tailMember
        · exact prefix_trans path (path ++ [byte]) entry.key ⟨[byte], rfl⟩
            (valid_tree_prefix (path ++ [byte]) child childValid entry childMember)
        · exact valid_forest_prefix path tail tailValid entry tailMember
end

theorem forest_terminal_absent (path : Key) (forest : Forest) (valid : ValidForest path forest) :
    linearLookup path (forestEntries forest) = none := by
  cases forest with
  | nil => rfl
  | cons byte child tail =>
      obtain ⟨childValid, tailValid, _⟩ := valid
      rw [forestEntries, linear_append,
        linear_outside_prefix (path ++ [byte]) path (entries child)
          (valid_tree_prefix _ _ childValid) (terminal_not_edge_prefix path byte),
        forest_terminal_absent path tail tailValid]
      rfl

theorem forest_missing_route (path suffix : Key) (target : UInt8) (forest : Forest)
    (valid : ValidForest path forest) (missing : MissingEdge target forest) :
    linearLookup (path ++ target :: suffix) (forestEntries forest) = none := by
  cases forest with
  | nil => rfl
  | cons byte child tail =>
      obtain ⟨childValid, tailValid, _⟩ := valid
      obtain ⟨different, restMissing⟩ := missing
      rw [forestEntries, linear_append,
        linear_outside_prefix (path ++ [byte]) (path ++ target :: suffix) (entries child)
          (valid_tree_prefix _ _ childValid) (different_edge_prefix path suffix byte target (Ne.symm different)),
        forest_missing_route path suffix target tail tailValid restMissing]
      rfl

theorem valid_branch_prefix (path pfx : Key) (terminal : Option Entry) (children : Forest)
    (valid : Valid path (.branch pfx terminal children)) :
    KeysExtend (path ++ pfx) (entries (.branch pfx terminal children)) := by
  obtain ⟨terminalValid, childrenValid⟩ := valid
  intro entry member
  simp only [entries, List.mem_append] at member
  rcases member with terminalMember | childMember
  · have eq : terminal = some entry := by simpa using terminalMember
    rw [terminalValid entry eq]
    exact prefix_refl _
  · exact valid_forest_prefix _ _ childrenValid entry childMember

mutual
  theorem tree_lookup_refines (path : Key) (tree : Tree) (remaining : Key)
      (valid : Valid path tree) :
      treeLookup (path ++ remaining) tree remaining = linearLookup (path ++ remaining) (entries tree) := by
    cases tree with
    | leaf entry => simp [treeLookup, entries, linearLookup]
    | branch pfx terminal children =>
        obtain ⟨terminalValid, childrenValid⟩ := valid
        cases h : strip pfx remaining with
        | none =>
            have outside : ¬ HasPrefix (path ++ pfx) (path ++ remaining) := by
              rw [prefix_cancel]
              exact (strip_none_iff _ _).mp h
            rw [treeLookup, h]
            exact (linear_outside_prefix _ _ _
              (valid_branch_prefix path pfx terminal children ⟨terminalValid, childrenValid⟩) outside).symm
        | some suffix =>
            cases suffix with
            | nil =>
                have eq := (strip_empty_suffix _ _).mp h
                rw [treeLookup, h, eq, entries, linear_append,
                  forest_terminal_absent _ _ childrenValid]
                cases terminal with
                | none => rfl
                | some entry => simp [linearLookup, terminalValid entry rfl]
            | cons byte rest =>
                have eq := (strip_some_iff _ _ _).mp h
                have query : path ++ remaining = (path ++ pfx) ++ byte :: rest := by
                  simp [eq, List.append_assoc]
                have terminalAbsent : linearLookup ((path ++ pfx) ++ byte :: rest) terminal.toList = none := by
                  cases terminal with
                  | none => rfl
                  | some entry =>
                      simp [linearLookup, terminalValid entry rfl]
                rw [treeLookup, h, entries, query, linear_append, terminalAbsent]
                simpa using forest_lookup_refines (path ++ pfx) children byte rest childrenValid
  theorem forest_lookup_refines (path : Key) (forest : Forest) (target : UInt8) (suffix : Key)
      (valid : ValidForest path forest) :
      forestLookup (path ++ target :: suffix) forest target suffix =
        linearLookup (path ++ target :: suffix) (forestEntries forest) := by
    cases forest with
    | nil => rfl
    | cons byte child tail =>
        obtain ⟨childValid, tailValid, missing⟩ := valid
        by_cases same : target = byte
        · subst target
          rw [forestLookup, if_pos rfl, forestEntries, linear_append,
            forest_missing_route path suffix byte tail tailValid missing]
          simpa [List.append_assoc] using tree_lookup_refines (path ++ [byte]) child suffix childValid
        · rw [forestLookup, if_neg same, forestEntries, linear_append,
            linear_outside_prefix (path ++ [byte]) (path ++ target :: suffix) (entries child)
              (valid_tree_prefix _ _ childValid) (different_edge_prefix path suffix byte target (Ne.symm same))]
          simpa using forest_lookup_refines path tail target suffix tailValid
end

theorem root_lookup_refines (tree : Tree) (query : Key) (valid : Valid [] tree) :
    treeLookup query tree query = linearLookup query (entries tree) := by
  simpa using tree_lookup_refines [] tree query valid

end Kv9.Radix
