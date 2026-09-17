//! The anchor-mapping kernel, verified (0045): `map_position` carries the
//! proof of its contract in-tree; `cargo verus verify` checks it, and the
//! normal build compiles the `verus!` block as plain Rust (ghost code
//! erases). This is the production function `editor/transact.rs` calls —
//! not a copy.
//!
//! Verified contract (Verus 0.2026.09.06, rustc 1.98.0, z3 4.16.0):
//! - anchors before the edit never move;
//! - anchors at/past the old end shift by the delta (right affinity);
//! - anchors inside the replaced range collapse to the edit start;
//! - the mapping is monotone and stays inside the new buffer length;
//! - batch composition (0057 VF02, the 0045 stretch obligation): folding
//!   the per-edit mappings over a validated batch in its reverse
//!   application order — exactly what `apply_prepared` publishes and
//!   `transact.rs` consumes — equals the direct whole-batch map over the
//!   original coordinates, and the composed mapping is monotone and in
//!   bounds.

use vstd::prelude::*;

verus! {

/// The mapping, mathematically: right-affinity anchor through one edit.
pub open spec fn map_spec(position: int, start: int, old_end: int, new_end: int) -> int {
    if position < start {
        position
    } else if position >= old_end {
        new_end + (position - old_end)
    } else {
        start
    }
}

/// One anchor through one edit. Callers guarantee no overflow: positions
/// are byte offsets bounded by the buffer length, and `new_end` is the
/// post-edit end of the replaced range (`start + replacement length`).
pub fn map_position(position: usize, start: usize, old_end: usize, new_end: usize) -> (mapped: usize)
    requires
        start <= old_end,
        position >= old_end ==> new_end + (position - old_end) <= usize::MAX,
    ensures
        mapped as int == map_spec(position as int, start as int, old_end as int, new_end as int),
{
    if position < start {
        position
    } else if position >= old_end {
        new_end.saturating_add(position - old_end)
    } else {
        start
    }
}

/// Monotonicity: anchors keep their relative order through one edit.
/// `start <= new_end` holds by construction (the post-edit end of a
/// replacement is its start plus the replacement length).
proof fn map_spec_monotone(a: int, b: int, start: int, old_end: int, new_end: int)
    requires
        start <= old_end,
        start <= new_end,
        a <= b,
    ensures
        map_spec(a, start, old_end, new_end) <= map_spec(b, start, old_end, new_end),
{
}

/// Bounds: an anchor inside the old buffer stays inside the new one.
proof fn map_spec_in_bounds(
    position: int,
    start: int,
    old_end: int,
    new_end: int,
    old_len: int,
    new_len: int,
)
    requires
        0 <= start <= old_end <= old_len,
        0 <= start <= new_end <= new_len,
        0 <= position <= old_len,
        new_len == old_len - (old_end - start) + (new_end - start),
    ensures
        0 <= map_spec(position, start, old_end, new_end) <= new_len,
{
}

}

verus! {

/// Seq of usize pairs as spec ints (Seq::map carries the indexing axiom).
pub open spec fn to_ints(edits: Seq<(usize, usize)>) -> Seq<(int, int)> {
    edits.map_values(|pair: (usize, usize)| (pair.0 as int, pair.1 as int))
}

/// The geometry contract a prepared batch meets (buffer/mutation.rs).
pub open spec fn batch_ok(edits: Seq<(int, int)>, len: int) -> bool {
    forall|i: int| #![trigger edits[i]] 0 <= i < edits.len() ==> {
        let (s, e) = edits[i];
        &&& 0 <= s <= e <= len
        &&& i + 1 < edits.len() ==> {
            let (s2, _) = edits[i + 1];
            &&& s < s2
            &&& e <= s2
        }
    }
}

/// Insert algebra for one insertion-sort step: inserting (i, pair) at the
/// walked position j keeps the index bookkeeping and consecutive sortedness.
proof fn insert_preserves(
    edits: Seq<(usize, usize)>,
    pre: Seq<(usize, (usize, usize))>,
    j: usize,
    i: usize,
    pair: (usize, usize),
)
    requires
        j as int <= pre.len(),
        pre.len() == i as int,
        (i as int) < edits.len(),
        pair == edits[i as int],
        forall|k: int| #![trigger pre[k]] 0 <= k < pre.len() ==> {
            &&& (pre[k].0 as int) < i as int
            &&& pre[k].1 == edits[pre[k].0 as int]
        },
        forall|a: int, b: int| #![trigger pre[a], pre[b]]
            0 <= a < b < pre.len() ==> pre[a].0 != pre[b].0,
        forall|k: int| #![trigger pre[k]] 0 <= k < pre.len() - 1 ==>
            pre[k].1.0 <= pre[k + 1].1.0,
        j > 0 ==> pre[j as int - 1].1.0 as int <= pair.0 as int,
        forall|p: int| #![trigger pre[p]] j as int <= p < pre.len() ==>
            pre[p].1.0 as int > pair.0 as int,
    ensures
        ({
            let post = pre.insert(j as int, (i, pair));
            &&& post.len() == i as int + 1
            &&& forall|k: int| #![trigger post[k]] 0 <= k < post.len() ==> {
                &&& (post[k].0 as int) < i as int + 1
                &&& post[k].1 == edits[post[k].0 as int]
            }
            &&& forall|a: int, b: int| #![trigger post[a], post[b]]
                0 <= a < b < post.len() ==> post[a].0 != post[b].0
            &&& forall|k: int| #![trigger post[k]] 0 <= k < post.len() - 1 ==>
                post[k].1.0 <= post[k + 1].1.0
        }),
{
    let post = pre.insert(j as int, (i, pair));
    assert forall|k: int| #![trigger post[k]] 0 <= k < post.len() implies {
        &&& (post[k].0 as int) < i as int + 1
        &&& post[k].1 == edits[post[k].0 as int]
    } by {
        if k == j as int {
            assert(post[k] == (i, pair));
        } else if k < j as int {
            assert(post[k] == pre[k]);
        } else {
            assert(post[k] == pre[k - 1]);
        }
    }
    assert forall|a: int, b: int| #![trigger post[a], post[b]]
        0 <= a < b < post.len() implies post[a].0 != post[b].0 by {
        if a == j as int {
            assert(post[b] == pre[b - 1]);
        } else if b == j as int {
            assert(post[a] == pre[a]);
        } else {
            assert(post[a] == if a < j as int { pre[a] } else { pre[a - 1] });
            assert(post[b] == if b < j as int { pre[b] } else { pre[b - 1] });
        }
    }
    assert forall|k: int| #![trigger post[k]] 0 <= k < post.len() - 1 implies
        post[k].1.0 <= post[k + 1].1.0 by {
        if k + 1 < j as int {
            assert(post[k] == pre[k]);
            assert(post[k + 1] == pre[k + 1]);
        } else if k + 1 == j as int {
            assert(post[k] == pre[k]);
            assert(post[k + 1] == (i, pair));
            // walk exit: j > 0 here, so pre[j-1].1.0 <= pair.0
        } else if k == j as int {
            assert(post[k] == (i, pair));
            assert(post[k + 1] == pre[k]);
            // walk invariant: pre[j].1.0 > pair.0
        } else {
            assert(post[k] == pre[k - 1]);
            assert(post[k + 1] == pre[k]);
        }
    }
}

/// Insertion sort by start, carrying each edit's original index so error
/// witnesses can name two DISTINCT input edits. Batches are small.
/// clippy::ptr_arg: Verus's Vec specs reason over Vec, not slices — the
/// proof covers this exact shape.
#[allow(clippy::ptr_arg, clippy::needless_range_loop)]
fn sort_by_start(edits: &Vec<(usize, usize)>) -> (out: Vec<(usize, (usize, usize))>)
    ensures
        out@.len() == edits@.len(),
        // every entry carries an original index and that index's value
        forall|k: int| #![trigger out@[k]] 0 <= k < out@.len() ==> {
            &&& (out@[k].0 as int) < edits@.len()
            &&& out@[k].1 == edits@[out@[k].0 as int]
        },
        // carried indices are pairwise distinct
        forall|a: int, b: int| #![trigger out@[a], out@[b]]
            0 <= a < b < out@.len() ==> out@[a].0 != out@[b].0,
        // sorted ascending by start (consecutive form)
        forall|k: int| #![trigger out@[k]] 0 <= k < out@.len() - 1 ==>
            out@[k].1.0 <= out@[k + 1].1.0,
{
    let mut out: Vec<(usize, (usize, usize))> = Vec::new();
    for i in 0..edits.len()
        invariant
            i <= edits.len(),
            out@.len() == i as int,
            forall|k: int| #![trigger out@[k]] 0 <= k < out@.len() ==> {
                &&& (out@[k].0 as int) < i as int
                &&& out@[k].1 == edits@[out@[k].0 as int]
            },
            forall|a: int, b: int| #![trigger out@[a], out@[b]]
                0 <= a < b < out@.len() ==> out@[a].0 != out@[b].0,
            forall|k: int| #![trigger out@[k]] 0 <= k < out@.len() - 1 ==>
                out@[k].1.0 <= out@[k + 1].1.0,
    {
        let pair = edits[i];
        let s = pair.0;
        let ghost pre = out@;
        // walk down past every entry whose start exceeds s
        let mut j = out.len();
        while j > 0 && out[j - 1].1.0 > s
            invariant
                j <= out.len(),
                out@ == pre,
                forall|p: int| #![trigger out@[p]] j <= p < out@.len() ==>
                    out@[p].1.0 > s as int,
            decreases j,
        {
            j -= 1;
        }
        out.insert(j, (i, pair));
        proof {
            insert_preserves(edits@, pre, j, i, pair);
        }
    }
    out
}

/// Sort by start and validate: out is the batch production applies.
/// (Sort, then sweep consecutive pairs, like prepare_replacements.)
/// clippy::result_unit_err / needless_range_loop: the unit error is the
/// verified contract (the ensures clauses carry the reason), and the
/// indexed sweep is what the invariants prove over.
#[allow(clippy::result_unit_err, clippy::needless_range_loop)]
pub fn check_batch(len: usize, edits: Vec<(usize, usize)>) -> (out: Result<Vec<(usize, usize)>, ()>)
    ensures
        out.is_ok() ==> batch_ok(to_ints(out.unwrap()@), len as int),
        out.is_err() ==> {
            ||| exists|i: int| #![trigger edits@[i]] 0 <= i < edits@.len() && {
                let (s, e) = edits@[i]; !(0 <= s <= e <= len as int)
            }
            ||| exists|i: int, j: int| #![trigger edits@[i], edits@[j]]
                0 <= i < edits@.len() && 0 <= j < edits@.len() && i != j && {
                let (si, ei) = edits@[i]; let (sj, _ej) = edits@[j];
                &&& si == sj || (si <= sj && ei > sj)
            }
        },
{
    let sorted = sort_by_start(&edits);
    let n = sorted.len();
    let mut i = 0usize;
    // sweep: bounds for each edit, conflict against the previous (sorted) edit
    while i < n
        invariant
            0 <= i <= n,
            n == sorted@.len(),
            // sort_by_start postconditions, restated so the body can use them
            forall|k: int| #![trigger sorted@[k]] 0 <= k < sorted@.len() ==> {
                &&& (sorted@[k].0 as int) < edits@.len()
                &&& sorted@[k].1 == edits@[sorted@[k].0 as int]
            },
            forall|a: int, b: int| #![trigger sorted@[a], sorted@[b]]
                0 <= a < b < sorted@.len() ==> sorted@[a].0 != sorted@[b].0,
            forall|k: int| #![trigger sorted@[k]] 0 <= k < sorted@.len() - 1 ==>
                sorted@[k].1.0 <= sorted@[k + 1].1.0,
            forall|k: int| #![trigger sorted@[k]] 0 <= k < i as int ==> {
                let (sk, ek) = sorted@[k].1;
                &&& 0 <= sk as int
                &&& sk <= ek
                &&& ek <= len
            },
            forall|k: int| #![trigger sorted@[k]] 0 <= k < i as int - 1 ==> {
                let (sk, ek) = sorted@[k].1;
                let (sk1, _ek1) = sorted@[k + 1].1;
                &&& sk < sk1
                &&& ek <= sk1
            },
        decreases n - i,
    {
        let (s, e) = sorted[i].1;
        if !(s <= e && e <= len) {
            proof {
                let w = sorted@[i as int].0 as int;
                assert(0 <= w < edits@.len());
                assert(edits@[w] == (s, e));
                assert(!(0 <= s as int && s <= e && e <= len));
                assert(exists|i2: int| #![trigger edits@[i2]] 0 <= i2 < edits@.len() && {
                    let (s2, e2) = edits@[i2]; !(0 <= s2 <= e2 <= len as int)
                });
            }
            return Err(());
        }
        if i > 0 {
            let (sp, ep) = sorted[i - 1].1;
            if sp == s || ep > s {
                proof {
                    let a = sorted@[i as int - 1].0 as int;
                    let b = sorted@[i as int].0 as int;
                    assert(0 <= a < edits@.len());
                    assert(0 <= b < edits@.len());
                    assert(a != b);
                    assert(edits@[a] == (sp, ep));
                    assert(edits@[b] == (s, e));
                    // sorted consecutive: sp <= s
                    assert(sp as int <= s as int);
                    assert(exists|i2: int, j2: int| #![trigger edits@[i2], edits@[j2]]
                        0 <= i2 < edits@.len() && 0 <= j2 < edits@.len() && i2 != j2 && {
                        let (si, ei) = edits@[i2]; let (sj, _ej) = edits@[j2];
                        &&& si == sj || (si <= sj && ei > sj)
                    });
                }
                return Err(());
            }
        }
        i += 1;
    }
    // strip the carried indices
    let mut result: Vec<(usize, usize)> = Vec::new();
    let mut k = 0usize;
    while k < n
        invariant
            0 <= k <= n,
            n == sorted@.len(),
            result@.len() == k as int,
            forall|p: int| #![trigger result@[p]] 0 <= p < k as int ==>
                result@[p] == sorted@[p].1,
        decreases n - k,
    {
        result.push(sorted[k].1);
        k += 1;
    }
    proof {
        let ri = to_ints(result@);
        assert forall|p: int| #![trigger ri[p]] 0 <= p < ri.len() implies {
            let (s, e) = ri[p];
            &&& 0 <= s <= e <= len as int
            &&& p + 1 < ri.len() ==> {
                let (s2, _) = ri[p + 1];
                &&& s < s2 &&& e <= s2
            }
        } by {
            assert(result@[p] == sorted@[p].1);
            if p + 1 < ri.len() {
                assert(result@[p + 1] == sorted@[p + 1].1);
            }
        }
        assert(batch_ok(ri, len as int));
    }
    Ok(result)
}

}

verus! {

// ---- 0057 VF02: batch anchor-mapping composition (the 0045 stretch) -----
//
// Production applies a validated batch in REVERSE: `apply_prepared`
// (buffer/mutation.rs, "reversed application preserves coordinates")
// iterates `prepared.edits.into_iter().rev()`, publishing one `Change`
// per edit, and `editor/transact.rs` folds `map_position` over the
// published journal in order — each fold step using the edit's ORIGINAL
// coordinates. The proofs below establish, for any batch meeting the
// `check_batch` geometry contract plus the per-edit `start <= new_end`
// construction (`new_end = start + replacement length`):
//
//   * coordinate preservation (`len_after_covers`): each edit's original
//     range is still in bounds at the moment production applies it;
//   * composition (`batch_map_composition`): folding the per-edit
//     mappings in application order equals the direct whole-batch map —
//     classify the anchor once against the original coordinates and
//     accumulate the deltas of the edits fully before it;
//   * the composed mapping is monotone (`batch_map_monotone`) and stays
//     inside the final buffer length (`batch_map_in_bounds`).
//
// The four spec functions below are #[verifier::opaque] with explicit
// reveal() at each use site: left transparent, their definitional axioms
// inflated the (unrelated) insert_preserves query past the pinned
// toolchain's default solver budget. Isolation keeps every proof at the
// default rlimit — a proof needing more room is a red flag, not a
// reason to raise the budget.

/// A validated batch with replacement extents: (start, old_end, new_end)
/// per edit — `check_batch`'s geometry plus `start <= new_end`, which
/// holds by construction (the post-edit end is start + replacement len).
#[verifier::opaque]
pub open spec fn batch3_ok(edits: Seq<(int, int, int)>, len: int) -> bool {
    forall|i: int| #![trigger edits[i]] 0 <= i < edits.len() ==> {
        let (s, oe, ne) = edits[i];
        &&& 0 <= s <= oe <= len
        &&& s <= ne
        &&& i + 1 < edits.len() ==> {
            let (s2, _oe2, _ne2) = edits[i + 1];
            &&& s < s2
            &&& oe <= s2
        }
    }
}

/// Buffer length after the last `t` edits were applied (application runs
/// in reverse: edit `edits.len() - t` is applied at step `t).
#[verifier::opaque]
pub open spec fn len_after(len0: int, edits: Seq<(int, int, int)>, t: int) -> int
    decreases t,
{
    if t <= 0 || t > edits.len() {
        len0
    } else {
        let (_s, oe, ne) = edits[edits.len() - t];
        len_after(len0, edits, t - 1) + (ne - oe)
    }
}

/// The composed mapping, exactly as production computes it: fold the
/// per-edit mapping over the last `t` edits in application (reverse)
/// order, each edit at its original coordinates.
#[verifier::opaque]
pub open spec fn batch_map_spec(pos: int, edits: Seq<(int, int, int)>, t: int) -> int
    decreases t,
{
    if t <= 0 || t > edits.len() {
        pos
    } else {
        let (s, oe, ne) = edits[edits.len() - t];
        map_spec(batch_map_spec(pos, edits, t - 1), s, oe, ne)
    }
}

/// The direct whole-batch map: one left-to-right pass from edit `i` over
/// the ORIGINAL coordinates, with `delta` the accumulated length change
/// of the edits fully before the anchor. This is the definition callers
/// reason about; production computes the fold.
#[verifier::opaque]
pub open spec fn direct_map(pos: int, edits: Seq<(int, int, int)>, i: int, delta: int) -> int
    decreases edits.len() - i,
{
    if i < 0 || i >= edits.len() {
        pos + delta
    } else {
        let (s, oe, ne) = edits[i];
        if pos < s {
            pos + delta
        } else if pos >= oe {
            direct_map(pos, edits, i + 1, delta + (ne - oe))
        } else {
            // inside the replaced range: collapse to the edit's start,
            // shifted by the deltas of the (disjoint, earlier) edits
            s + delta
        }
    }
}

/// Sorted non-overlapping edits chain: an earlier edit's end never
/// exceeds a later edit's start.
proof fn lemma_batch3_chain(edits: Seq<(int, int, int)>, len: int, i: int, j: int)
    requires
        batch3_ok(edits, len),
        0 <= i < j < edits.len(),
    ensures
        edits[i].1 <= edits[j].0,
    decreases j - i,
{
    reveal(batch3_ok);
    if j - i > 1 {
        lemma_batch3_chain(edits, len, i + 1, j);
        assert(edits[i].0 < edits[i + 1].0 && edits[i].1 <= edits[i + 1].0);
        assert(edits[i + 1].0 <= edits[i + 1].1);
    }
}

/// The accumulated delta distributes out of the direct map:
/// classification is over the original coordinates, so every result
/// shifts together.
proof fn lemma_direct_map_delta(
    pos: int,
    edits: Seq<(int, int, int)>,
    i: int,
    delta: int,
    extra: int,
)
    requires
        0 <= i <= edits.len(),
    ensures
        direct_map(pos, edits, i, delta + extra) == direct_map(pos, edits, i, delta) + extra,
    decreases edits.len() - i,
{
    reveal(direct_map);
    if i < edits.len() {
        let (_s, oe, ne) = edits[i];
        if pos >= oe && pos >= _s {
            lemma_direct_map_delta(pos, edits, i + 1, delta + (ne - oe), extra);
            assert(direct_map(pos, edits, i, delta + extra)
                == direct_map(pos, edits, i + 1, delta + (ne - oe) + extra));
            assert(direct_map(pos, edits, i, delta)
                == direct_map(pos, edits, i + 1, delta + (ne - oe)));
        }
    }
}

/// Lower bound: classifying an anchor at or past `bound` against edits
/// that all start at or past `bound` never pulls it below `bound` —
/// later deletions are disjoint and lie inside `[bound, pos]`.
proof fn lemma_direct_lower(
    pos: int,
    edits: Seq<(int, int, int)>,
    len: int,
    i: int,
    bound: int,
)
    requires
        batch3_ok(edits, len),
        0 <= i <= edits.len(),
        pos >= bound,
        forall|j: int| #![trigger edits[j]] i <= j < edits.len() ==> edits[j].0 >= bound,
    ensures
        direct_map(pos, edits, i, 0) >= bound,
    decreases edits.len() - i,
{
    reveal(batch3_ok);
    reveal(direct_map);
    if i < edits.len() {
        let (s, oe, ne) = edits[i];
        if pos >= oe && pos >= s {
            // every later edit starts at or past this edit's end
            assert forall|j: int| #![trigger edits[j]]
                i + 1 <= j < edits.len() implies edits[j].0 >= oe by {
                lemma_batch3_chain(edits, len, i, j);
            }
            lemma_direct_lower(pos, edits, len, i + 1, oe);
            lemma_direct_map_delta(pos, edits, i + 1, 0, ne - oe);
            assert(direct_map(pos, edits, i, 0)
                == direct_map(pos, edits, i + 1, ne - oe));
            // >= oe + (ne - oe) == ne >= s >= bound
        }
    }
}

/// Coordinate preservation: at the moment production applies edit
/// `edits.len() - t`, every not-yet-applied edit's original range is
/// still in bounds — reverse application never invalidates pending
/// coordinates.
proof fn lemma_len_after_covers(len0: int, edits: Seq<(int, int, int)>, t: int)
    requires
        batch3_ok(edits, len0),
        0 <= t <= edits.len(),
    ensures
        t > 0 ==> len_after(len0, edits, t) >= edits[edits.len() - t].2,
        forall|i: int| #![trigger edits[i]]
            0 <= i < edits.len() - t ==> edits[i].1 <= len_after(len0, edits, t),
    decreases t,
{
    reveal(batch3_ok);
    reveal(len_after);
    if t > 0 {
        lemma_len_after_covers(len0, edits, t - 1);
        let j = edits.len() - t;
        let (sj, oej, nej) = edits[j];
        // IH at i == j: oej <= len_after(t - 1)
        assert(len_after(len0, edits, t) == len_after(len0, edits, t - 1) + (nej - oej));
        assert(len_after(len0, edits, t) >= nej && nej >= sj);
        assert forall|i: int| #![trigger edits[i]]
            0 <= i < edits.len() - t implies edits[i].1 <= len_after(len0, edits, t) by {
            lemma_batch3_chain(edits, len0, i, j);
            // edits[i].1 <= sj <= nej <= len_after(t)
        }
    }
}

/// Monotonicity of the composed mapping: anchors keep their relative
/// order through the whole validated batch.
pub proof fn batch_map_monotone(a: int, b: int, len0: int, edits: Seq<(int, int, int)>, t: int)
    requires
        batch3_ok(edits, len0),
        0 <= t <= edits.len(),
        a <= b,
    ensures
        batch_map_spec(a, edits, t) <= batch_map_spec(b, edits, t),
    decreases t,
{
    reveal(batch3_ok);
    reveal(batch_map_spec);
    if t > 0 {
        batch_map_monotone(a, b, len0, edits, t - 1);
        let (s, oe, ne) = edits[edits.len() - t];
        map_spec_monotone(batch_map_spec(a, edits, t - 1), batch_map_spec(b, edits, t - 1), s, oe, ne);
    }
}

/// The composed mapping stays inside the final buffer length: an anchor
/// inside the pre-batch buffer lands inside the post-batch buffer.
pub proof fn batch_map_in_bounds(pos: int, len0: int, edits: Seq<(int, int, int)>, t: int)
    requires
        batch3_ok(edits, len0),
        0 <= t <= edits.len(),
        0 <= pos <= len0,
    ensures
        0 <= batch_map_spec(pos, edits, t) <= len_after(len0, edits, t),
    decreases t,
{
    reveal(batch3_ok);
    reveal(batch_map_spec);
    reveal(len_after);
    if t > 0 {
        batch_map_in_bounds(pos, len0, edits, t - 1);
        lemma_len_after_covers(len0, edits, t - 1);
        let j = edits.len() - t;
        let (s, oe, ne) = edits[j];
        assert(len_after(len0, edits, t)
            == len_after(len0, edits, t - 1) - (oe - s) + (ne - s));
        map_spec_in_bounds(
            batch_map_spec(pos, edits, t - 1),
            s,
            oe,
            ne,
            len_after(len0, edits, t - 1),
            len_after(len0, edits, t),
        );
    }
}

/// One composition step: the direct map through edit `i` and the rest
/// equals mapping the direct map of the rest through edit `i`. This is
/// the heart of the theorem — a later fold step with the edit's
/// ORIGINAL coordinates agrees with the single-pass direct map.
proof fn lemma_direct_step(pos: int, edits: Seq<(int, int, int)>, len0: int, i: int)
    requires
        batch3_ok(edits, len0),
        0 <= i < edits.len(),
        0 <= pos <= len0,
    ensures
        direct_map(pos, edits, i, 0) == map_spec(
            direct_map(pos, edits, i + 1, 0),
            edits[i].0,
            edits[i].1,
            edits[i].2,
        ),
{
    let (s, oe, ne) = edits[i];
    let rest = direct_map(pos, edits, i + 1, 0);
    reveal(batch3_ok);
    reveal(direct_map);
    if pos < s {
        // the anchor precedes this edit and every later one (strict
        // sort): both sides leave it alone
        if i + 1 < edits.len() {
            assert(pos < edits[i + 1].0);
        }
        assert(rest == pos);
    } else if pos >= oe {
        // the rest map never drops below this edit's end, so map_spec
        // shifts it by exactly this edit's delta
        assert forall|j: int| #![trigger edits[j]]
            i + 1 <= j < edits.len() implies edits[j].0 >= oe by {
            lemma_batch3_chain(edits, len0, i, j);
        }
        lemma_direct_lower(pos, edits, len0, i + 1, oe);
        lemma_direct_map_delta(pos, edits, i + 1, 0, ne - oe);
        assert(direct_map(pos, edits, i, 0) == direct_map(pos, edits, i + 1, ne - oe));
        assert(rest >= oe);
    } else {
        // inside the replaced range: every later edit starts at or past
        // oe, so the rest pass leaves the anchor alone and both sides
        // collapse to the edit's start
        if i + 1 < edits.len() {
            assert(pos < edits[i + 1].0);
        }
        assert(rest == pos);
    }
}

/// Fold/direct equality at every prefix of the application sequence.
proof fn lemma_batch_compose_step(pos: int, len0: int, edits: Seq<(int, int, int)>, t: int)
    requires
        batch3_ok(edits, len0),
        0 <= t <= edits.len(),
        0 <= pos <= len0,
    ensures
        batch_map_spec(pos, edits, t) == direct_map(pos, edits, edits.len() - t, 0),
    decreases t,
{
    reveal(batch_map_spec);
    reveal(direct_map);
    if t > 0 {
        lemma_batch_compose_step(pos, len0, edits, t - 1);
        lemma_direct_step(pos, edits, len0, edits.len() - t);
        // batch_map_spec(pos, edits, t)
        //   == map_spec(batch_map_spec(pos, edits, t-1), edits[len-t])   (def)
        //   == map_spec(direct_map(pos, edits, len-t+1, 0), edits[len-t]) (IH)
        //   == direct_map(pos, edits, len-t, 0)                           (step)
        assert(batch_map_spec(pos, edits, t)
            == map_spec(direct_map(pos, edits, edits.len() - t + 1, 0),
                edits[edits.len() - t].0, edits[edits.len() - t].1, edits[edits.len() - t].2));
    }
}

/// The composition theorem (0045 stretch obligation, closed for VF02):
/// mapping an anchor through a validated batch — folding the per-edit
/// mappings in the reverse application order production publishes —
/// equals the direct whole-batch map over the original coordinates.
/// Multicursor and collection anchors rely on exactly this equality.
pub proof fn batch_map_composition(pos: int, len0: int, edits: Seq<(int, int, int)>)
    requires
        batch3_ok(edits, len0),
        0 <= pos <= len0,
    ensures
        batch_map_spec(pos, edits, edits.len() as int) == direct_map(pos, edits, 0, 0),
{
    lemma_batch_compose_step(pos, len0, edits, edits.len() as int);
}

}
