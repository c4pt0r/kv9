import CursorTraversal

set_option autoImplicit false

namespace Kv9.Radix

theorem filter_constant (rows : List Entry) (predicate : Entry → Bool) (flag : Bool)
    (constant : ∀ entry ∈ rows, predicate entry = flag) :
    rows.filter predicate = if flag then rows else [] := by
  induction rows with
  | nil => cases flag <;> rfl
  | cons entry tail ih =>
      have head := constant entry (by simp)
      have rest := ih (fun e member => constant e (by simp [member]))
      cases flag <;> simp [head, rest]

-- Under the proper-mismatch guard, the old prefix tail is nonempty. Reading
-- these heads models prefix[common] and suffix.get(common), respectively.
def mismatchGreater (pfx suffix : Key) : Bool :=
  let shared := common pfx suffix
  match pfx.drop shared.length, suffix.drop shared.length with
  | _ :: _, [] => true
  | a :: _, b :: _ => decide (b < a)
  | [], _ => false

theorem mismatch_accepts (path pfx query : Key) (inclusive reverse : Bool) (entry : Entry)
    (queryValid : HasPrefix path query) (entryValid : HasPrefix (path ++ pfx) entry.key)
    (proper : (common pfx (query.drop path.length)).length < pfx.length) :
    accepts query inclusive reverse entry = (mismatchGreater pfx (query.drop path.length) != reverse) := by
  let suffix := query.drop path.length
  let shared := common pfx suffix
  have left := common_decompose_left pfx suffix
  have right := common_decompose_right pfx suffix
  have separated := common_separates pfx suffix
  obtain ⟨extra, entryKey⟩ := entryValid
  have context : path ++ pfx = (path ++ shared) ++ pfx.drop shared.length := by
    calc
      path ++ pfx = path ++ (shared ++ pfx.drop shared.length) :=
        congrArg (fun bytes : Key => path ++ bytes) left
      _ = _ := (List.append_assoc path shared _).symm
  have queryKey : query = (path ++ shared) ++ suffix.drop shared.length := by
    calc
      query = path ++ suffix := prefix_drop path query queryValid
      _ = path ++ (shared ++ suffix.drop shared.length) :=
        congrArg (fun bytes : Key => path ++ bytes) right
      _ = _ := (List.append_assoc path shared _).symm
  rw [context] at entryKey
  change accepts query inclusive reverse entry = (mismatchGreater pfx suffix != reverse)
  dsimp only [mismatchGreater]
  change accepts query inclusive reverse entry =
    ((match pfx.drop shared.length, suffix.drop shared.length with
      | _ :: _, [] => true
      | a :: _, b :: _ => decide (b < a)
      | [], _ => false) != reverse)
  cases oldEq : pfx.drop shared.length with
  | nil =>
      have bound := List.drop_eq_nil_iff.mp oldEq
      change shared.length < pfx.length at proper
      omega
  | cons a oldTail =>
      rw [oldEq] at entryKey separated
      cases queryEq : suffix.drop shared.length with
      | nil =>
          rw [queryEq] at queryKey
          have greater : query < entry.key := by
            simpa [queryKey, entryKey, List.append_assoc] using
              key_extension_lt (path ++ shared) (oldTail ++ extra) a
          rw [accepts_greater query inclusive reverse entry greater]
          cases reverse <;> rfl
      | cons b queryTail =>
          rw [queryEq] at queryKey separated
          have different : a ≠ b := separated
          by_cases greater : b < a
          · have less : query < entry.key := by
              simpa [queryKey, entryKey, List.append_assoc] using
                key_edge_lt (path ++ shared) queryTail (oldTail ++ extra) b a greater
            rw [accepts_greater query inclusive reverse entry less]
            simp only [decide_eq_true greater]
            cases reverse <;> rfl
          · have before : a.toNat < b.toNat := byte_reverse_lt b a (Ne.symm different) greater
            have less : entry.key < query := by
              simpa [queryKey, entryKey, List.append_assoc] using
                key_edge_lt (path ++ shared) (oldTail ++ extra) queryTail a b before
            rw [accepts_less query inclusive reverse entry less]
            simp only [decide_eq_false greater]
            cases reverse <;> rfl

theorem forest_accepts_after (path suffix : Key) (target : UInt8) (forest : Forest)
    (inclusive reverse : Bool) (valid : ValidForest path forest) (above : Above target forest) :
    ∀ entry ∈ forestEntries forest, accepts (path ++ target :: suffix) inclusive reverse entry = !reverse := by
  intro entry member
  exact accepts_greater _ _ _ entry (forest_after_edge path suffix target forest valid above entry member)

theorem tree_accepts_before (path suffix : Key) (byte target : UInt8) (tree : Tree)
    (inclusive reverse : Bool) (valid : Valid (path ++ [byte]) tree) (less : byte.toNat < target.toNat) :
    ∀ entry ∈ entries tree, accepts (path ++ target :: suffix) inclusive reverse entry = reverse := by
  intro entry member
  obtain ⟨extra, entryKey⟩ := valid_tree_prefix _ _ valid entry member
  apply accepts_less
  simpa [entryKey, List.append_assoc] using key_edge_lt path extra suffix byte target less

theorem terminal_accepts_before (path suffix : Key) (target : UInt8) (terminal : Option Entry)
    (inclusive reverse : Bool) (valid : ∀ entry, terminal = some entry → entry.key = path) :
    ∀ entry ∈ terminal.toList, accepts (path ++ target :: suffix) inclusive reverse entry = reverse := by
  intro entry member
  have someEntry : terminal = some entry := by simpa using member
  apply accepts_less
  rw [valid entry someEntry]
  exact key_extension_lt path suffix target

end Kv9.Radix
