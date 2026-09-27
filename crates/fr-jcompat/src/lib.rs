//! Bit-exact emulations of the Java library behaviour the Freerouting engine depends on.
//!
//! Every item documents the JDK class/method it reproduces. Ground truth for the tests is
//! generated with a real JDK 25 by the programs in `java/` (see `tests/data/`).
//!
//! | Module | Java |
//! |---|---|
//! | [`random`] | `java.util.Random`, `Collections.shuffle(List, Random)` |
//! | [`treemap`] | `java.util.TreeMap`, `java.util.TreeSet` (exact red-black shape, works with non-transitive comparators) |
//! | [`hashmap`] | iteration order of `java.util.HashMap<Integer, V>` / `HashSet<Integer>` (incl. tree bins) |
//! | [`sum`] | `DoubleStream.sum()/average()`, `DoubleSummaryStatistics`, `Collectors.summingDouble` |
//! | [`string`] | `String.compareTo`, `String.compareToIgnoreCase` |
//! | [`math`] | `Math.round`, `Math.rint`, `(int)` casts, `Double.toString`, `Float.toString` |
//! | [`int`] | shift operators, `Integer/Long/Double/Float.hashCode` |
//! | `bigint` (feature) | `BigInteger.hashCode/intValue/doubleValue/signum` |

pub mod hashmap;
pub mod int;
pub mod math;
pub mod random;
pub mod string;
pub mod sum;
pub mod treemap;

#[cfg(feature = "bigint")]
pub mod bigint;

pub use hashmap::{JavaIntHashMap, JavaIntHashSet};
pub use int::{
    arrays_hash_code_i32, boolean_hash_code, double_hash_code, double_to_long_bits, float_hash_code,
    float_to_int_bits, string_hash_code, int_hash_code, java_shl, java_shl_long, java_shr, java_shr_long,
    java_ushr, java_ushr_long, long_hash_code,
};
pub use math::{
    d2i, d2l, double_to_string, float_to_string, java_rint, java_round_f32, java_round_f64, math_rint,
    math_round, math_round_i32, INT_MAX_F64, INT_MIN_F64,
};
pub use random::{shuffle, JavaRandom};
pub use string::{compare_to_ignore_case, java_string_compare};
pub use sum::{compensated_average, compensated_sum, CompensatedSum};
pub use treemap::{EntryId, JavaComparator, JavaTreeMap, JavaTreeSet, TreeCursor};
