import MachineBounds

set_option autoImplicit false

namespace Kv9.Radix

def EntryBefore (a b : Entry) : Prop := a.key < b.key
def SortedEntries (rows : List Entry) : Prop := rows.Pairwise EntryBefore

theorem key_prefix_lt (path a b : Key) : path ++ a < path ++ b ↔ a < b := by
  induction path with
  | nil => simp
  | cons byte tail ih => simpa only [List.cons_append, List.cons_lt_cons_self] using ih

theorem key_edge_lt (path left right : Key) (a b : UInt8) (less : a.toNat < b.toNat) :
    path ++ a :: left < path ++ b :: right := by
  apply (key_prefix_lt path _ _).mpr
  exact List.Lex.rel less

theorem key_extension_lt (path suffix : Key) (byte : UInt8) : path < path ++ byte :: suffix := by
  simpa using (key_prefix_lt path [] (byte :: suffix)).mpr List.Lex.nil

theorem key_lt_ne (a b : Key) (less : a < b) : a ≠ b := by
  intro same
  subst b
  exact List.lt_irrefl a less

theorem forest_terminal_before (path : Key) (forest : Forest) (valid : ValidForest path forest) :
    ∀ entry ∈ forestEntries forest, path < entry.key := by
  cases forest with
  | nil => simp [forestEntries]
  | cons byte child tail =>
      intro entry member
      simp only [forestEntries, List.mem_append] at member
      rcases member with inChild | inTail
      · obtain ⟨suffix, key⟩ := valid_tree_prefix _ _ valid.1 entry inChild
        simpa [key, List.append_assoc] using key_extension_lt path suffix byte
      · exact forest_terminal_before path tail valid.2.1 entry inTail

theorem forest_after_edge (path suffix : Key) (byte : UInt8) (forest : Forest)
    (valid : ValidForest path forest) (above : Above byte forest) :
    ∀ entry ∈ forestEntries forest, path ++ byte :: suffix < entry.key := by
  cases forest with
  | nil => simp [forestEntries]
  | cons next child tail =>
      intro entry member
      simp only [forestEntries, List.mem_append] at member
      rcases member with inChild | inTail
      · obtain ⟨rest, key⟩ := valid_tree_prefix _ _ valid.1 entry inChild
        simpa [key, List.append_assoc] using key_edge_lt path suffix rest byte next above.1
      · exact forest_after_edge path suffix byte tail valid.2.1 above.2 entry inTail

mutual
  theorem valid_tree_sorted (path : Key) (tree : Tree) (valid : Valid path tree)
      (ordered : OrderedTree tree) : SortedEntries (entries tree) := by
    cases tree with
    | leaf entry => simp [SortedEntries, entries]
    | branch pfx terminal children =>
        have childSorted := valid_forest_sorted (path ++ pfx) children valid.2 ordered
        cases terminal with
        | none => simpa [SortedEntries, entries] using childSorted
        | some entry =>
            rw [SortedEntries, entries]
            simp only [Option.toList_some, List.singleton_append, List.pairwise_cons]
            refine ⟨?_, childSorted⟩
            intro other member
            change entry.key < other.key
            rw [valid.1 entry rfl]
            exact forest_terminal_before _ _ valid.2 other member
  theorem valid_forest_sorted (path : Key) (forest : Forest) (valid : ValidForest path forest)
      (ordered : OrderedForest forest) : SortedEntries (forestEntries forest) := by
    cases forest with
    | nil => simp [SortedEntries, forestEntries]
    | cons byte child tail =>
        rw [SortedEntries, forestEntries, List.pairwise_append]
        refine ⟨valid_tree_sorted _ _ valid.1 ordered.1,
          valid_forest_sorted path tail valid.2.1 ordered.2.1, ?_⟩
        intro a aMember b bMember
        obtain ⟨suffix, key⟩ := valid_tree_prefix _ _ valid.1 a aMember
        change a.key < b.key
        simpa [key, List.append_assoc] using
          forest_after_edge path suffix byte tail valid.2.1 ordered.2.2 b bMember
end

theorem good_root_sorted (root : Option Tree) (good : Good root) : SortedEntries (optionalEntries root) := by
  cases root with
  | none => simp [SortedEntries, optionalEntries]
  | some tree => exact valid_tree_sorted [] tree good.1 (good.2 tree rfl).2

def accepts (query : Key) (inclusive reverse : Bool) (entry : Entry) : Bool :=
  (inclusive && decide (entry.key = query)) ||
    if reverse then decide (entry.key < query) else decide (query < entry.key)

theorem accepts_equal (query : Key) (inclusive reverse : Bool) (entry : Entry) (same : entry.key = query) :
    accepts query inclusive reverse entry = inclusive := by
  cases reverse <;> simp [accepts, same, List.lt_irrefl]

theorem accepts_less (query : Key) (inclusive reverse : Bool) (entry : Entry) (less : entry.key < query) :
    accepts query inclusive reverse entry = reverse := by
  have different := key_lt_ne _ _ less
  have opposite := List.lt_asymm less
  cases reverse <;> simp [accepts, different, less, opposite]

theorem accepts_greater (query : Key) (inclusive reverse : Bool) (entry : Entry) (less : query < entry.key) :
    accepts query inclusive reverse entry = !reverse := by
  have different := Ne.symm (key_lt_ne _ _ less)
  have opposite := List.lt_asymm less
  cases reverse <;> simp [accepts, different, less, opposite]

def orient (reverse : Bool) (rows : List Entry) : List Entry := if reverse then rows.reverse else rows

theorem orient_append (reverse : Bool) (a b : List Entry) :
    orient reverse (a ++ b) =
      if reverse then orient reverse b ++ orient reverse a else orient reverse a ++ orient reverse b := by
  cases reverse <;> simp [orient]

theorem filtered_entries_sorted (query : Key) (inclusive reverse : Bool) (rows : List Entry)
    (sorted : SortedEntries rows) : SortedEntries (rows.filter (accepts query inclusive reverse)) :=
  sorted.filter _

end Kv9.Radix
