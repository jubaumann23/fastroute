// Ground-truth generator for fr-jcompat: iteration order of java.util.HashMap<Integer, Integer>.
// Run: reference/jdk25/bin/java crates/fr-jcompat/java/HashMapGen.java > crates/fr-jcompat/tests/data/hashmap.txt
//
//   CASE <dist> <seed> <initialCapacity or -1 for the default constructor>
//   put k v -> old|null
//   pia k v -> old|null        putIfAbsent
//   cia k v -> value           computeIfAbsent(k, x -> v)
//   rem k -> old|null
//   get k -> v|null
//   clear
//   dump -> size ; k:v,...     (entrySet iteration order)
//   END
import java.util.*;

public class HashMapGen {
  static String s(Object o) { return o == null ? "null" : o.toString(); }

  static int key(String dist, Random r, int n) {
    switch (dist) {
      case "seq": return r.nextInt(n);
      case "m64": return 64 * r.nextInt(n);
      case "m1k": return 1024 * r.nextInt(n) - 4096;
      case "hi": return (r.nextInt(n) << 16) | r.nextInt(4);
      case "rand": return r.nextInt();
      case "bin0": return r.nextInt(n) * 256 + (r.nextInt(8) == 0 ? r.nextInt(256) : 0);
      default: {
        int w = r.nextInt(4);
        return w == 0 ? r.nextInt(n) : w == 1 ? 128 * r.nextInt(n) : w == 2 ? r.nextInt() : -r.nextInt(n);
      }
    }
  }

  public static void main(String[] args) {
    StringBuilder sb = new StringBuilder();
    String[] dists = {"seq", "m64", "m1k", "hi", "rand", "bin0", "mix"};
    int caseNo = 0;
    for (String dist : dists) {
      for (int rep = 0; rep < 16; rep++) {
        long seed = 7919L * (caseNo++) + 3;
        Random r = new Random(seed);
        int initCap = (rep % 4 == 3) ? r.nextInt(100) : -1;
        int n = new int[] {12, 40, 150, 600}[rep % 4];
        int nOps = 200 + r.nextInt(600);
        // heavier removal in some cases
        int remW = rep % 2 == 0 ? 10 : 30;
        sb.append("CASE ").append(dist).append(' ').append(seed).append(' ').append(initCap).append('\n');
        HashMap<Integer, Integer> map = initCap < 0 ? new HashMap<>() : new HashMap<>(initCap);
        for (int op = 0; op < nOps; op++) {
          int w = r.nextInt(100);
          int k = key(dist, r, n);
          if (w < 30) {
            int v = r.nextInt(1000);
            sb.append("put ").append(k).append(' ').append(v).append(" -> ").append(s(map.put(k, v)));
          } else if (w < 40) {
            int v = r.nextInt(1000);
            sb.append("pia ").append(k).append(' ').append(v).append(" -> ").append(s(map.putIfAbsent(k, v)));
          } else if (w < 65) {
            int v = r.nextInt(1000);
            sb.append("cia ").append(k).append(' ').append(v).append(" -> ").append(s(map.computeIfAbsent(k, x -> v)));
          } else if (w < 65 + remW) {
            // remove mostly existing keys
            if (!map.isEmpty() && r.nextInt(3) != 0) {
              int idx = r.nextInt(map.size());
              Iterator<Integer> it = map.keySet().iterator();
              for (int i = 0; i < idx; i++) it.next();
              k = it.next();
            }
            sb.append("rem ").append(k).append(" -> ").append(s(map.remove(k)));
          } else if (w < 97) {
            sb.append("get ").append(k).append(" -> ").append(s(map.get(k)));
          } else if (w < 98 && r.nextInt(4) == 0) {
            map.clear();
            sb.append("clear");
          } else {
            sb.append("dump -> ").append(dump(map));
          }
          sb.append('\n');
        }
        sb.append("dump -> ").append(dump(map)).append('\n');
        sb.append("END\n");
      }
    }
    System.out.print(sb);
  }

  static String dump(HashMap<Integer, Integer> map) {
    StringBuilder d = new StringBuilder();
    d.append(map.size()).append(" ; ");
    boolean any = false;
    for (Map.Entry<Integer, Integer> e : map.entrySet()) {
      if (any) d.append(',');
      any = true;
      d.append(e.getKey()).append(':').append(e.getValue());
    }
    if (!any) d.append('-');
    return d.toString();
  }
}
