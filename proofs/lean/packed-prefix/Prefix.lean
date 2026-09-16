import Std

namespace Kv9.PackedPrefix

-- Natural elements generalize unsigned bytes; the Rust mapping specializes to 0..255.
abbrev Key := List Nat

def cmp : Key → Key → Ordering
  | [], [] => .eq
  | [], _ :: _ => .lt
  | _ :: _, [] => .gt
  | a :: as, b :: bs => if a = b then cmp as bs else if a < b then .lt else .gt

def Prefix (p key : Key) : Prop := ∃ suffix, key = p ++ suffix

def common : Key → Key → Key
  | a :: as, b :: bs => if a = b then a :: common as bs else []
  | _, _ => []

theorem shared_prefix_order (p a b : Key) : cmp (p ++ a) (p ++ b) = cmp a b := by
  induction p with
  | nil => rfl
  | cons head tail ih => simp [cmp, ih]

theorem common_left (a b : Key) : Prefix (common a b) a := by
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

theorem common_right (a b : Key) : Prefix (common a b) b := by
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

theorem prefix_trans (p q key : Key) (hp : Prefix p q) (hq : Prefix q key) : Prefix p key := by
  obtain ⟨a, rfl⟩ := hp
  obtain ⟨b, rfl⟩ := hq
  exact ⟨a ++ b, by simp [List.append_assoc]⟩

theorem common_shortens_existing (p fresh key : Key) (h : Prefix p key) :
    Prefix (common p fresh) key :=
  prefix_trans _ _ _ (common_left p fresh) h

theorem common_admits_new (p fresh : Key) : Prefix (common p fresh) fresh :=
  common_right p fresh

-- Every key in an ordered interval with a common prefix has that same prefix.
theorem prefix_interval (p left middle right : Key)
    (lower : cmp (p ++ left) middle ≠ .gt)
    (upper : cmp middle (p ++ right) ≠ .gt) : Prefix p middle := by
  induction p generalizing middle with
  | nil => exact ⟨middle, rfl⟩
  | cons head tail ih =>
      cases middle with
      | nil => simp [cmp] at lower
      | cons other rest =>
          by_cases same : head = other
          · subst other
            have hl : cmp (tail ++ left) rest ≠ .gt := by simpa [cmp] using lower
            have hu : cmp rest (tail ++ right) ≠ .gt := by simpa [cmp] using upper
            obtain ⟨suffix, hs⟩ := ih rest hl hu
            exact ⟨suffix, by simp [hs]⟩
          · have reverse : other ≠ head := Ne.symm same
            have hl : head < other := by
              by_cases hlt : head < other
              · exact hlt
              · simp [cmp, same, hlt] at lower
            have hu : other < head := by
              by_cases hlt : other < head
              · exact hlt
              · simp [cmp, reverse, hlt] at upper
            omega

theorem endpoint_prefix_covers_interval (first key last : Key)
    (lower : cmp first key ≠ .gt) (upper : cmp key last ≠ .gt) :
    Prefix (common first last) key := by
  obtain ⟨left, hl⟩ := common_left first last
  obtain ⟨right, hr⟩ := common_right first last
  apply prefix_interval (common first last) left key right
  · simpa [← hl] using lower
  · simpa [← hr] using upper

theorem prefix_bound (p key : Key) (h : Prefix p key) : p.length ≤ key.length := by
  obtain ⟨suffix, rfl⟩ := h
  simp

theorem prefix_take (p key : Key) (h : Prefix p key) : key.take p.length = p := by
  obtain ⟨suffix, rfl⟩ := h
  simp

theorem prefix_drop (p key : Key) (h : Prefix p key) : key = p ++ key.drop p.length := by
  obtain ⟨suffix, rfl⟩ := h
  simp

theorem drop_shared_prefix (p a b : Key) (ha : Prefix p a) (hb : Prefix p b) :
    cmp (a.drop p.length) (b.drop p.length) = cmp a b := by
  rw [prefix_drop p a ha, prefix_drop p b hb]
  simp [shared_prefix_order]

-- A checked query may use the cached prefix, or take the unmodified full-key path.
def guarded (p stored query : Key) : Ordering :=
  if p.length ≤ query.length ∧ query.take p.length = p then cmp (stored.drop p.length) (query.drop p.length)
  else cmp stored query

theorem guarded_order (p stored query : Key) (stored_prefix : Prefix p stored) :
    guarded p stored query = cmp stored query := by
  unfold guarded
  split
  · next h =>
      have query_prefix : Prefix p query := ⟨query.drop p.length, by
        calc
          query = query.take p.length ++ query.drop p.length := (List.take_append_drop _ _).symm
          _ = p ++ query.drop p.length := by rw [h.2]⟩
      exact drop_shared_prefix p stored query stored_prefix query_prefix
  · rfl

theorem subset_preserves_prefix (p : Key) (before after : List Key)
    (all_before : ∀ key ∈ before, Prefix p key)
    (subset : ∀ key ∈ after, key ∈ before) : ∀ key ∈ after, Prefix p key := by
  intro key hk
  exact all_before key (subset key hk)

theorem insertion_preserves_shortened_prefix (p fresh : Key) (before : List Key)
    (all_before : ∀ key ∈ before, Prefix p key) :
    ∀ key ∈ fresh :: before, Prefix (common p fresh) key := by
  intro key hk
  simp only [List.mem_cons] at hk
  rcases hk with added | old
  · rw [added]
    exact common_admits_new p fresh
  · exact common_shortens_existing p fresh key (all_before key old)

theorem endpoints_cover_node (first last : Key) (keys : List Key)
    (bounded : ∀ key ∈ keys, cmp first key ≠ .gt ∧ cmp key last ≠ .gt) :
    ∀ key ∈ keys, Prefix (common first last) key := by
  intro key hk
  exact endpoint_prefix_covers_interval first key last (bounded key hk).1 (bounded key hk).2

theorem checked_slices_in_bounds (p stored query : Key)
    (hs : Prefix p stored) (hq : Prefix p query) :
    p.length ≤ stored.length ∧ p.length ≤ query.length :=
  ⟨prefix_bound p stored hs, prefix_bound p query hq⟩

theorem empty_prefix_fallback (stored query : Key) :
    cmp (stored.drop 0) (query.drop 0) = cmp stored query := by simp

end Kv9.PackedPrefix
