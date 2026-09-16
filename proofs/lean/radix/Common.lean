import Tree

set_option autoImplicit false

namespace Kv9.Radix

def common : Key → Key → Key
  | a :: as, b :: bs => if a = b then a :: common as bs else []
  | _, _ => []

theorem common_left (a b : Key) : HasPrefix (common a b) a := by
  induction a generalizing b with
  | nil => exact ⟨[], rfl⟩
  | cons head tail ih =>
      cases b with
      | nil => exact ⟨head :: tail, rfl⟩
      | cons other rest =>
          by_cases same : head = other
          · obtain ⟨suffix, hs⟩ := ih rest
            refine ⟨suffix, ?_⟩
            simp only [common, if_pos same, List.cons_append]
            exact congrArg (List.cons head) hs
          · exact ⟨head :: tail, by simp [common, same]⟩

theorem common_right (a b : Key) : HasPrefix (common a b) b := by
  induction a generalizing b with
  | nil => exact ⟨b, rfl⟩
  | cons head tail ih =>
      cases b with
      | nil => exact ⟨[], rfl⟩
      | cons other rest =>
          by_cases same : head = other
          · obtain ⟨suffix, hs⟩ := ih rest
            refine ⟨suffix, ?_⟩
            simp only [common, if_pos same, List.cons_append]
            rw [same]
            exact congrArg (List.cons other) hs
          · exact ⟨other :: rest, by simp [common, same]⟩

theorem prefix_drop (pfx key : Key) (valid : HasPrefix pfx key) :
    key = pfx ++ key.drop pfx.length := by
  obtain ⟨suffix, rfl⟩ := valid
  simp

theorem prefix_take (pfx key : Key) (valid : HasPrefix pfx key) : key.take pfx.length = pfx := by
  obtain ⟨suffix, rfl⟩ := valid
  simp

theorem common_decompose_left (a b : Key) :
    a = common a b ++ a.drop (common a b).length := prefix_drop _ _ (common_left a b)

theorem common_decompose_right (a b : Key) :
    b = common a b ++ b.drop (common a b).length := prefix_drop _ _ (common_right a b)

theorem common_bounds (a b : Key) : (common a b).length ≤ a.length ∧ (common a b).length ≤ b.length := by
  obtain ⟨left, hl⟩ := common_left a b
  obtain ⟨right, hr⟩ := common_right a b
  have leftLength := congrArg List.length hl
  have rightLength := congrArg List.length hr
  simp only [List.length_append] at leftLength rightLength
  omega

-- Exactly the zip/take-while/count computation used by Rust common_prefix.
theorem common_matches_zip_count (a b : Key) :
    (common a b).length = ((a.zip b).takeWhile (fun pair => pair.1 == pair.2)).length := by
  induction a generalizing b with
  | nil => simp [common]
  | cons head tail ih =>
      cases b with
      | nil => simp [common]
      | cons other rest =>
          by_cases same : head = other
          · subst other
            simpa [common] using congrArg Nat.succ (ih rest)
          · simp [common, same]

def Separated : Key → Key → Prop
  | a :: _, b :: _ => a ≠ b
  | _, _ => True

theorem common_separates (a b : Key) :
    Separated (a.drop (common a b).length) (b.drop (common a b).length) := by
  induction a generalizing b with
  | nil => simp [common, Separated]
  | cons head tail ih =>
      cases b with
      | nil => simp [common, Separated]
      | cons other rest =>
          by_cases same : head = other
          · subst other
            simpa [common] using ih rest
          · simp [common, Separated, same]

theorem common_full_left (a b : Key) (full : (common a b).length = a.length) : common a b = a := by
  have taken := prefix_take _ _ (common_left a b)
  simpa [full] using taken.symm

theorem common_exhausted_both (a b : Key)
    (left : a.drop (common a b).length = []) (right : b.drop (common a b).length = []) : a = b := by
  have ha := common_decompose_left a b
  have hb := common_decompose_right a b
  simp only [left, List.append_nil] at ha
  simp only [right, List.append_nil] at hb
  exact ha.trans hb.symm

theorem full_key_from_suffix (path key suffix : Key) (valid : HasPrefix path key)
    (remaining : key.drop path.length = suffix) : key = path ++ suffix := by
  rw [prefix_drop path key valid, remaining]

theorem common_prefix_append (pfx suffix : Key) : common pfx (pfx ++ suffix) = pfx := by
  induction pfx with
  | nil => rfl
  | cons byte tail ih => simp [common, ih]

theorem proper_common_excludes_prefix (pfx key : Key)
    (proper : (common pfx key).length < pfx.length) : ¬ HasPrefix pfx key := by
  rintro ⟨suffix, eq⟩
  simp [eq, common_prefix_append] at proper

end Kv9.Radix
