import PendingVector

set_option autoImplicit false

namespace Kv9.Radix

theorem seek_forest_indexed_hit (path query : Key) (inclusive reverse : Bool) (forest : Forest)
    (target : UInt8) (index : Nat) (child : Tree) (ordered : OrderedForest forest)
    (found : edgeGet index forest = some (target, child)) :
    seekForest path query inclusive reverse forest target =
      seekTree (path ++ [target]) query inclusive reverse child ++
        (if reverse then (forestTasks (edgeTake index forest)).reverse else forestTasks (edgeDrop (index + 1) forest)) := by
  induction index generalizing forest with
  | zero =>
      cases forest with
      | nil => simp [edgeGet] at found
      | cons byte node tail =>
          have same : byte = target ∧ node = child := by simpa [edgeGet] using found
          rcases same with ⟨rfl, rfl⟩
          cases reverse <;> simp [seekForest, edgeTake, edgeDrop, forestTasks]
  | succ n ih =>
      cases forest with
      | nil => simp [edgeGet] at found
      | cons byte node tail =>
          have less := above_get byte tail n target child ordered.2.2 found
          obtain ⟨different, notBefore⟩ := byte_less_excludes byte target less
          rw [seekForest, if_neg different, if_neg notBefore, ih tail ordered.2.1 found]
          cases reverse <;> simp [edgeTake, edgeDrop, forestTasks, List.append_assoc]

theorem seek_forest_indexed_miss (path query : Key) (inclusive reverse : Bool) (forest : Forest)
    (target : UInt8) (index : Nat) (miss : edgeSearch target forest = .miss index) :
    seekForest path query inclusive reverse forest target =
      (if reverse then (forestTasks (edgeTake index forest)).reverse else forestTasks (edgeDrop index forest)) := by
  cases forest with
  | nil =>
      have eq : index = 0 := by simpa [edgeSearch] using miss.symm
      subst index
      cases reverse <;> rfl
  | cons byte child tail =>
      by_cases same : target = byte
      · simp [edgeSearch, same] at miss
      · by_cases before : target < byte
        · have eq : index = 0 := by simpa [edgeSearch, same, before] using miss.symm
          subst index
          cases reverse <;> simp [seekForest, same, before, edgeTake, edgeDrop, forestTasks]
        · cases search : edgeSearch target tail with
          | hit n => simp [edgeSearch, same, before, search, bumpSearch] at miss
          | miss n =>
              have eq : index = n + 1 := by simpa [edgeSearch, same, before, search, bumpSearch] using miss.symm
              subst index
              rw [seekForest, if_neg same, if_neg before, seek_forest_indexed_miss path query inclusive reverse tail target n search]
              cases reverse <;> simp [edgeTake, edgeDrop, forestTasks]

def endpointPush (inclusive reverse : Bool) (terminal : Option Entry) (children : Forest) : List CursorTask :=
  (if reverse then [] else (forestTasks children).reverse) ++ (if inclusive then terminalTasks terminal else [])

def seekPush (reverse : Bool) (terminal : Option Entry) (children : Forest) (index : Nat) (found : Bool) : List CursorTask :=
  if reverse then terminalTasks terminal ++ forestTasks (edgeTake index children)
  else (forestTasks (edgeDrop (index + (if found then 1 else 0)) children)).reverse

theorem endpoint_push_reverse (inclusive reverse : Bool) (terminal : Option Entry) (children : Forest) :
    (endpointPush inclusive reverse terminal children).reverse = endpointTasks inclusive reverse terminal children := by
  cases inclusive <;> cases reverse <;> cases terminal <;> simp [endpointPush, endpointTasks, terminalTasks]

theorem seek_push_reverse (reverse : Bool) (terminal : Option Entry) (children : Forest) (index : Nat) (found : Bool) :
    (seekPush reverse terminal children index found).reverse =
      (if reverse then (forestTasks (edgeTake index children)).reverse
        else forestTasks (edgeDrop (index + (if found then 1 else 0)) children)) ++
          (if reverse then terminalTasks terminal else []) := by
  cases reverse <;> cases terminal <;> simp [seekPush, terminalTasks]

-- This records the source's prefix[common] and suffix.get(common) accesses.
def mismatchAt (pfx suffix : Key) : Option Bool :=
  let sharedLength := (common pfx suffix).length
  match pfx[sharedLength]? with
  | none => none
  | some oldByte => some (match suffix[sharedLength]? with
    | none => true
    | some byte => decide (byte < oldByte))

theorem mismatch_at_refines (pfx suffix : Key) (proper : (common pfx suffix).length < pfx.length) :
    mismatchAt pfx suffix = some (mismatchGreater pfx suffix) := by
  dsimp only [mismatchAt, mismatchGreater]
  rw [← drop_head_access, ← drop_head_access]
  cases old : pfx.drop (common pfx suffix).length with
  | nil =>
      have bound := List.drop_eq_nil_iff.mp old
      omega
  | cons byte rest => cases suffix.drop (common pfx suffix).length <;> rfl

end Kv9.Radix
