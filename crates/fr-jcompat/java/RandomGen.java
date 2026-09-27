// Ground-truth generator for fr-jcompat: java.util.Random and Collections.shuffle.
// Run: reference/jdk25/bin/java crates/fr-jcompat/java/RandomGen.java > crates/fr-jcompat/tests/data/random.txt
//
// Line formats (all values decimal; doubles/floats as raw bit patterns):
//   int <seed> : v...                   20 x nextInt()
//   bound <seed> <bound> : v...         20 x nextInt(bound)
//   range <seed> <origin> <bound> : v... 20 x nextInt(origin, bound)
//   long <seed> : v...                  20 x nextLong()
//   double <seed> : bits...             20 x nextDouble()
//   float <seed> : bits...              20 x nextFloat()
//   bool <seed> : 0/1...                40 x nextBoolean()
//   mixed <seed> : v...                 interleaved nextInt(7), nextLong(), nextBoolean(), nextDouble() (bits)
//   reseed <seed1> <seed2> : v...       new Random(seed1); nextInt(); setSeed(seed2); 10 x nextInt()
//   shuffle <seed> <n> : v...           Collections.shuffle(ArrayList 0..n-1, new Random(seed))
//   shuffle2 <seed> <n> : v...          two consecutive shuffles with the same Random
import java.util.*;

public class RandomGen {
  public static void main(String[] args) {
    long[] seeds = {0L, 1L, 42L, -1L, 7L, 123456789L, Long.MIN_VALUE, Long.MAX_VALUE, 0x5DEECE66DL,
        -987654321987L, 31415926535L};
    int[] bounds = {1, 2, 3, 7, 10, 16, 64, 100, 1000, 1 << 20, (1 << 30) + 1, (1 << 30), 1_500_000_000,
        Integer.MAX_VALUE, 17, 255, 256, 257};
    StringBuilder sb = new StringBuilder();
    for (long s : seeds) {
      Random r = new Random(s);
      sb.append("int ").append(s).append(" :");
      for (int i = 0; i < 20; i++) sb.append(' ').append(r.nextInt());
      sb.append('\n');
      for (int b : bounds) {
        r = new Random(s);
        sb.append("bound ").append(s).append(' ').append(b).append(" :");
        for (int i = 0; i < 20; i++) sb.append(' ').append(r.nextInt(b));
        sb.append('\n');
      }
      int[][] ranges = {{0, 10}, {-5, 5}, {-1000000, 1000000}, {Integer.MIN_VALUE, Integer.MAX_VALUE},
          {-2000000000, 2000000000}, {3, 4}, {0, 1 << 16}, {Integer.MIN_VALUE, 0}};
      for (int[] rg : ranges) {
        r = new Random(s);
        sb.append("range ").append(s).append(' ').append(rg[0]).append(' ').append(rg[1]).append(" :");
        for (int i = 0; i < 20; i++) sb.append(' ').append(r.nextInt(rg[0], rg[1]));
        sb.append('\n');
      }
      r = new Random(s);
      sb.append("long ").append(s).append(" :");
      for (int i = 0; i < 20; i++) sb.append(' ').append(r.nextLong());
      sb.append('\n');
      r = new Random(s);
      sb.append("double ").append(s).append(" :");
      for (int i = 0; i < 20; i++) sb.append(' ').append(Double.doubleToRawLongBits(r.nextDouble()));
      sb.append('\n');
      r = new Random(s);
      sb.append("float ").append(s).append(" :");
      for (int i = 0; i < 20; i++) sb.append(' ').append(Float.floatToRawIntBits(r.nextFloat()));
      sb.append('\n');
      r = new Random(s);
      sb.append("bool ").append(s).append(" :");
      for (int i = 0; i < 40; i++) sb.append(' ').append(r.nextBoolean() ? 1 : 0);
      sb.append('\n');
      r = new Random(s);
      sb.append("mixed ").append(s).append(" :");
      for (int i = 0; i < 10; i++) {
        sb.append(' ').append(r.nextInt(7));
        sb.append(' ').append(r.nextLong());
        sb.append(' ').append(r.nextBoolean() ? 1 : 0);
        sb.append(' ').append(Double.doubleToRawLongBits(r.nextDouble()));
      }
      sb.append('\n');
      r = new Random(s);
      r.nextInt();
      r.setSeed(s * 31 + 5);
      sb.append("reseed ").append(s).append(' ').append(s * 31 + 5).append(" :");
      for (int i = 0; i < 10; i++) sb.append(' ').append(r.nextInt());
      sb.append('\n');
    }
    for (long s : seeds) {
      for (int n : new int[] {0, 1, 2, 3, 5, 10, 17, 64, 100, 257}) {
        List<Integer> l = new ArrayList<>();
        for (int i = 0; i < n; i++) l.add(i);
        Random r = new Random(s);
        Collections.shuffle(l, r);
        sb.append("shuffle ").append(s).append(' ').append(n).append(" :");
        for (int v : l) sb.append(' ').append(v);
        sb.append('\n');
        l.clear();
        for (int i = 0; i < n; i++) l.add(i);
        r = new Random(s);
        Collections.shuffle(l, r);
        Collections.shuffle(l, r);
        sb.append("shuffle2 ").append(s).append(' ').append(n).append(" :");
        for (int v : l) sb.append(' ').append(v);
        sb.append('\n');
      }
    }
    System.out.print(sb);
  }
}
