import SeekBounds

set_option autoImplicit false

namespace Kv9.Radix

def endpointTasks (inclusive reverse : Bool) (terminal : Option Entry) (children : Forest) : List CursorTask :=
  (if inclusive then terminalTasks terminal else []) ++ (if reverse then [] else forestTasks children)

mutual
  def seekTree (path query : Key) (inclusive reverse : Bool) : Tree → List CursorTask
    | .leaf entry => if accepts query inclusive reverse entry then [.entry entry] else []
    | .branch pfx terminal children =>
        let suffix := query.drop path.length
        if (common pfx suffix).length < pfx.length then
          if mismatchGreater pfx suffix != reverse then [.node (.branch pfx terminal children)] else []
        else
          match suffix.drop pfx.length with
          | [] => endpointTasks inclusive reverse terminal children
          | byte :: _ => seekForest (path ++ pfx) query inclusive reverse children byte ++
              (if reverse then terminalTasks terminal else [])
  def seekForest (path query : Key) (inclusive reverse : Bool) : Forest → UInt8 → List CursorTask
    | .nil, _ => []
    | .cons byte child tail, target =>
        if target = byte then
          seekTree (path ++ [byte]) query inclusive reverse child ++ (if reverse then [] else forestTasks tail)
        else if target < byte then
          if reverse then [] else forestTasks (.cons byte child tail)
        else
          seekForest path query inclusive reverse tail target ++ (if reverse then [.node child] else [])
end

mutual
  theorem seek_tree_rows (path query : Key) (inclusive reverse : Bool) (tree : Tree)
      (valid : Valid path tree) (ordered : OrderedTree tree) (queryValid : HasPrefix path query) :
      pendingRows reverse (seekTree path query inclusive reverse tree) =
        orient reverse ((entries tree).filter (accepts query inclusive reverse)) := by
    cases tree with
    | leaf entry =>
        cases kept : accepts query inclusive reverse entry <;>
          cases reverse <;> simp [seekTree, kept, entries, pendingRows, taskRows, orient]
    | branch pfx terminal children =>
        by_cases proper : (common pfx (query.drop path.length)).length < pfx.length
        · have constant : ∀ entry ∈ entries (.branch pfx terminal children),
              accepts query inclusive reverse entry = (mismatchGreater pfx (query.drop path.length) != reverse) := by
            intro entry member
            exact mismatch_accepts path pfx query inclusive reverse entry queryValid
              (valid_branch_prefix _ _ _ _ valid entry member) proper
          dsimp only [seekTree]
          rw [if_pos proper, filter_constant _ _ _ constant]
          cases flag : (mismatchGreater pfx (query.drop path.length) != reverse) <;>
            simp [pendingRows, taskRows, orient]
        · have queryKey := matching_key_decomposition path pfx query queryValid proper
          dsimp only [seekTree]
          rw [if_neg proper]
          generalize suffixEq : (query.drop path.length).drop pfx.length = suffix
          rw [suffixEq] at queryKey
          cases suffix with
          | nil =>
              have queryEnd : query = path ++ pfx := by simpa using queryKey
              have terminals : ∀ entry ∈ terminal.toList, accepts query inclusive reverse entry = inclusive := by
                intro entry member
                have someEntry : terminal = some entry := by simpa using member
                exact accepts_equal query inclusive reverse entry ((valid.1 entry someEntry).trans queryEnd.symm)
              have descendants : ∀ entry ∈ forestEntries children, accepts query inclusive reverse entry = !reverse := by
                intro entry member
                apply accepts_greater
                rw [queryEnd]
                exact forest_terminal_before _ _ valid.2 entry member
              rw [entries, List.filter_append, filter_constant _ _ _ terminals, filter_constant _ _ _ descendants]
              cases reverse <;> cases inclusive <;> cases terminal <;>
                simp [endpointTasks, pending_rows_append, terminal_tasks_rows, forest_tasks_rows,
                  pendingRows, orient]
          | cons byte rest =>
              have childQuery : HasPrefix ((path ++ pfx) ++ [byte]) query :=
                ⟨rest, by simpa [List.append_assoc] using queryKey⟩
              have terminals : ∀ entry ∈ terminal.toList, accepts query inclusive reverse entry = reverse := by
                simpa only [← queryKey] using
                  terminal_accepts_before (path ++ pfx) rest byte terminal inclusive reverse valid.1
              rw [pending_rows_append,
                seek_forest_rows (path ++ pfx) query inclusive reverse children byte valid.2 ordered childQuery,
                entries, List.filter_append, filter_constant _ _ _ terminals]
              cases reverse <;> cases terminal <;> simp [pendingRows, terminal_tasks_rows, orient]
  theorem seek_forest_rows (path query : Key) (inclusive reverse : Bool) (forest : Forest) (target : UInt8)
      (valid : ValidForest path forest) (ordered : OrderedForest forest)
      (queryValid : HasPrefix (path ++ [target]) query) :
      pendingRows reverse (seekForest path query inclusive reverse forest target) =
        orient reverse ((forestEntries forest).filter (accepts query inclusive reverse)) := by
    cases forest with
    | nil => cases reverse <;> rfl
    | cons byte child tail =>
        obtain ⟨suffix, queryKey⟩ := queryValid
        have queryEq : query = path ++ target :: suffix := by simpa [List.append_assoc] using queryKey
        by_cases same : target = byte
        · subst target
          have later : ∀ entry ∈ forestEntries tail, accepts query inclusive reverse entry = !reverse := by
            simpa only [← queryEq] using
              forest_accepts_after path suffix byte tail inclusive reverse valid.2.1 ordered.2.2
          rw [seekForest, if_pos rfl, pending_rows_append,
            seek_tree_rows (path ++ [byte]) query inclusive reverse child valid.1 ordered.1 ⟨suffix, queryKey⟩,
            forestEntries, List.filter_append, filter_constant _ _ _ later]
          cases reverse <;> simp [pendingRows, forest_tasks_rows, orient]
        · by_cases before : target < byte
          · have above : Above target (.cons byte child tail) :=
              ⟨before, above_trans target byte tail before ordered.2.2⟩
            have allLater : ∀ entry ∈ forestEntries (.cons byte child tail),
                accepts query inclusive reverse entry = !reverse := by
              simpa only [← queryEq] using
                forest_accepts_after path suffix target (.cons byte child tail) inclusive reverse valid above
            rw [seekForest, if_neg same, if_pos before, filter_constant _ _ _ allLater]
            cases reverse <;> simp [pendingRows, forest_tasks_rows, orient]
          · have earlier : ∀ entry ∈ entries child, accepts query inclusive reverse entry = reverse := by
              simpa only [← queryEq] using tree_accepts_before path suffix byte target child inclusive reverse
                valid.1 (byte_reverse_lt target byte same before)
            rw [seekForest, if_neg same, if_neg before, pending_rows_append,
              seek_forest_rows path query inclusive reverse tail target valid.2.1 ordered.2.1 ⟨suffix, queryKey⟩,
              forestEntries, List.filter_append, filter_constant _ _ _ earlier]
            cases reverse <;> simp [pendingRows, taskRows, orient]
end

def seekRoot (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool) : List CursorTask :=
  match root, bound with
  | none, _ => []
  | some tree, none => [.node tree]
  | some tree, some (query, inclusive) => seekTree [] query inclusive reverse tree

def boundRows (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool) : List Entry :=
  orient reverse (match bound with
    | none => optionalEntries root
    | some (query, inclusive) => (optionalEntries root).filter (accepts query inclusive reverse))

theorem seek_root_rows (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool) (good : Good root) :
    pendingRows reverse (seekRoot root bound reverse) = boundRows root bound reverse := by
  cases root with
  | none => cases bound <;> cases reverse <;> rfl
  | some tree =>
      cases bound with
      | none => simp [seekRoot, boundRows, pendingRows, taskRows, optionalEntries]
      | some pair =>
          obtain ⟨query, inclusive⟩ := pair
          exact seek_tree_rows [] query inclusive reverse tree good.1 (good.2 tree rfl).2 ⟨query, rfl⟩

theorem seek_next_yield (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool) (good : Good root)
    (entry : Entry) (tail : List CursorTask) (yielded : nextTask reverse (seekRoot root bound reverse) = some (entry, tail)) :
    boundRows root bound reverse = entry :: pendingRows reverse tail := by
  rw [← seek_root_rows root bound reverse good]
  exact next_task_yield reverse _ tail entry yielded

theorem seek_next_empty (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool) (good : Good root)
    (stopped : nextTask reverse (seekRoot root bound reverse) = none) : boundRows root bound reverse = [] := by
  rw [← seek_root_rows root bound reverse good]
  exact next_task_empty reverse _ stopped

end Kv9.Radix
