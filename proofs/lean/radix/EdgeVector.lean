import RangeProperties

set_option autoImplicit false

namespace Kv9.Radix

-- Forest is the contents of the Rust Vec<Edge>, preserving index order.
-- These definitions model ordinary Vec/slice contracts, not their allocator.
def edgeRows : Forest → List (UInt8 × Tree)
  | .nil => []
  | .cons byte child tail => (byte, child) :: edgeRows tail

def edgeForest : List (UInt8 × Tree) → Forest
  | [] => .nil
  | (byte, child) :: tail => .cons byte child (edgeForest tail)

theorem edge_rows_forest (rows : List (UInt8 × Tree)) : edgeRows (edgeForest rows) = rows := by
  induction rows with
  | nil => rfl
  | cons pair tail ih => cases pair; simp [edgeForest, edgeRows, ih]

theorem edge_forest_rows (forest : Forest) : edgeForest (edgeRows forest) = forest := by
  cases forest with
  | nil => rfl
  | cons byte child tail => simp [edgeForest, edgeRows, edge_forest_rows tail]

theorem edge_rows_length (forest : Forest) : (edgeRows forest).length = edgeCount forest := by
  cases forest with
  | nil => rfl
  | cons byte child tail => simp [edgeRows, edgeCount, edge_rows_length tail]

def edgeAppend : Forest → Forest → Forest
  | .nil, right => right
  | .cons byte child tail, right => .cons byte child (edgeAppend tail right)

def edgeTake : Nat → Forest → Forest
  | 0, _ => .nil
  | _ + 1, .nil => .nil
  | n + 1, .cons byte child tail => .cons byte child (edgeTake n tail)

def edgeDrop : Nat → Forest → Forest
  | 0, forest => forest
  | _ + 1, .nil => .nil
  | n + 1, .cons _ _ tail => edgeDrop n tail

def edgeGet : Nat → Forest → Option (UInt8 × Tree)
  | _, .nil => none
  | 0, .cons byte child _ => some (byte, child)
  | n + 1, .cons _ _ tail => edgeGet n tail

def edgeInsert : Nat → UInt8 → Tree → Forest → Forest
  | 0, byte, child, forest => .cons byte child forest
  | _ + 1, byte, child, .nil => .cons byte child .nil -- unreachable for Vec::insert
  | n + 1, byte, child, .cons oldByte oldChild tail => .cons oldByte oldChild (edgeInsert n byte child tail)

def edgeRemove : Nat → Forest → Forest
  | _, .nil => .nil -- unreachable after successful indexed access
  | 0, .cons _ _ tail => tail
  | n + 1, .cons byte child tail => .cons byte child (edgeRemove n tail)

def edgeSet : Nat → Tree → Forest → Forest
  | _, _, .nil => .nil -- unreachable after successful indexed access
  | 0, child, .cons byte _ tail => .cons byte child tail
  | n + 1, child, .cons byte old tail => .cons byte old (edgeSet n child tail)

theorem edge_rows_append (a b : Forest) : edgeRows (edgeAppend a b) = edgeRows a ++ edgeRows b := by
  cases a with
  | nil => rfl
  | cons byte child tail => simp [edgeAppend, edgeRows, edge_rows_append tail b]

theorem edge_rows_take (index : Nat) (forest : Forest) : edgeRows (edgeTake index forest) = (edgeRows forest).take index := by
  induction index generalizing forest with
  | zero => rfl
  | succ n ih => cases forest <;> simp [edgeTake, edgeRows, ih]

theorem edge_rows_drop (index : Nat) (forest : Forest) : edgeRows (edgeDrop index forest) = (edgeRows forest).drop index := by
  induction index generalizing forest with
  | zero => rfl
  | succ n ih => cases forest <;> simp [edgeDrop, edgeRows, ih]

theorem edge_rows_get (index : Nat) (forest : Forest) : edgeGet index forest = (edgeRows forest)[index]? := by
  induction index generalizing forest with
  | zero => cases forest <;> rfl
  | succ n ih => cases forest <;> simp [edgeGet, edgeRows, ih]

theorem edge_take_drop (index : Nat) (forest : Forest) : edgeAppend (edgeTake index forest) (edgeDrop index forest) = forest := by
  have rows : edgeRows (edgeAppend (edgeTake index forest) (edgeDrop index forest)) = edgeRows forest := by
    rw [edge_rows_append, edge_rows_take, edge_rows_drop, List.take_append_drop]
  simpa only [edge_forest_rows] using congrArg edgeForest rows

theorem edge_rows_insert (index : Nat) (byte : UInt8) (child : Tree) (forest : Forest) :
    edgeRows (edgeInsert index byte child forest) =
      (edgeRows forest).take index ++ (byte, child) :: (edgeRows forest).drop index := by
  induction index generalizing forest with
  | zero => rfl
  | succ n ih => cases forest <;> simp [edgeInsert, edgeRows, ih]

theorem edge_rows_remove (index : Nat) (forest : Forest) :
    edgeRows (edgeRemove index forest) = (edgeRows forest).take index ++ (edgeRows forest).drop (index + 1) := by
  induction index generalizing forest with
  | zero => cases forest <;> rfl
  | succ n ih => cases forest <;> simp [edgeRemove, edgeRows, ih]

theorem edge_get_bound (index : Nat) (forest : Forest) (byte : UInt8) (child : Tree)
    (found : edgeGet index forest = some (byte, child)) : index < edgeCount forest := by
  rw [edge_rows_get] at found
  simpa only [edge_rows_length] using (List.getElem?_eq_some_iff.mp found).1

theorem edge_remove_restore (index : Nat) (forest : Forest) (byte : UInt8) (child replacement : Tree)
    (found : edgeGet index forest = some (byte, child)) :
    edgeInsert index byte replacement (edgeRemove index forest) = edgeSet index replacement forest := by
  induction index generalizing forest with
  | zero => cases forest <;> simp_all [edgeGet, edgeRemove, edgeInsert, edgeSet]
  | succ n ih => cases forest <;> simp_all [edgeGet, edgeRemove, edgeInsert, edgeSet]

theorem edge_remove_length (index : Nat) (forest : Forest) (byte : UInt8) (child : Tree)
    (found : edgeGet index forest = some (byte, child)) : edgeCount (edgeRemove index forest) + 1 = edgeCount forest := by
  induction index generalizing forest with
  | zero => cases forest <;> simp_all [edgeGet, edgeRemove, edgeCount]
  | succ n ih => cases forest <;> simp_all [edgeGet, edgeRemove, edgeCount]

theorem edge_restore_index_safe (index : Nat) (forest : Forest) (byte : UInt8) (child : Tree)
    (found : edgeGet index forest = some (byte, child)) : index ≤ edgeCount (edgeRemove index forest) := by
  have bound := edge_get_bound index forest byte child found
  have removed := edge_remove_length index forest byte child found
  omega

theorem edge_get_valid (path : Key) (forest : Forest) (index : Nat) (byte : UInt8) (child : Tree)
    (valid : ValidForest path forest) (found : edgeGet index forest = some (byte, child)) : Valid (path ++ [byte]) child := by
  induction index generalizing forest with
  | zero => cases forest <;> simp_all [edgeGet, ValidForest]
  | succ n ih =>
      cases forest with
      | nil => simp [edgeGet] at found
      | cons _ _ tail => exact ih tail valid.2.1 found

theorem edge_get_ordered (forest : Forest) (index : Nat) (byte : UInt8) (child : Tree)
    (ordered : OrderedForest forest) (found : edgeGet index forest = some (byte, child)) : OrderedTree child := by
  induction index generalizing forest with
  | zero => cases forest <;> simp_all [edgeGet, OrderedForest]
  | succ n ih =>
      cases forest with
      | nil => simp [edgeGet] at found
      | cons _ _ tail => exact ih tail ordered.2.1 found

theorem edge_get_work (forest : Forest) (index : Nat) (byte : UInt8) (child : Tree)
    (found : edgeGet index forest = some (byte, child)) : treeWork child ≤ forestWork forest := by
  induction index generalizing forest with
  | zero => cases forest <;> simp_all [edgeGet, forestWork]
  | succ n ih =>
      cases forest with
      | nil => simp [edgeGet] at found
      | cons next node tail =>
          have smaller := ih tail found
          simp only [forestWork]
          omega

end Kv9.Radix
