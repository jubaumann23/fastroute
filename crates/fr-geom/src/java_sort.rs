//! Port of `java.util.ComparableTimSort` (JDK 17), used by `Arrays.sort(Object[])`.
//!
//! For consistent comparators any stable sort gives the same result, but `Line.compareTo` is
//! not a consistent total order for degenerate (zero length) lines or huge coordinates. To
//! reproduce Java's element order exactly in those cases the TimSort algorithm itself is ported,
//! including binary insertion sort, run detection and galloping merges.

const MIN_MERGE: usize = 32;
const MIN_GALLOP: i32 = 7;

/// Sorts `v` exactly like Java `Arrays.sort(Object[])` with `compare_to` as
/// `Comparable.compareTo`. Panics where Java throws "Comparison method violates its general
/// contract!".
pub fn sort_by<T: Clone, F: FnMut(&T, &T) -> i32>(v: &mut [T], mut compare_to: F) {
    let n = v.len();
    if n < 2 {
        return;
    }
    let mut idx: Vec<usize> = (0..n).collect();
    {
        let src: &[T] = v;
        let mut c = |x: usize, y: usize| compare_to(&src[x], &src[y]);
        tim_sort(&mut idx, &mut c);
    }
    let sorted: Vec<T> = idx.iter().map(|&i| v[i].clone()).collect();
    v.clone_from_slice(&sorted);
}

fn tim_sort<C: FnMut(usize, usize) -> i32>(a: &mut [usize], c: &mut C) {
    let hi = a.len();
    let mut lo = 0usize;
    let mut n_remaining = hi - lo;
    if n_remaining < 2 {
        return;
    }
    if n_remaining < MIN_MERGE {
        let init_run_len = count_run_and_make_ascending(a, lo, hi, c);
        binary_sort(a, lo, hi, lo + init_run_len, c);
        return;
    }
    let mut ts = TimSort {
        tmp: Vec::new(),
        min_gallop: MIN_GALLOP,
        run_base: Vec::new(),
        run_len: Vec::new(),
    };
    let min_run = min_run_length(n_remaining);
    loop {
        let mut run_len = count_run_and_make_ascending(a, lo, hi, c);
        if run_len < min_run {
            let force = if n_remaining <= min_run {
                n_remaining
            } else {
                min_run
            };
            binary_sort(a, lo, lo + force, lo + run_len, c);
            run_len = force;
        }
        ts.run_base.push(lo);
        ts.run_len.push(run_len);
        ts.merge_collapse(a, c);
        lo += run_len;
        n_remaining -= run_len;
        if n_remaining == 0 {
            break;
        }
    }
    ts.merge_force_collapse(a, c);
}

fn binary_sort<C: FnMut(usize, usize) -> i32>(
    a: &mut [usize],
    lo: usize,
    hi: usize,
    start: usize,
    c: &mut C,
) {
    let mut start = if start == lo { start + 1 } else { start };
    while start < hi {
        let pivot = a[start];
        let mut left = lo;
        let mut right = start;
        while left < right {
            let mid = (left + right) >> 1;
            if c(pivot, a[mid]) < 0 {
                right = mid;
            } else {
                left = mid + 1;
            }
        }
        a.copy_within(left..start, left + 1);
        a[left] = pivot;
        start += 1;
    }
}

fn count_run_and_make_ascending<C: FnMut(usize, usize) -> i32>(
    a: &mut [usize],
    lo: usize,
    hi: usize,
    c: &mut C,
) -> usize {
    let mut run_hi = lo + 1;
    if run_hi == hi {
        return 1;
    }
    let first = a[run_hi];
    run_hi += 1;
    if c(first, a[lo]) < 0 {
        // descending
        while run_hi < hi && c(a[run_hi], a[run_hi - 1]) < 0 {
            run_hi += 1;
        }
        a[lo..run_hi].reverse();
    } else {
        // ascending
        while run_hi < hi && c(a[run_hi], a[run_hi - 1]) >= 0 {
            run_hi += 1;
        }
    }
    run_hi - lo
}

fn min_run_length(mut n: usize) -> usize {
    let mut r = 0;
    while n >= MIN_MERGE {
        r |= n & 1;
        n >>= 1;
    }
    n + r
}

/// `gallopLeft(key, a, base, len, hint)`; `a` is given as slice (either the array or tmp).
fn gallop_left<C: FnMut(usize, usize) -> i32>(
    key: usize,
    a: &[usize],
    base: usize,
    len: usize,
    hint: usize,
    c: &mut C,
) -> usize {
    let mut last_ofs: isize = 0;
    let mut ofs: isize = 1;
    let (hint_i, len_i) = (hint as isize, len as isize);
    if c(key, a[base + hint]) > 0 {
        // gallop right until a[base+hint+lastOfs] < key <= a[base+hint+ofs]
        let max_ofs = len_i - hint_i;
        while ofs < max_ofs && c(key, a[(base as isize + hint_i + ofs) as usize]) > 0 {
            last_ofs = ofs;
            ofs = (ofs << 1) + 1;
        }
        if ofs > max_ofs {
            ofs = max_ofs;
        }
        last_ofs += hint_i;
        ofs += hint_i;
    } else {
        // gallop left until a[base+hint-ofs] < key <= a[base+hint-lastOfs]
        let max_ofs = hint_i + 1;
        while ofs < max_ofs && c(key, a[(base as isize + hint_i - ofs) as usize]) <= 0 {
            last_ofs = ofs;
            ofs = (ofs << 1) + 1;
        }
        if ofs > max_ofs {
            ofs = max_ofs;
        }
        let tmp = last_ofs;
        last_ofs = hint_i - ofs;
        ofs = hint_i - tmp;
    }
    last_ofs += 1;
    while last_ofs < ofs {
        let m = last_ofs + ((ofs - last_ofs) >> 1);
        if c(key, a[(base as isize + m) as usize]) > 0 {
            last_ofs = m + 1;
        } else {
            ofs = m;
        }
    }
    ofs as usize
}

fn gallop_right<C: FnMut(usize, usize) -> i32>(
    key: usize,
    a: &[usize],
    base: usize,
    len: usize,
    hint: usize,
    c: &mut C,
) -> usize {
    let mut ofs: isize = 1;
    let mut last_ofs: isize = 0;
    let (hint_i, len_i) = (hint as isize, len as isize);
    if c(key, a[base + hint]) < 0 {
        // gallop left until a[b+hint - ofs] <= key < a[b+hint - lastOfs]
        let max_ofs = hint_i + 1;
        while ofs < max_ofs && c(key, a[(base as isize + hint_i - ofs) as usize]) < 0 {
            last_ofs = ofs;
            ofs = (ofs << 1) + 1;
        }
        if ofs > max_ofs {
            ofs = max_ofs;
        }
        let tmp = last_ofs;
        last_ofs = hint_i - ofs;
        ofs = hint_i - tmp;
    } else {
        // gallop right until a[b+hint + lastOfs] <= key < a[b+hint + ofs]
        let max_ofs = len_i - hint_i;
        while ofs < max_ofs && c(key, a[(base as isize + hint_i + ofs) as usize]) >= 0 {
            last_ofs = ofs;
            ofs = (ofs << 1) + 1;
        }
        if ofs > max_ofs {
            ofs = max_ofs;
        }
        last_ofs += hint_i;
        ofs += hint_i;
    }
    last_ofs += 1;
    while last_ofs < ofs {
        let m = last_ofs + ((ofs - last_ofs) >> 1);
        if c(key, a[(base as isize + m) as usize]) < 0 {
            ofs = m;
        } else {
            last_ofs = m + 1;
        }
    }
    ofs as usize
}

struct TimSort {
    tmp: Vec<usize>,
    min_gallop: i32,
    run_base: Vec<usize>,
    run_len: Vec<usize>,
}

const CONTRACT: &str = "IllegalArgumentException: Comparison method violates its general contract!";

impl TimSort {
    fn merge_collapse<C: FnMut(usize, usize) -> i32>(&mut self, a: &mut [usize], c: &mut C) {
        while self.run_len.len() > 1 {
            let mut n = self.run_len.len() as isize - 2;
            let rl = |i: isize| self.run_len[i as usize];
            if n > 0 && rl(n - 1) <= rl(n) + rl(n + 1) || n > 1 && rl(n - 2) <= rl(n) + rl(n - 1) {
                if rl(n - 1) < rl(n + 1) {
                    n -= 1;
                }
            } else if n < 0 || rl(n) > rl(n + 1) {
                break; // invariant is established
            }
            self.merge_at(n as usize, a, c);
        }
    }

    fn merge_force_collapse<C: FnMut(usize, usize) -> i32>(&mut self, a: &mut [usize], c: &mut C) {
        while self.run_len.len() > 1 {
            let mut n = self.run_len.len() - 2;
            if n > 0 && self.run_len[n - 1] < self.run_len[n + 1] {
                n -= 1;
            }
            self.merge_at(n, a, c);
        }
    }

    fn merge_at<C: FnMut(usize, usize) -> i32>(&mut self, i: usize, a: &mut [usize], c: &mut C) {
        let mut base1 = self.run_base[i];
        let mut len1 = self.run_len[i];
        let base2 = self.run_base[i + 1];
        let mut len2 = self.run_len[i + 1];
        // record the length of the combined runs; remove run i + 1
        self.run_len[i] = len1 + len2;
        self.run_base.remove(i + 1);
        self.run_len.remove(i + 1);

        // find where the first element of run2 goes in run1
        let k = gallop_right(a[base2], a, base1, len1, 0, c);
        base1 += k;
        len1 -= k;
        if len1 == 0 {
            return;
        }
        // find where the last element of run1 goes in run2
        len2 = gallop_left(a[base1 + len1 - 1], a, base2, len2, len2 - 1, c);
        if len2 == 0 {
            return;
        }
        if len1 <= len2 {
            self.merge_lo(base1, len1, base2, len2, a, c);
        } else {
            self.merge_hi(base1, len1, base2, len2, a, c);
        }
    }

    fn merge_lo<C: FnMut(usize, usize) -> i32>(
        &mut self,
        base1: usize,
        mut len1: usize,
        base2: usize,
        mut len2: usize,
        a: &mut [usize],
        c: &mut C,
    ) {
        self.tmp.clear();
        self.tmp.extend_from_slice(&a[base1..base1 + len1]);
        let tmp = std::mem::take(&mut self.tmp);
        let mut cursor1 = 0usize; // into tmp
        let mut cursor2 = base2; // into a
        let mut dest = base1; // into a
        a[dest] = a[cursor2];
        dest += 1;
        cursor2 += 1;
        len2 -= 1;
        if len2 == 0 {
            a[dest..dest + len1].copy_from_slice(&tmp[cursor1..cursor1 + len1]);
            self.tmp = tmp;
            return;
        }
        if len1 == 1 {
            a.copy_within(cursor2..cursor2 + len2, dest);
            a[dest + len2] = tmp[cursor1];
            self.tmp = tmp;
            return;
        }
        let mut min_gallop = self.min_gallop;
        'outer: loop {
            let mut count1: i32 = 0;
            let mut count2: i32 = 0;
            // straightforward merge until one run starts winning consistently
            loop {
                if c(a[cursor2], tmp[cursor1]) < 0 {
                    a[dest] = a[cursor2];
                    dest += 1;
                    cursor2 += 1;
                    count2 += 1;
                    count1 = 0;
                    len2 -= 1;
                    if len2 == 0 {
                        break 'outer;
                    }
                } else {
                    a[dest] = tmp[cursor1];
                    dest += 1;
                    cursor1 += 1;
                    count1 += 1;
                    count2 = 0;
                    len1 -= 1;
                    if len1 == 1 {
                        break 'outer;
                    }
                }
                if (count1 | count2) >= min_gallop {
                    break;
                }
            }
            // galloping
            loop {
                count1 = gallop_right(a[cursor2], &tmp, cursor1, len1, 0, c) as i32;
                if count1 != 0 {
                    let n = count1 as usize;
                    a[dest..dest + n].copy_from_slice(&tmp[cursor1..cursor1 + n]);
                    dest += n;
                    cursor1 += n;
                    len1 -= n;
                    if len1 <= 1 {
                        break 'outer;
                    }
                }
                a[dest] = a[cursor2];
                dest += 1;
                cursor2 += 1;
                len2 -= 1;
                if len2 == 0 {
                    break 'outer;
                }
                count2 = gallop_left(tmp[cursor1], a, cursor2, len2, 0, c) as i32;
                if count2 != 0 {
                    let n = count2 as usize;
                    a.copy_within(cursor2..cursor2 + n, dest);
                    dest += n;
                    cursor2 += n;
                    len2 -= n;
                    if len2 == 0 {
                        break 'outer;
                    }
                }
                a[dest] = tmp[cursor1];
                dest += 1;
                cursor1 += 1;
                len1 -= 1;
                if len1 == 1 {
                    break 'outer;
                }
                min_gallop -= 1;
                if !(count1 >= MIN_GALLOP || count2 >= MIN_GALLOP) {
                    break;
                }
            }
            if min_gallop < 0 {
                min_gallop = 0;
            }
            min_gallop += 2; // penalize for leaving gallop mode
        }
        self.min_gallop = if min_gallop < 1 { 1 } else { min_gallop };
        if len1 == 1 {
            a.copy_within(cursor2..cursor2 + len2, dest);
            a[dest + len2] = tmp[cursor1];
        } else if len1 == 0 {
            panic!("{}", CONTRACT);
        } else {
            a[dest..dest + len1].copy_from_slice(&tmp[cursor1..cursor1 + len1]);
        }
        self.tmp = tmp;
    }

    fn merge_hi<C: FnMut(usize, usize) -> i32>(
        &mut self,
        base1: usize,
        mut len1: usize,
        base2: usize,
        mut len2: usize,
        a: &mut [usize],
        c: &mut C,
    ) {
        self.tmp.clear();
        self.tmp.extend_from_slice(&a[base2..base2 + len2]);
        let tmp = std::mem::take(&mut self.tmp);
        // cursors may become -1, so use isize
        let mut cursor1: isize = (base1 + len1) as isize - 1; // into a
        let mut cursor2: isize = len2 as isize - 1; // into tmp
        let mut dest: isize = (base2 + len2) as isize - 1; // into a
        a[dest as usize] = a[cursor1 as usize];
        dest -= 1;
        cursor1 -= 1;
        len1 -= 1;
        if len1 == 0 {
            let d = (dest - (len2 as isize - 1)) as usize;
            a[d..d + len2].copy_from_slice(&tmp[0..len2]);
            self.tmp = tmp;
            return;
        }
        if len2 == 1 {
            dest -= len1 as isize;
            cursor1 -= len1 as isize;
            a.copy_within(
                (cursor1 + 1) as usize..(cursor1 + 1) as usize + len1,
                (dest + 1) as usize,
            );
            a[dest as usize] = tmp[cursor2 as usize];
            self.tmp = tmp;
            return;
        }
        let mut min_gallop = self.min_gallop;
        'outer: loop {
            let mut count1: i32 = 0;
            let mut count2: i32 = 0;
            loop {
                if c(tmp[cursor2 as usize], a[cursor1 as usize]) < 0 {
                    a[dest as usize] = a[cursor1 as usize];
                    dest -= 1;
                    cursor1 -= 1;
                    count1 += 1;
                    count2 = 0;
                    len1 -= 1;
                    if len1 == 0 {
                        break 'outer;
                    }
                } else {
                    a[dest as usize] = tmp[cursor2 as usize];
                    dest -= 1;
                    cursor2 -= 1;
                    count2 += 1;
                    count1 = 0;
                    len2 -= 1;
                    if len2 == 1 {
                        break 'outer;
                    }
                }
                if (count1 | count2) >= min_gallop {
                    break;
                }
            }
            loop {
                count1 = (len1 - gallop_right(tmp[cursor2 as usize], a, base1, len1, len1 - 1, c))
                    as i32;
                if count1 != 0 {
                    let n = count1 as usize;
                    dest -= n as isize;
                    cursor1 -= n as isize;
                    len1 -= n;
                    a.copy_within(
                        (cursor1 + 1) as usize..(cursor1 + 1) as usize + n,
                        (dest + 1) as usize,
                    );
                    if len1 == 0 {
                        break 'outer;
                    }
                }
                a[dest as usize] = tmp[cursor2 as usize];
                dest -= 1;
                cursor2 -= 1;
                len2 -= 1;
                if len2 == 1 {
                    break 'outer;
                }
                count2 =
                    (len2 - gallop_left(a[cursor1 as usize], &tmp, 0, len2, len2 - 1, c)) as i32;
                if count2 != 0 {
                    let n = count2 as usize;
                    dest -= n as isize;
                    cursor2 -= n as isize;
                    len2 -= n;
                    let s = (cursor2 + 1) as usize;
                    let d = (dest + 1) as usize;
                    a[d..d + n].copy_from_slice(&tmp[s..s + n]);
                    if len2 <= 1 {
                        break 'outer;
                    }
                }
                a[dest as usize] = a[cursor1 as usize];
                dest -= 1;
                cursor1 -= 1;
                len1 -= 1;
                if len1 == 0 {
                    break 'outer;
                }
                min_gallop -= 1;
                if !(count1 >= MIN_GALLOP || count2 >= MIN_GALLOP) {
                    break;
                }
            }
            if min_gallop < 0 {
                min_gallop = 0;
            }
            min_gallop += 2;
        }
        self.min_gallop = if min_gallop < 1 { 1 } else { min_gallop };
        if len2 == 1 {
            dest -= len1 as isize;
            cursor1 -= len1 as isize;
            a.copy_within(
                (cursor1 + 1) as usize..(cursor1 + 1) as usize + len1,
                (dest + 1) as usize,
            );
            a[dest as usize] = tmp[cursor2 as usize];
        } else if len2 == 0 {
            panic!("{}", CONTRACT);
        } else {
            let d = (dest - (len2 as isize - 1)) as usize;
            a[d..d + len2].copy_from_slice(&tmp[0..len2]);
        }
        self.tmp = tmp;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_like_a_stable_sort_for_consistent_comparators() {
        let mut state = 12345u64;
        for n in [0usize, 1, 2, 5, 31, 32, 33, 64, 100, 257, 1000, 5000] {
            let v: Vec<(i32, usize)> = (0..n)
                .map(|i| {
                    state = state
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    (((state >> 33) % 50) as i32, i)
                })
                .collect();
            let mut a = v.clone();
            sort_by(&mut a, |x, y| (x.0 > y.0) as i32 - (x.0 < y.0) as i32);
            let mut b = v.clone();
            b.sort_by_key(|x| x.0);
            assert_eq!(a, b, "n = {n}");
        }
    }

    #[test]
    fn partially_sorted_inputs_exercise_galloping() {
        // long ascending runs with interleaved values force galloping merges
        let mut v: Vec<(i32, usize)> = Vec::new();
        for i in 0..400 {
            v.push((i * 2, v.len()));
        }
        for i in 0..400 {
            v.push((i, v.len()));
        }
        for i in (0..300).rev() {
            v.push((i * 3, v.len()));
        }
        let mut a = v.clone();
        sort_by(&mut a, |x, y| (x.0 > y.0) as i32 - (x.0 < y.0) as i32);
        let mut b = v.clone();
        b.sort_by_key(|x| x.0);
        assert_eq!(a, b);
    }
}
