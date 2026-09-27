// Ground-truth generator for fr-jcompat: java.util.TreeMap / TreeSet with (non-transitive) comparators.
// Run: reference/jdk25/bin/java crates/fr-jcompat/java/TreeMapGen.java > crates/fr-jcompat/tests/data/treemap.txt
//
// Each case is a random operation script with the observed results; the Rust test replays the script.
//   CASE <map|set> <cmp> <seed>
//   put k v -> old|null          (map)       add k -> true|false        (set)
//   rem k -> old|null            (map)       rem k -> true|false        (set)
//   get k -> v|null              (map)       has k -> true|false        (set)
//   ceil|floor|higher|lower k -> key|null
//   pf|pl -> k:v|null (map) / k|null (set)   pollFirst / pollLast
//   first|last -> key|null
//   iterrem m -> visited keys ('-' if none)   iterate ascending, iterator.remove() when key % m == 0 (map: or value % m == 0)
//   dump -> size ; k:v,... (map) / k,... (set)
//   END
import java.util.*;

public class TreeMapGen {
  static int sign(int x) { return Integer.compare(x, 0); }

  static Comparator<Integer> comparator(String name) {
    switch (name) {
      case "nat": return Integer::compare;
      // tolerance comparator: equal when within 5 -> not transitive
      case "tol": return (a, b) -> a < b - 5 ? -1 : (a > b + 5 ? 1 : 0);
      // antisymmetric but not transitive (tournament)
      case "xor": return (a, b) -> {
        if (a.intValue() == b.intValue()) return 0;
        int h = ((a ^ b) * 0x9E3779B9) >>> 7;
        int s = (h & 1) == 0 ? 1 : -1;
        return a < b ? -s : s;
      };
      // not even antisymmetric: argument order of calls matters
      case "asym": return (a, b) -> Math.floorMod(a * 7 + b * 3, 5) - 2;
      default: throw new IllegalArgumentException(name);
    }
  }

  static String s(Object o) { return o == null ? "null" : o.toString(); }

  public static void main(String[] args) {
    StringBuilder sb = new StringBuilder();
    String[] cmps = {"nat", "tol", "xor", "asym"};
    int caseNo = 0;
    for (String mode : new String[] {"map", "set"}) {
      for (String cn : cmps) {
        for (int rep = 0; rep < 25; rep++) {
          long seed = 1000L * (caseNo++) + 17;
          Random r = new Random(seed);
          int keyRange = new int[] {20, 60, 200, 1000}[rep % 4];
          int nOps = 150 + r.nextInt(250);
          sb.append("CASE ").append(mode).append(' ').append(cn).append(' ').append(seed).append('\n');
          Comparator<Integer> c = comparator(cn);
          TreeMap<Integer, Integer> map = new TreeMap<>(c);
          TreeSet<Integer> set = new TreeSet<>(c);
          boolean isMap = mode.equals("map");
          for (int op = 0; op < nOps; op++) {
            int w = r.nextInt(100);
            int k = r.nextInt(keyRange) - keyRange / 4;
            if (w < 45) {
              if (isMap) {
                int v = r.nextInt(1000);
                sb.append("put ").append(k).append(' ').append(v).append(" -> ").append(s(map.put(k, v)));
              } else {
                sb.append("add ").append(k).append(" -> ").append(set.add(k));
              }
            } else if (w < 58) {
              if (isMap) sb.append("rem ").append(k).append(" -> ").append(s(map.remove(k)));
              else sb.append("rem ").append(k).append(" -> ").append(set.remove(k));
            } else if (w < 62) {
              if (isMap) sb.append("get ").append(k).append(" -> ").append(s(map.get(k)));
              else sb.append("has ").append(k).append(" -> ").append(set.contains(k));
            } else if (w < 78) {
              String[] q = {"ceil", "floor", "higher", "lower"};
              int qi = r.nextInt(4);
              Integer res;
              if (isMap) {
                res = switch (qi) {
                  case 0 -> map.ceilingKey(k);
                  case 1 -> map.floorKey(k);
                  case 2 -> map.higherKey(k);
                  default -> map.lowerKey(k);
                };
              } else {
                res = switch (qi) {
                  case 0 -> set.ceiling(k);
                  case 1 -> set.floor(k);
                  case 2 -> set.higher(k);
                  default -> set.lower(k);
                };
              }
              sb.append(q[qi]).append(' ').append(k).append(" -> ").append(s(res));
            } else if (w < 84) {
              boolean first = r.nextBoolean();
              sb.append(first ? "pf" : "pl").append(" -> ");
              if (isMap) {
                Map.Entry<Integer, Integer> e = first ? map.pollFirstEntry() : map.pollLastEntry();
                sb.append(e == null ? "null" : e.getKey() + ":" + e.getValue());
              } else {
                sb.append(s(first ? set.pollFirst() : set.pollLast()));
              }
            } else if (w < 88) {
              boolean first = r.nextBoolean();
              Integer res;
              if (isMap) res = map.isEmpty() ? null : (first ? map.firstKey() : map.lastKey());
              else res = set.isEmpty() ? null : (first ? set.first() : set.last());
              sb.append(first ? "first" : "last").append(" -> ").append(s(res));
            } else if (w < 92) {
              int m = 2 + r.nextInt(4);
              sb.append("iterrem ").append(m).append(" -> ");
              StringBuilder vis = new StringBuilder();
              if (isMap) {
                Iterator<Map.Entry<Integer, Integer>> it = map.entrySet().iterator();
                while (it.hasNext()) {
                  Map.Entry<Integer, Integer> e = it.next();
                  if (vis.length() > 0) vis.append(',');
                  vis.append(e.getKey());
                  if (Math.floorMod(e.getKey(), m) == 0 || Math.floorMod(e.getValue(), m) == 0) it.remove();
                }
              } else {
                Iterator<Integer> it = set.iterator();
                while (it.hasNext()) {
                  int e = it.next();
                  if (vis.length() > 0) vis.append(',');
                  vis.append(e);
                  if (Math.floorMod(e, m) == 0) it.remove();
                }
              }
              sb.append(vis.length() == 0 ? "-" : vis);
            } else {
              sb.append("dump -> ");
              StringBuilder d = new StringBuilder();
              if (isMap) {
                sb.append(map.size()).append(" ; ");
                for (Map.Entry<Integer, Integer> e : map.entrySet()) {
                  if (d.length() > 0) d.append(',');
                  d.append(e.getKey()).append(':').append(e.getValue());
                }
              } else {
                sb.append(set.size()).append(" ; ");
                for (int e : set) {
                  if (d.length() > 0) d.append(',');
                  d.append(e);
                }
              }
              sb.append(d.length() == 0 ? "-" : d);
            }
            sb.append('\n');
          }
          // final dump
          sb.append("dump -> ");
          StringBuilder d = new StringBuilder();
          if (isMap) {
            sb.append(map.size()).append(" ; ");
            for (Map.Entry<Integer, Integer> e : map.entrySet()) {
              if (d.length() > 0) d.append(',');
              d.append(e.getKey()).append(':').append(e.getValue());
            }
          } else {
            sb.append(set.size()).append(" ; ");
            for (int e : set) {
              if (d.length() > 0) d.append(',');
              d.append(e);
            }
          }
          sb.append(d.length() == 0 ? "-" : d).append('\n');
          sb.append("END\n");
        }
      }
    }
    System.out.print(sb);
  }
}
