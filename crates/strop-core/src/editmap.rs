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
//! - the mapping is monotone and stays inside the new buffer length.

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
