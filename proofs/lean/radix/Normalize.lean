import Tree

set_option autoImplicit false

namespace Kv9.Radix

-- One normalization step, matching RadixMap::normalize. A leaf already holds
-- its full key; only a branch child's relative prefix needs to be prepended.
def normalize : Tree → Option Tree
  | .leaf entry => some (.leaf entry)
  | .branch _ none .nil => none
  | .branch _ (some entry) .nil => some (.leaf entry)
  | .branch pfx none (.cons byte child .nil) =>
      match child with
      | .leaf entry => some (.leaf entry)
      | .branch childPrefix terminal children =>
          some (.branch (pfx ++ byte :: childPrefix) terminal children)
  | tree => some tree

def optionalEntries : Option Tree → List Entry
  | none => []
  | some tree => entries tree

def ValidOptional (path : Key) : Option Tree → Prop
  | none => True
  | some tree => Valid path tree

theorem normalize_entries (tree : Tree) : optionalEntries (normalize tree) = entries tree := by
  cases tree with
  | leaf entry => rfl
  | branch pfx terminal children =>
      cases children with
      | nil => cases terminal <;> rfl
      | cons byte child tail =>
          cases terminal with
          | some entry => rfl
          | none =>
              cases tail with
              | nil => cases child <;> simp [normalize, optionalEntries, entries, forestEntries]
              | cons nextByte nextChild nextTail => rfl

theorem normalize_valid (path : Key) (tree : Tree) (valid : Valid path tree) :
    ValidOptional path (normalize tree) := by
  cases tree with
  | leaf entry => exact valid
  | branch pfx terminal children =>
      obtain ⟨terminalValid, childrenValid⟩ := valid
      cases children with
      | nil =>
          cases terminal with
          | none => trivial
          | some entry => exact ⟨pfx, terminalValid entry rfl⟩
      | cons byte child tail =>
          cases terminal with
          | some entry => exact ⟨terminalValid, childrenValid⟩
          | none =>
              cases tail with
              | cons nextByte nextChild nextTail => exact ⟨terminalValid, childrenValid⟩
              | nil =>
                  obtain ⟨childValid, _, _⟩ := childrenValid
                  cases child with
                  | leaf entry =>
                      exact prefix_trans path ((path ++ pfx) ++ [byte]) entry.key
                        ⟨pfx ++ [byte], by simp [List.append_assoc]⟩ childValid
                  | branch childPrefix childTerminal grandchildren =>
                      simpa [normalize, ValidOptional, Valid, List.append_assoc] using childValid

def optionalLookup (query : Key) : Option Tree → Option Value
  | none => none
  | some tree => treeLookup query tree query

theorem normalized_root_lookup (tree : Tree) (query : Key) (valid : Valid [] tree) :
    optionalLookup query (normalize tree) = linearLookup query (entries tree) := by
  have retained := normalize_entries tree
  have validResult := normalize_valid [] tree valid
  cases h : normalize tree with
  | none =>
      simp [h, optionalEntries] at retained
      simp [optionalLookup, retained, linearLookup]
  | some result =>
      simp only [h, optionalEntries] at retained
      simp only [h, ValidOptional] at validResult
      rw [optionalLookup, root_lookup_refines result query validResult, retained]

-- Canonicality is separate from routing validity: transient deletion parents
-- may have one alternative, but already-normalized children must be canonical.
def alternatives (terminal : Option Entry) : Forest → Nat
  | .nil => terminal.toList.length
  | .cons _ _ tail => 1 + alternatives terminal tail

mutual
  def Canonical : Tree → Prop
    | .leaf _ => True
    | .branch _ terminal children => 2 ≤ alternatives terminal children ∧ CanonicalForest children
  def CanonicalForest : Forest → Prop
    | .nil => True
    | .cons _ child tail => Canonical child ∧ CanonicalForest tail
end

def ChildrenCanonical : Tree → Prop
  | .leaf _ => True
  | .branch _ _ children => CanonicalForest children

theorem normalize_canonical (tree : Tree) (children : ChildrenCanonical tree) :
    ∀ result, normalize tree = some result → Canonical result := by
  cases tree with
  | leaf entry =>
      intro result eq
      simp only [normalize, Option.some.injEq] at eq
      subst result
      trivial
  | branch pfx terminal forest =>
      cases forest with
      | nil =>
          cases terminal with
          | none => simp [normalize]
          | some entry =>
              intro result eq
              simp only [normalize, Option.some.injEq] at eq
              subst result
              trivial
      | cons byte child tail =>
          cases terminal with
          | some entry =>
              intro result eq
              simp only [normalize, Option.some.injEq] at eq
              subst result
              refine ⟨?_, children⟩
              have lower : 1 ≤ alternatives (some entry) tail := by
                cases tail <;> simp [alternatives] <;> omega
              simp only [alternatives]
              omega
          | none =>
              cases tail with
              | nil =>
                  cases child with
                  | leaf entry =>
                      intro result eq
                      simp only [normalize, Option.some.injEq] at eq
                      subst result
                      trivial
                  | branch cp ct cf =>
                      intro result eq
                      simp only [normalize, Option.some.injEq] at eq
                      subst result
                      exact children.1
              | cons nextByte nextChild nextTail =>
                  intro result eq
                  simp only [normalize, Option.some.injEq] at eq
                  subst result
                  refine ⟨?_, children⟩
                  simp only [alternatives]
                  omega

end Kv9.Radix
