//! Replays ground-truth vectors generated with a real JDK 25 by the programs in `java/`.

use fr_jcompat::hashmap::JavaIntHashMap;
use fr_jcompat::string::{compare_to_ignore_case_utf16, java_string_compare_utf16};
use fr_jcompat::treemap::{ordering_from_i32, JavaTreeMap, JavaTreeSet};
use fr_jcompat::*;
use std::cmp::Ordering;

fn data(name: &str) -> String {
    let path = format!("{}/tests/data/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn nums<T: std::str::FromStr>(s: &str) -> Vec<T>
where
    T::Err: std::fmt::Debug,
{
    s.split_whitespace().map(|t| t.parse().unwrap()).collect()
}

// ---------------------------------------------------------------------------------------------
// Random / shuffle

#[test]
fn random_matches_jdk() {
    let text = data("random.txt");
    let mut checked = 0;
    for line in text.lines() {
        let (head, vals) = line.split_once(" : ").unwrap_or((line.trim_end_matches(" :"), ""));
        let h: Vec<&str> = head.split_whitespace().collect();
        let seed: i64 = h[1].parse().unwrap();
        let mut r = JavaRandom::new(seed);
        let got: Vec<i64> = match h[0] {
            "int" => (0..20).map(|_| r.next_int() as i64).collect(),
            "bound" => {
                let b: i32 = h[2].parse().unwrap();
                (0..20).map(|_| r.next_int_bound(b) as i64).collect()
            }
            "range" => {
                let o: i32 = h[2].parse().unwrap();
                let b: i32 = h[3].parse().unwrap();
                (0..20).map(|_| r.next_int_range(o, b) as i64).collect()
            }
            "long" => (0..20).map(|_| r.next_long()).collect(),
            "double" => (0..20).map(|_| r.next_double().to_bits() as i64).collect(),
            "float" => (0..20).map(|_| r.next_float().to_bits() as i32 as i64).collect(),
            "bool" => (0..40).map(|_| r.next_boolean() as i64).collect(),
            "mixed" => {
                let mut v = vec![];
                for _ in 0..10 {
                    v.push(r.next_int_bound(7) as i64);
                    v.push(r.next_long());
                    v.push(r.next_boolean() as i64);
                    v.push(r.next_double().to_bits() as i64);
                }
                v
            }
            "reseed" => {
                r.next_int();
                r.set_seed(h[2].parse().unwrap());
                (0..10).map(|_| r.next_int() as i64).collect()
            }
            "shuffle" | "shuffle2" => {
                let n: usize = h[2].parse().unwrap();
                let mut l: Vec<i64> = (0..n as i64).collect();
                shuffle(&mut l, &mut r);
                if h[0] == "shuffle2" {
                    shuffle(&mut l, &mut r);
                }
                l
            }
            k => panic!("unknown kind {k}"),
        };
        let want: Vec<i64> = nums(vals);
        assert_eq!(got, want, "{line}");
        checked += 1;
    }
    assert!(checked > 500);
}

// ---------------------------------------------------------------------------------------------
// TreeMap / TreeSet

fn tree_cmp(name: &str) -> fn(&i32, &i32) -> Ordering {
    fn nat(a: &i32, b: &i32) -> Ordering {
        a.cmp(b)
    }
    fn tol(a: &i32, b: &i32) -> Ordering {
        if *a < b - 5 {
            Ordering::Less
        } else if *a > b + 5 {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
    fn xor(a: &i32, b: &i32) -> Ordering {
        if a == b {
            return Ordering::Equal;
        }
        let h = ((a ^ b).wrapping_mul(0x9E37_79B9u32 as i32) as u32) >> 7;
        let s = if h & 1 == 0 { 1 } else { -1 };
        ordering_from_i32(if a < b { -s } else { s })
    }
    fn asym(a: &i32, b: &i32) -> Ordering {
        ordering_from_i32((a * 7 + b * 3).rem_euclid(5) - 2)
    }
    match name {
        "nat" => nat,
        "tol" => tol,
        "xor" => xor,
        "asym" => asym,
        _ => panic!("{name}"),
    }
}

fn opt<T: std::fmt::Display>(o: Option<T>) -> String {
    o.map_or("null".to_string(), |v| v.to_string())
}

#[test]
fn treemap_matches_jdk() {
    let text = data("treemap.txt");
    let mut lines = text.lines();
    let mut cases = 0;
    while let Some(header) = lines.next() {
        let h: Vec<&str> = header.split_whitespace().collect();
        assert_eq!(h[0], "CASE");
        let is_map = h[1] == "map";
        let cmp = tree_cmp(h[2]);
        let mut map: JavaTreeMap<i32, i32, _> = JavaTreeMap::new(cmp);
        let mut set: JavaTreeSet<i32, _> = JavaTreeSet::new(cmp);
        for line in lines.by_ref() {
            if line == "END" {
                break;
            }
            let (op, want) = line.split_once(" -> ").unwrap();
            let o: Vec<&str> = op.split_whitespace().collect();
            let arg = |i: usize| -> i32 { o[i].parse().unwrap() };
            let got: String = match o[0] {
                "put" => opt(map.put(arg(1), arg(2))),
                "add" => set.add(arg(1)).to_string(),
                "rem" if is_map => opt(map.remove(&arg(1)).map(|(_, v)| v)),
                "rem" => set.remove(&arg(1)).is_some().to_string(),
                "get" => opt(map.get(&arg(1))),
                "has" => set.contains(&arg(1)).to_string(),
                "ceil" | "floor" | "higher" | "lower" => {
                    let k = arg(1);
                    let r = if is_map {
                        match o[0] {
                            "ceil" => map.ceiling_key(&k),
                            "floor" => map.floor_key(&k),
                            "higher" => map.higher_key(&k),
                            _ => map.lower_key(&k),
                        }
                    } else {
                        match o[0] {
                            "ceil" => set.ceiling(&k),
                            "floor" => set.floor(&k),
                            "higher" => set.higher(&k),
                            _ => set.lower(&k),
                        }
                    };
                    opt(r)
                }
                "pf" | "pl" => {
                    if is_map {
                        let e = if o[0] == "pf" { map.poll_first() } else { map.poll_last() };
                        opt(e.map(|(k, v)| format!("{k}:{v}")))
                    } else {
                        opt(if o[0] == "pf" { set.poll_first() } else { set.poll_last() })
                    }
                }
                "first" | "last" => {
                    if is_map {
                        opt(if o[0] == "first" { map.first_key() } else { map.last_key() })
                    } else {
                        opt(if o[0] == "first" { set.first() } else { set.last() })
                    }
                }
                "iterrem" => {
                    let m = arg(1);
                    let mut vis = vec![];
                    if is_map {
                        let mut c = map.cursor();
                        while let Some(e) = c.next(&map) {
                            let (k, v) = map.entry(e);
                            vis.push(k.to_string());
                            if k.rem_euclid(m) == 0 || v.rem_euclid(m) == 0 {
                                c.remove(&mut map);
                            }
                        }
                    } else {
                        let mut c = set.cursor();
                        while let Some(e) = c.next(set.as_map()) {
                            let k = *set.as_map().key(e);
                            vis.push(k.to_string());
                            if k.rem_euclid(m) == 0 {
                                c.remove(set.as_map_mut());
                            }
                        }
                    }
                    if vis.is_empty() {
                        "-".to_string()
                    } else {
                        vis.join(",")
                    }
                }
                "dump" => {
                    let (n, body) = if is_map {
                        (map.len(), map.iter().map(|(k, v)| format!("{k}:{v}")).collect::<Vec<_>>())
                    } else {
                        (set.len(), set.iter().map(|k| k.to_string()).collect::<Vec<_>>())
                    };
                    format!("{n} ; {}", if body.is_empty() { "-".to_string() } else { body.join(",") })
                }
                x => panic!("unknown op {x}"),
            };
            assert_eq!(got, want, "case {header}: {line}");
            if is_map {
                map.check_invariants();
            } else {
                set.as_map().check_invariants();
            }
        }
        cases += 1;
    }
    assert_eq!(cases, 200);
}

// ---------------------------------------------------------------------------------------------
// HashMap<Integer, Integer>

#[test]
fn hashmap_matches_jdk() {
    let text = data("hashmap.txt");
    let mut lines = text.lines();
    let mut cases = 0;
    let mut treeified_cases = 0;
    let mut tree_removals = 0;
    while let Some(header) = lines.next() {
        let h: Vec<&str> = header.split_whitespace().collect();
        assert_eq!(h[0], "CASE");
        let cap: i32 = h[3].parse().unwrap();
        let mut map: JavaIntHashMap<i32> = if cap < 0 { JavaIntHashMap::new() } else { JavaIntHashMap::with_capacity(cap) };
        let mut saw_tree = false;
        for line in lines.by_ref() {
            if line == "END" {
                break;
            }
            if let Some(k) = line.strip_prefix("rem ") {
                let k: i32 = k.split_whitespace().next().unwrap().parse().unwrap();
                tree_removals += map.bin_is_tree(k) as usize;
            }
            if line == "clear" {
                map.clear();
                continue;
            }
            let (op, want) = line.split_once(" -> ").unwrap();
            let o: Vec<&str> = op.split_whitespace().collect();
            let arg = |i: usize| -> i32 { o[i].parse().unwrap() };
            let got = match o[0] {
                "put" => opt(map.put(arg(1), arg(2))),
                "pia" => {
                    let old = map.get(arg(1)).copied();
                    let inserted = map.put_if_absent(arg(1), arg(2));
                    assert_eq!(inserted, old.is_none());
                    opt(old)
                }
                "cia" => map.compute_if_absent(arg(1), || arg(2)).to_string(),
                "rem" => opt(map.remove(arg(1))),
                "get" => opt(map.get(arg(1))),
                "dump" => {
                    let body: Vec<String> = map.iter().map(|(k, v)| format!("{k}:{v}")).collect();
                    format!("{} ; {}", map.len(), if body.is_empty() { "-".to_string() } else { body.join(",") })
                }
                x => panic!("unknown op {x}"),
            };
            saw_tree |= map.has_tree_bins();
            assert_eq!(got, want, "case {header}: {line}");
        }
        treeified_cases += saw_tree as usize;
        cases += 1;
    }
    assert_eq!(cases, 112);
    // make sure the tree-bin code paths are actually exercised by the vectors
    assert!(treeified_cases >= 5, "only {treeified_cases} cases with tree bins");
    assert!(tree_removals >= 50, "only {tree_removals} removals from tree bins");
}

// ---------------------------------------------------------------------------------------------
// DoubleStream.sum / average / DoubleSummaryStatistics

/// `Double.doubleToLongBits`: every NaN collapses to the canonical 0x7ff8000000000000 (the sign of
/// a NaN produced by inf - inf is platform dependent: x86 sets it, ARM does not).
fn double_to_long_bits(v: f64) -> i64 {
    if v.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        v.to_bits() as i64
    }
}

#[test]
fn sum_matches_jdk() {
    let text = data("sum.txt");
    let mut n = 0;
    for line in text.lines() {
        let (head, vals) = line.split_once(" :").unwrap();
        let h: Vec<&str> = head.split_whitespace().collect();
        let vals: Vec<f64> = vals.split_whitespace().map(|t| f64::from_bits(t.parse::<i64>().unwrap() as u64)).collect();
        let sum = compensated_sum(vals.iter().copied());
        assert_eq!(double_to_long_bits(sum), h[0].parse::<i64>().unwrap(), "{line}");
        let avg = compensated_average(vals.iter().copied());
        let want_avg = if h[1] == "none" { None } else { Some(h[1].parse::<i64>().unwrap()) };
        assert_eq!(avg.map(|a| double_to_long_bits(a)), want_avg, "{line}");
        let st: CompensatedSum = vals.iter().copied().collect();
        assert_eq!(double_to_long_bits(st.sum()), h[2].parse::<i64>().unwrap(), "{line}");
        n += 1;
    }
    assert_eq!(n, 1500);
}

// ---------------------------------------------------------------------------------------------
// Double.toString / Float.toString

#[test]
fn double_to_string_matches_jdk() {
    let text = data("dtoa.txt");
    let mut bad = vec![];
    let mut n = 0;
    for line in text.lines() {
        let (bits, want) = line.split_once(' ').unwrap();
        let v = f64::from_bits(bits.parse::<i64>().unwrap() as u64);
        let got = double_to_string(v);
        if got != want {
            bad.push(format!("{bits} ({v:e}): got {got}, want {want}"));
        }
        n += 1;
    }
    assert!(bad.is_empty(), "{} mismatches of {n}:\n{}", bad.len(), bad[..bad.len().min(30)].join("\n"));
}

#[test]
fn float_to_string_matches_jdk() {
    let text = data("ftoa.txt");
    let mut bad = vec![];
    for line in text.lines() {
        let (bits, want) = line.split_once(' ').unwrap();
        let v = f32::from_bits(bits.parse::<i32>().unwrap() as u32);
        let got = float_to_string(v);
        if got != want {
            bad.push(format!("{bits} ({v:e}): got {got}, want {want}"));
        }
    }
    assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad[..bad.len().min(30)].join("\n"));
}

// ---------------------------------------------------------------------------------------------
// Math.round / Math.rint

#[test]
fn round_matches_jdk() {
    let text = data("round.txt");
    for line in text.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t[0] == "d" {
            let v = f64::from_bits(t[1].parse::<i64>().unwrap() as u64);
            assert_eq!(java_round_f64(v), t[2].parse::<i64>().unwrap(), "{line}");
            assert_eq!(java_rint(v).to_bits() as i64, t[3].parse::<i64>().unwrap(), "{line}");
        } else {
            let v = f32::from_bits(t[1].parse::<i32>().unwrap() as u32);
            assert_eq!(java_round_f32(v), t[2].parse::<i32>().unwrap(), "{line}");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// String.compareTo / compareToIgnoreCase

#[test]
fn string_compare_matches_jdk() {
    let text = data("strcmp.txt");
    let units = |s: &str| -> Vec<u16> {
        if s == "-" {
            vec![]
        } else {
            s.split('.').map(|h| u16::from_str_radix(h, 16).unwrap()).collect()
        }
    };
    let mut bad = vec![];
    for line in text.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        let (a, b) = (units(t[0]), units(t[1]));
        let c: i32 = t[2].parse().unwrap();
        let ci: i32 = t[3].parse().unwrap();
        assert_eq!(java_string_compare_utf16(&a, &b), c, "{line}");
        let sa = String::from_utf16(&a).unwrap();
        let sb = String::from_utf16(&b).unwrap();
        assert_eq!(java_string_compare(&sa, &sb), c, "{line}");
        let got_ci = compare_to_ignore_case_utf16(&a, &b);
        if got_ci != ci {
            bad.push(format!("{sa:?} vs {sb:?}: got {got_ci}, want {ci}"));
        }
        assert_eq!(compare_to_ignore_case(&sa, &sb), got_ci);
    }
    assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.join("\n"));
}
