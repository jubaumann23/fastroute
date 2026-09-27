// Ground-truth generator for fr-jcompat numeric/string helpers.
// Run: reference/jdk25/bin/java crates/fr-jcompat/java/MiscGen.java <kind> > crates/fr-jcompat/tests/data/<kind>.txt
// kinds:
//   sum      : "<sumBits> <avgBits|none> <statsSumBits> : bits..."  DoubleStream.of(a).sum(), .average(),
//                                                                    DoubleSummaryStatistics.getSum()
//   dtoa     : "<doubleBits> <Double.toString>"
//   ftoa     : "<floatBits> <Float.toString>"
//   round    : "d <doubleBits> <Math.round(double)> <Math.rint bits>"  /  "f <floatBits> <Math.round(float)>"
//   strcmp   : "<hexA> <hexB> <compareTo> <compareToIgnoreCase>"   (strings as UTF-16 units in hex, '-' if empty)
import java.util.*;
import java.util.stream.*;

public class MiscGen {
  public static void main(String[] args) {
    String kind = args[0];
    StringBuilder sb = new StringBuilder();
    switch (kind) {
      case "sum" -> sum(sb);
      case "dtoa" -> dtoa(sb);
      case "ftoa" -> ftoa(sb);
      case "round" -> round(sb);
      case "strcmp" -> strcmp(sb);
      default -> throw new IllegalArgumentException(kind);
    }
    System.out.print(sb);
  }

  static double randDouble(Random r) {
    switch (r.nextInt(8)) {
      case 0: return Double.longBitsToDouble(r.nextLong());
      case 1: return r.nextDouble() * Math.pow(10, r.nextInt(40) - 20) * (r.nextBoolean() ? 1 : -1);
      case 2: return (r.nextInt(2000000) - 1000000) / 100.0;
      case 3: return (r.nextInt(2000000) - 1000000) / 1000.0 * Math.pow(10, r.nextInt(10));
      case 4: return r.nextInt() * 1.0e-4;
      case 5: return Math.sqrt(r.nextInt(1000000));
      case 6: return r.nextGaussian() * 1e6;
      default: return (double) (float) (r.nextDouble() * 1000);
    }
  }

  static void sum(StringBuilder sb) {
    Random r = new Random(4242);
    for (int c = 0; c < 1500; c++) {
      int n = r.nextInt(c < 100 ? 5 : 60);
      double[] a = new double[n];
      int style = r.nextInt(6);
      for (int i = 0; i < n; i++) {
        switch (style) {
          case 0 -> a[i] = r.nextDouble();
          case 1 -> a[i] = r.nextDouble() * Math.pow(10, r.nextInt(33) - 16) * (r.nextBoolean() ? 1 : -1);
          case 2 -> a[i] = i % 2 == 0 ? 1e16 : 1.0;
          case 3 -> a[i] = (r.nextInt(2000) - 1000) / 10.0;
          case 4 -> {
            int w = r.nextInt(20);
            a[i] = w == 0 ? Double.POSITIVE_INFINITY : w == 1 ? Double.NEGATIVE_INFINITY
                : w == 2 ? Double.NaN : w == 3 ? Double.MAX_VALUE : r.nextDouble() * 1e300;
          }
          default -> a[i] = randDouble(r);
        }
      }
      double sum = DoubleStream.of(a).sum();
      OptionalDouble avg = DoubleStream.of(a).average();
      DoubleSummaryStatistics st = new DoubleSummaryStatistics();
      for (double d : a) st.accept(d);
      sb.append(Double.doubleToRawLongBits(sum)).append(' ')
          .append(avg.isPresent() ? Long.toString(Double.doubleToRawLongBits(avg.getAsDouble())) : "none").append(' ')
          .append(Double.doubleToRawLongBits(st.getSum())).append(" :");
      for (double d : a) sb.append(' ').append(Double.doubleToRawLongBits(d));
      sb.append('\n');
    }
  }

  static List<Double> specialDoubles() {
    List<Double> l = new ArrayList<>(List.of(0.0, -0.0, 1.0, -1.0, Double.NaN, Double.POSITIVE_INFINITY,
        Double.NEGATIVE_INFINITY, Double.MIN_VALUE, -Double.MIN_VALUE, Double.MAX_VALUE, Double.MIN_NORMAL,
        0.1, 0.2, 0.3, 0.1 + 0.2, 1e-3, 0.001, 9.99e-4, 0.0009999999999999998, 1e7, 9999999.0, 9999999.999999998,
        1e7 - 1e-9, 1.0E23, 2e23, 8.41e21, 5e-324, 1e-323, 2.2250738585072014E-308, 4.9e-324, 1.7976931348623157E308,
        123456789.0, 1234567.0, 12345678.0, 0.5, 100.0, 1e21, 1e22, 1e-5, 3.0e-10, 2.0E-3, 9.9e-324,
        4.35, 0.49999999999999994, 1.0 / 3, 2.0 / 3, Math.PI, Math.E, 1e16, 1e17, 1e15, 123e-7));
    for (int e = -325; e <= 309; e++) {
      l.add(Double.parseDouble("1e" + e));
      l.add(Double.parseDouble("2e" + e));
      l.add(Double.parseDouble("5e" + e));
      l.add(Double.parseDouble("9e" + e));
    }
    for (int e = -1074; e <= 1023; e += 1) {
      l.add(Math.scalb(1.0, e));
      l.add(Math.nextUp(Math.scalb(1.0, e)));
      l.add(Math.nextDown(Math.scalb(1.0, e)));
    }
    for (long i = 0; i < 300; i++) l.add(Double.longBitsToDouble(i));
    return l;
  }

  static void dtoa(StringBuilder sb) {
    List<Double> l = specialDoubles();
    Random r = new Random(777);
    for (int i = 0; i < 40000; i++) l.add(randDouble(r));
    for (double d : l) sb.append(Double.doubleToRawLongBits(d)).append(' ').append(Double.toString(d)).append('\n');
  }

  static void ftoa(StringBuilder sb) {
    List<Float> l = new ArrayList<>(List.of(0.0f, -0.0f, 1.0f, Float.NaN, Float.POSITIVE_INFINITY,
        Float.NEGATIVE_INFINITY, Float.MIN_VALUE, Float.MAX_VALUE, Float.MIN_NORMAL, 0.1f, 1e7f, 9999999f,
        1e-3f, 9.999999e-4f, 2e-45f, 3.4e38f, 1.0E10f, 0.3f));
    for (int e = -46; e <= 39; e++) {
      l.add(Float.parseFloat("1e" + e));
      l.add(Float.parseFloat("2e" + e));
      l.add(Float.parseFloat("7e" + e));
    }
    for (int e = -149; e <= 127; e++) l.add(Math.scalb(1.0f, e));
    for (int i = 0; i < 300; i++) l.add(Float.intBitsToFloat(i));
    Random r = new Random(778);
    for (int i = 0; i < 20000; i++) {
      l.add(r.nextBoolean() ? Float.intBitsToFloat(r.nextInt()) : (float) randDouble(r));
    }
    for (float f : l) sb.append(Float.floatToRawIntBits(f)).append(' ').append(Float.toString(f)).append('\n');
  }

  static void round(StringBuilder sb) {
    List<Double> l = specialDoubles();
    Random r = new Random(779);
    for (int i = 0; i < 2000; i++) {
      l.add(randDouble(r));
      l.add(r.nextInt(2000) / 2.0 - 500);
      l.add(Math.nextUp(r.nextInt(2000) / 2.0 - 500));
      l.add(Math.nextDown(r.nextInt(2000) / 2.0 - 500));
    }
    for (double d : l) {
      sb.append("d ").append(Double.doubleToRawLongBits(d)).append(' ').append(Math.round(d)).append(' ')
          .append(Double.doubleToRawLongBits(Math.rint(d))).append('\n');
    }
    for (double d : l) {
      float f = (float) d;
      sb.append("f ").append(Float.floatToRawIntBits(f)).append(' ').append(Math.round(f)).append('\n');
    }
    for (int i = 0; i < 3000; i++) {
      float f = Float.intBitsToFloat(r.nextInt());
      sb.append("f ").append(Float.floatToRawIntBits(f)).append(' ').append(Math.round(f)).append('\n');
    }
  }

  static String hex(String s) {
    if (s.isEmpty()) return "-";
    StringBuilder b = new StringBuilder();
    for (int i = 0; i < s.length(); i++) {
      if (i > 0) b.append('.');
      b.append(Integer.toHexString(s.charAt(i)));
    }
    return b.toString();
  }

  static void strcmp(StringBuilder sb) {
    List<String> base = new ArrayList<>(List.of("", "a", "A", "b", "B", "abc", "ABC", "abd", "ab", "abcd", "Net1",
        "net1", "NET10", "net2", "GND", "gnd", "Vcc", "VCC", "_x", "[x", "`x", "{x", "été", "ÉTÉ",
        "ß", "SS", "µ", "Μ", "μ", "ÿ", "Ÿ", "ı", "I", "i", "İ", "ſ", "s",
        "S", "😀", "😁", "￿", "中文", "pad_1", "Pad_1", "PAD_2", "xé", "xĀ",
        "ẞ", "Ω", "ω", "K", "k"));
    Random r = new Random(780);
    String alphabet = "aAbBzZ09_-.éÉßıIſ中";
    for (int i = 0; i < 300; i++) {
      int n = r.nextInt(6);
      StringBuilder b = new StringBuilder();
      for (int j = 0; j < n; j++) b.append(alphabet.charAt(r.nextInt(alphabet.length())));
      base.add(b.toString());
    }
    for (String a : base.subList(0, 52)) {
      for (String b : base.subList(0, 52)) {
        sb.append(hex(a)).append(' ').append(hex(b)).append(' ').append(a.compareTo(b)).append(' ')
            .append(a.compareToIgnoreCase(b)).append('\n');
      }
    }
    for (int i = 52; i + 1 < base.size(); i++) {
      String a = base.get(i), b = base.get(i + 1);
      sb.append(hex(a)).append(' ').append(hex(b)).append(' ').append(a.compareTo(b)).append(' ')
          .append(a.compareToIgnoreCase(b)).append('\n');
    }
  }
}
