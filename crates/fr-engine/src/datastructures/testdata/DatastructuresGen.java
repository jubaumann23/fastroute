// Ground-truth generator for fr-engine::datastructures, calling the real Freerouting classes.
// Run from the workspace root:
//   D=crates/fr-engine/src/datastructures/testdata
//   reference/jdk25/bin/javac -cp reference/bin/freerouting-2.4.1.jar -d /tmp/dsgen $D/DatastructuresGen.java
//   for k in tree delaunay identifier indent; do
//     reference/jdk25/bin/java -cp reference/bin/freerouting-2.4.1.jar:/tmp/dsgen DatastructuresGen $k > $D/$k.txt
//   done
//
// Formats (one record per line, tokens separated by single spaces):
//   tree:       "T <dirs>" starts a scenario (dirs = ortho | 45)
//               "I <id> <n> <shape>*n"  insert object id with n shapes; shape = B lx ly rx uy |
//                                       O lx ly rx uy ulx lrx llx urx
//               "R <id>"                remove all entries of object id
//               "Q <shape> : <id:idx>*" overlaps(query bounding shape) in result order
//               "A <id:idx:depth>*"     toArray() with Leaf.distanceToRoot()
//   delaunay:   "D <nobjects>" then per object "P <id> <n> x y ..." then "E <count>" and per edge
//               "<startId> x y <endId> x y" (startId/endId -1 for null)
//   identifier: "S <quoteHex> <reservedHex,...> <inputHex> <outputBytesHex>" (UTF-16 units in hex
//               4 digits each; '-' for empty)
//   indent:     "W <ops> <outputBytesHex>" ops: s1 s0 e n w<text-hex>
import app.freerouting.datastructures.*;
import app.freerouting.geometry.planar.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.*;

public class DatastructuresGen {
  static StringBuilder out = new StringBuilder();

  public static void main(String[] args) throws Exception {
    switch (args[0]) {
      case "tree" -> tree();
      case "delaunay" -> delaunay();
      case "identifier" -> identifier();
      case "indent" -> indent();
      default -> throw new IllegalArgumentException(args[0]);
    }
    System.out.print(out);
  }

  // ------------------------------------------------------------------------------------------
  static final class Obj implements ShapeTree.Storable {
    final int id;
    final TileShape[] shapes;
    ShapeTree.Leaf[] entries;

    Obj(int id, TileShape[] shapes) {
      this.id = id;
      this.shapes = shapes;
    }

    public int compareTo(Object o) {
      return Integer.compare(id, ((Obj) o).id);
    }

    public int treeShapeCount(ShapeTree t) {
      return shapes.length;
    }

    public TileShape getTreeShape(ShapeTree t, int i) {
      return shapes[i];
    }

    public void setSearchTreeEntries(ShapeTree.Leaf[] e, ShapeTree t) {
      entries = e;
    }
  }

  static TileShape randomShape(Random r, boolean allowOctagon, int range, int maxSize) {
    int lx = r.nextInt(2 * range) - range;
    int ly = r.nextInt(2 * range) - range;
    int w = r.nextInt(8) == 0 ? 0 : r.nextInt(maxSize);
    int h = r.nextInt(8) == 0 ? 0 : r.nextInt(maxSize);
    int rx = lx + w;
    int uy = ly + h;
    if (allowOctagon && r.nextBoolean()) {
      int s = (w + h) / 2 + 1;
      IntOctagon o =
          new IntOctagon(
                  lx,
                  ly,
                  rx,
                  uy,
                  lx - uy + r.nextInt(s),
                  rx - ly - r.nextInt(s),
                  lx + ly + r.nextInt(s),
                  rx + uy - r.nextInt(s))
              .normalize();
      if (!o.isEmpty()) {
        return o;
      }
    }
    return new IntBox(lx, ly, rx, uy);
  }

  static String shapeStr(TileShape s) {
    if (s instanceof IntBox b) {
      return "B " + b.ll.x + " " + b.ll.y + " " + b.ur.x + " " + b.ur.y;
    }
    IntOctagon o = (IntOctagon) s;
    return "O " + o.leftX + " " + o.bottomY + " " + o.rightX + " " + o.topY + " "
        + o.upperLeftDiagonalX + " " + o.lowerRightDiagonalX + " " + o.lowerLeftDiagonalX + " "
        + o.upperRightDiagonalX;
  }

  static void tree() {
    int scenario = 0;
    for (boolean fortyfive : new boolean[] {false, true, false, true}) {
      for (int range : new int[] {1000, 100000}) {
        Random r = new Random(1234 + scenario++);
        ShapeBoundingDirections dirs =
            fortyfive
                ? FortyfiveDegreeBoundingDirections.INSTANCE
                : OrthogonalBoundingDirections.INSTANCE;
        MinAreaTree tree = new MinAreaTree(dirs);
        out.append("T ").append(fortyfive ? "45" : "ortho").append('\n');
        TreeMap<Integer, Obj> live = new TreeMap<>();
        int nextId = 1;
        int steps = 700;
        for (int step = 0; step < steps; step++) {
          int op = r.nextInt(10);
          if (op < 6 || live.isEmpty()) {
            int n = 1 + r.nextInt(3);
            TileShape[] shapes = new TileShape[n];
            for (int i = 0; i < n; i++) {
              shapes[i] = randomShape(r, fortyfive, range, range / 5);
            }
            Obj o = new Obj(nextId++, shapes);
            tree.insert(o);
            live.put(o.id, o);
            out.append("I ").append(o.id).append(' ').append(n);
            for (TileShape s : shapes) {
              out.append(' ').append(shapeStr(s));
            }
            out.append('\n');
          } else if (op < 8) {
            int k = r.nextInt(live.size());
            Integer id = new ArrayList<>(live.keySet()).get(k);
            Obj o = live.remove(id);
            tree.remove(o.entries);
            out.append("R ").append(id).append('\n');
          } else {
            TileShape q = randomShape(r, fortyfive, range, range / 2);
            RegularTileShape rq = q.boundingShape(dirs);
            out.append("Q ").append(shapeStr(q)).append(" :");
            for (ShapeTree.Leaf l : tree.overlaps(rq)) {
              out.append(' ').append(((Obj) l.object).id).append(':').append(l.shapeIndexInObject);
            }
            out.append('\n');
          }
          if (step % 50 == 49 || step == steps - 1) {
            out.append('A');
            for (ShapeTree.Leaf l : tree.toArray()) {
              int depth = tree.size() == 1 ? 0 : l.distanceToRoot();
              out.append(' ')
                  .append(((Obj) l.object).id)
                  .append(':')
                  .append(l.shapeIndexInObject)
                  .append(':')
                  .append(depth);
            }
            out.append('\n');
          }
        }
      }
    }
  }

  // ------------------------------------------------------------------------------------------
  static final class DObj implements PlanarDelaunayTriangulation.Storable {
    final int id;
    final Point[] corners;

    DObj(int id, Point[] corners) {
      this.id = id;
      this.corners = corners;
    }

    public Point[] getTriangulationCorners() {
      return corners;
    }
  }

  static void delaunay() {
    Random r = new Random(4711);
    int[][] configs = {
      // objects, maxCornersPerObject, coordinate range, grid step
      {1, 1, 100, 1},
      {2, 1, 100, 1},
      {3, 1, 100, 1},
      {4, 2, 10, 1},
      {10, 3, 20, 5},
      {20, 2, 1000, 1},
      {30, 4, 50, 10},
      {60, 3, 100000, 1},
      {100, 2, 30, 3},
      {200, 3, 1000000, 1},
      {300, 1, 5000, 250},
      {400, 2, 10000000, 1},
    };
    for (int rep = 0; rep < 3; rep++) {
      for (int[] cfg : configs) {
        emitDelaunay(r, cfg[0], cfg[1], cfg[2], cfg[3]);
      }
    }
    // collinear points
    List<DObj> line = new ArrayList<>();
    for (int i = 0; i < 12; i++) {
      line.add(new DObj(i, new Point[] {new IntPoint(i * 100, i * 50)}));
    }
    runDelaunay(line);
    // grid (many cocircular points) incl. duplicates of other objects
    List<DObj> grid = new ArrayList<>();
    int id = 0;
    for (int x = 0; x < 6; x++) {
      for (int y = 0; y < 6; y++) {
        grid.add(new DObj(id++, new Point[] {new IntPoint(x * 1000, y * 1000)}));
      }
    }
    grid.add(new DObj(id++, new Point[] {new IntPoint(2000, 2000), new IntPoint(0, 0)}));
    runDelaunay(grid);
  }

  static void emitDelaunay(Random r, int objects, int maxCorners, int range, int step) {
    List<DObj> list = new ArrayList<>();
    for (int i = 0; i < objects; i++) {
      int n = 1 + r.nextInt(maxCorners);
      Point[] pts = new Point[n];
      for (int k = 0; k < n; k++) {
        if (k > 0 && r.nextInt(4) == 0) {
          pts[k] = pts[k - 1]; // repeated corner of the same object
        } else if (!list.isEmpty() && r.nextInt(10) == 0) {
          DObj other = list.get(r.nextInt(list.size()));
          pts[k] = other.corners[0]; // corner shared with another object
        } else {
          pts[k] =
              new IntPoint(
                  (r.nextInt(2 * range + 1) - range) / step * step,
                  (r.nextInt(2 * range + 1) - range) / step * step);
        }
      }
      list.add(new DObj(i, pts));
    }
    runDelaunay(list);
  }

  static void runDelaunay(List<DObj> list) {
    out.append("D ").append(list.size()).append('\n');
    for (DObj o : list) {
      out.append("P ").append(o.id).append(' ').append(o.corners.length);
      for (Point p : o.corners) {
        IntPoint ip = (IntPoint) p;
        out.append(' ').append(ip.x).append(' ').append(ip.y);
      }
      out.append('\n');
    }
    Collection<PlanarDelaunayTriangulation.Storable> objs = new LinkedList<>(list);
    PlanarDelaunayTriangulation t = new PlanarDelaunayTriangulation(objs);
    Collection<PlanarDelaunayTriangulation.ResultEdge> edges = t.getEdgeLines();
    out.append("E ").append(edges.size()).append('\n');
    for (PlanarDelaunayTriangulation.ResultEdge e : edges) {
      IntPoint s = (IntPoint) e.startPoint;
      IntPoint en = (IntPoint) e.endPoint;
      out.append(e.startObject == null ? -1 : ((DObj) e.startObject).id)
          .append(' ')
          .append(s.x)
          .append(' ')
          .append(s.y)
          .append(' ')
          .append(e.endObject == null ? -1 : ((DObj) e.endObject).id)
          .append(' ')
          .append(en.x)
          .append(' ')
          .append(en.y)
          .append('\n');
    }
  }

  // ------------------------------------------------------------------------------------------
  static String hex16(String s) {
    if (s.isEmpty()) {
      return "-";
    }
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < s.length(); i++) {
      sb.append(String.format("%04x", (int) s.charAt(i)));
    }
    return sb.toString();
  }

  static String hexBytes(byte[] b) {
    if (b.length == 0) {
      return "-";
    }
    StringBuilder sb = new StringBuilder();
    for (byte x : b) {
      sb.append(String.format("%02x", x & 0xff));
    }
    return sb.toString();
  }

  static void identifier() throws Exception {
    String[][] reservedSets = {
      {"(", ")", " ", ";", "-", "_", "/", "~", "{", "}"},
      {"(", ")", " ", ";", "-", "_"},
      {"(", ")", " ", "-"},
      {"ab", "é"},
    };
    String[] quotes = {"\"", "'", "", "$$"};
    String[] fixed = {
      "", "a", "\"", "\"\"", "\"\"\"", "\"a\"", "\"ab\"", "\"abc\"", "\"\"x\"\"", "\"\"\"\"\"\"",
      "600", "-600", "-", "--1", "-a", "1", "1\n", "1\r", "1\u0085", "1 ", "1 x", "\n1",
      "600a", "test", "test-with-reserved", "R1", "C_12", "U$1", "a'b", "'a'", "$$a$$", "a$$$b",
      "é", "été", "a\u0000", "😀", "\"a😀\"", "x\ud83d", "\ude00y",
      "\"😀\"", "1\ud83d", "GND", "+5V", "Net-(U1-Pad3)", "/sheet/net", "a b", "a;b",
      "١٢", "１", "٣x", "\t1", " 1", "1 ", "0x10", "9", "-9z",
    };
    List<String> inputs = new ArrayList<>(Arrays.asList(fixed));
    Random r = new Random(99);
    char[] alphabet = {
      'a', 'Z', '0', '5', '-', '_', '"', '\'', '$', '(', ')', ' ', ';', '/', '~', '{', '}', '\n',
      '\r', 'é', ' ', '\u0000', '\ud83d', '\ude00', '+', '.'
    };
    for (int i = 0; i < 400; i++) {
      int len = r.nextInt(8);
      StringBuilder sb = new StringBuilder();
      for (int k = 0; k < len; k++) {
        sb.append(alphabet[r.nextInt(alphabet.length)]);
      }
      if (r.nextInt(3) == 0) {
        sb.insert(0, '"').append('"');
      }
      inputs.add(sb.toString());
    }
    for (int si = 0; si < reservedSets.length; si++) {
      for (String q : quotes) {
        IdentifierType it = new IdentifierType(reservedSets[si], q);
        StringBuilder res = new StringBuilder();
        for (int k = 0; k < reservedSets[si].length; k++) {
          if (k > 0) {
            res.append(',');
          }
          res.append(hex16(reservedSets[si][k]));
        }
        for (String in : inputs) {
          ByteArrayOutputStream baos = new ByteArrayOutputStream();
          OutputStreamWriter w = new OutputStreamWriter(baos, StandardCharsets.UTF_8);
          it.write(in, w);
          w.flush();
          out.append("S ")
              .append(hex16(q))
              .append(' ')
              .append(res)
              .append(' ')
              .append(hex16(in))
              .append(' ')
              .append(hexBytes(baos.toByteArray()))
              .append('\n');
        }
      }
    }
  }

  static void indent() throws Exception {
    Random r = new Random(7);
    String[] texts = {"session x.ses", "(", ")", "é", "a b", "", "😀", "\n"};
    for (int c = 0; c < 60; c++) {
      ByteArrayOutputStream baos = new ByteArrayOutputStream();
      IndentFileWriter w = new IndentFileWriter(baos);
      StringBuilder ops = new StringBuilder();
      int n = 1 + r.nextInt(30);
      for (int k = 0; k < n; k++) {
        if (k > 0) {
          ops.append(',');
        }
        switch (r.nextInt(5)) {
          case 0 -> {
            w.startScope(true);
            ops.append("s1");
          }
          case 1 -> {
            w.startScope(false);
            ops.append("s0");
          }
          case 2 -> {
            w.endScope();
            ops.append("e");
          }
          case 3 -> {
            w.newLine();
            ops.append("n");
          }
          default -> {
            String t = texts[r.nextInt(texts.length)];
            w.write(t);
            ops.append('w').append(hex16(t));
          }
        }
      }
      w.flush();
      out.append("W ").append(ops).append(' ').append(hexBytes(baos.toByteArray())).append('\n');
    }
  }
}
