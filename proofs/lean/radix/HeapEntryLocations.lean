import HeapDeleteRoot

set_option autoImplicit false

namespace Kv9.Radix

-- Each occupied Node contains at most one resident Entry: either a Leaf
-- payload or a Branch terminal. These are object locations, not Vec buffers.
def storedEntry : StoredNode → Option Entry
  | .leaf entry => some entry
  | .branch _ terminal _ => terminal

def heapEntry (heap : NodeHeap) (address : NodeId) : Option Entry :=
  (heapRead heap address).bind storedEntry

def EntryLocations (heap : NodeHeap) (locations : List (NodeId × Entry)) : Prop :=
  ∀ location ∈ locations, heapEntry heap location.1 = some location.2

mutual
  theorem node_rep_entry_locations (heap : NodeHeap) (address : NodeId) (tree : Tree)
      (rep : NodeRep heap address tree) :
      ∃ locations, locations.map Prod.snd = entries tree ∧ EntryLocations heap locations := by
    cases rep with
    | leaf _ entry read =>
        refine ⟨[(address, entry)], rfl, ?_⟩
        intro location member
        have same := List.mem_singleton.mp member
        simp [same, heapEntry, read, storedEntry]
    | branch _ pfx terminal edges children read descendants =>
        obtain ⟨locations, values, resident⟩ := edges_rep_entry_locations heap edges children descendants
        cases terminal with
        | none => exact ⟨locations, values, resident⟩
        | some entry =>
            refine ⟨(address, entry) :: locations, by simp [values, entries], ?_⟩
            intro location member
            rcases List.mem_cons.mp member with first | later
            · simp [first, heapEntry, read, storedEntry]
            · exact resident location later
  theorem edges_rep_entry_locations (heap : NodeHeap) (edges : List StoredEdge) (children : Forest)
      (rep : EdgesRep heap edges children) :
      ∃ locations, locations.map Prod.snd = forestEntries children ∧ EntryLocations heap locations := by
    cases rep with
    | nil => exact ⟨[], rfl, by intro location member; cases member⟩
    | cons byte address edges child tail node rest =>
        obtain ⟨childLocations, childValues, childResident⟩ := node_rep_entry_locations heap address child node
        obtain ⟨tailLocations, tailValues, tailResident⟩ := edges_rep_entry_locations heap edges tail rest
        refine ⟨childLocations ++ tailLocations, by simp [childValues, tailValues, forestEntries], ?_⟩
        intro location member
        rcases List.mem_append.mp member with first | later
        · exact childResident location first
        · exact tailResident location later
end

-- Shared snapshots may refer to the same resident Entry. Within one valid
-- map, unique full keys imply that its unfolding visits no Entry location
-- twice. A graph identity alone still gives no native address-space bound.
theorem entry_locations_unique (heap : NodeHeap) (locations : List (NodeId × Entry))
    (resident : EntryLocations heap locations) (unique : UniqueKeys (locations.map Prod.snd)) :
    (locations.map Prod.fst).Nodup := by
  induction locations with
  | nil => simp
  | cons location tail ih =>
      rcases location with ⟨address, entry⟩
      have distinct : entry.key ∉ (tail.map Prod.snd).map Entry.key ∧ UniqueKeys (tail.map Prod.snd) := by
        simpa only [UniqueKeys, List.map_cons, List.nodup_cons] using unique
      refine List.nodup_cons.mpr ⟨?_, ih (fun item member => resident item (by simp [member])) distinct.2⟩
      intro member
      obtain ⟨other, otherMember, same⟩ := List.mem_map.mp member
      have first := resident (address, entry) (by simp)
      have second := resident other (by simp [otherMember])
      rw [same] at second
      have entrySame := Option.some.inj (first.symm.trans second)
      exact distinct.1 (List.mem_map.mpr ⟨other.2, List.mem_map.mpr ⟨other, otherMember, rfl⟩,
        congrArg Entry.key entrySame.symm⟩)

theorem valid_root_entry_locations (heap : NodeHeap) (root : Option NodeId) (tree : Option Tree)
    (rep : RootRep heap root tree) (valid : ValidOptional [] tree) :
    ∃ locations, locations.map Prod.snd = optionalEntries tree ∧ EntryLocations heap locations ∧
      (locations.map Prod.fst).Nodup := by
  cases root with
  | none => cases tree with
    | none =>
        refine ⟨[], rfl, ?_, by simp⟩
        intro location member
        cases member
    | some value => cases rep
  | some address => cases tree with
    | none => cases rep
    | some value =>
        obtain ⟨locations, values, resident⟩ := node_rep_entry_locations heap address value rep
        exact ⟨locations, values, resident, entry_locations_unique heap locations resident
          (by rw [values]; exact valid_tree_unique [] value valid)⟩

end Kv9.Radix
