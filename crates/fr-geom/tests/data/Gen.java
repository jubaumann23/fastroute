package app.freerouting.geometry.planar;

import java.math.BigInteger;
import java.util.*;

/** Golden vector generator for the Rust port (differential testing). */
public class Gen {
  static Random rnd;
  static StringBuilder out = new StringBuilder();

  static String d(double v) { return Long.toHexString(Double.doubleToRawLongBits(v)); }
  static String pt(Point p) {
    if (p == null) return "N";
    if (p instanceof IntPoint ip) return "I " + ip.x + " " + ip.y;
    RationalPoint r = (RationalPoint) p;
    return "R " + r.x + " " + r.y + " " + r.z;
  }
  static String fp(FloatPoint p) { return p == null ? "N" : "F " + d(p.x) + " " + d(p.y); }
  static String ln(Line l) { return "L " + pt(l.a) + " " + pt(l.b); }
  static String oct(IntOctagon o) {
    return "O " + o.leftX + " " + o.bottomY + " " + o.rightX + " " + o.topY + " " + o.upperLeftDiagonalX + " "
        + o.lowerRightDiagonalX + " " + o.lowerLeftDiagonalX + " " + o.upperRightDiagonalX + " " + (o.isEmpty() ? 1 : 0);
  }
  static String box(IntBox b) { return "B " + b.ll.x + " " + b.ll.y + " " + b.ur.x + " " + b.ur.y; }
  static String tile(TileShape t) {
    if (t == null) return "N";
    if (t instanceof IntBox b) return box(b);
    if (t instanceof IntOctagon o) return oct(o);
    Simplex s = (Simplex) t;
    StringBuilder sb = new StringBuilder("S " + s.borderLineCount());
    for (int i = 0; i < s.borderLineCount(); i++) sb.append(" ").append(ln(s.borderLine(i)));
    return sb.toString();
  }
  static String tiles(TileShape[] ts) {
    if (ts == null) return "N";
    StringBuilder sb = new StringBuilder("A " + ts.length);
    for (TileShape t : ts) sb.append(" ").append(tile(t));
    return sb.toString();
  }
  static String dir(Direction dd) {
    if (dd == null) return "N";
    if (dd instanceof IntDirection id) return "D " + id.x + " " + id.y;
    BigIntDirection b = (BigIntDirection) dd;
    return "BD " + b.x + " " + b.y;
  }

  interface Case { String run(); }
  @SuppressWarnings("removal")
  static void emit(String op, String in, Case c) {
    final String[] res = new String[1];
    Thread th = new Thread(() -> {
      try { res[0] = c.run(); } catch (Throwable t) { res[0] = "EXC"; }
    });
    th.start();
    try { th.join(5000); } catch (InterruptedException e) { throw new RuntimeException(e); }
    if (th.isAlive()) {
      th.stop(); // the Java code under test loops forever for this input
      res[0] = "HANG";
    }
    out.append(op).append(' ').append(in).append(" | ").append(res[0]).append('\n');
  }

  static int ri(int lo, int hi) { return (int) (lo + (long) Math.floor(rnd.nextDouble() * ((long) hi - lo + 1))); }
  static IntPoint rp(int r) { return new IntPoint(ri(-r, r), ri(-r, r)); }
  static final int[][] DIRS45 = {{1,0},{1,1},{0,1},{-1,1},{-1,0},{-1,-1},{0,-1},{1,-1}};
  static Line rline(int r, boolean only45) {
    IntPoint a = rp(r);
    IntPoint b;
    if (only45 || rnd.nextInt(3) == 0) {
      int[] dd = DIRS45[rnd.nextInt(8)];
      int k = ri(1, 50);
      b = new IntPoint(a.x + dd[0] * k, a.y + dd[1] * k);
    } else {
      do { b = rp(r); } while (b.equals(a));
    }
    return new Line(a, b);
  }
  static IntBox rbox(int r) {
    int x1 = ri(-r, r), x2 = ri(-r, r), y1 = ri(-r, r), y2 = ri(-r, r);
    return new IntBox(Math.min(x1, x2), Math.min(y1, y2), Math.max(x1, x2), Math.max(y1, y2));
  }
  static IntOctagon roct(int r) {
    if (rnd.nextInt(5) == 0) {
      return new IntOctagon(ri(-r, r), ri(-r, r), ri(-r, r), ri(-r, r), ri(-2 * r, 2 * r), ri(-2 * r, 2 * r), ri(-2 * r, 2 * r), ri(-2 * r, 2 * r));
    }
    IntBox b = rbox(r);
    int w = Math.max(1, (b.ur.x - b.ll.x) + (b.ur.y - b.ll.y));
    return new IntOctagon(b.ll.x, b.ll.y, b.ur.x, b.ur.y, b.ll.x - b.ur.y + ri(0, w / 4), b.ur.x - b.ll.y - ri(0, w / 4),
        b.ll.x + b.ll.y + ri(0, w / 4), b.ur.x + b.ur.y - ri(0, w / 4));
  }
  /** Random convex polygon (points on an ellipse, counter clockwise). */
  static Point[] rconvex(int r, int n) {
    int cx = ri(-r, r), cy = ri(-r, r);
    int rx = ri(5, r), ry = ri(5, r);
    double[] ang = new double[n];
    for (int i = 0; i < n; i++) ang[i] = rnd.nextDouble() * 2 * Math.PI;
    Arrays.sort(ang);
    Point[] p = new Point[n];
    for (int i = 0; i < n; i++) p[i] = new IntPoint(cx + (int) Math.round(rx * Math.cos(ang[i])), cy + (int) Math.round(ry * Math.sin(ang[i])));
    return p;
  }
  static TileShape rtile(int r) {
    switch (rnd.nextInt(4)) {
      case 0: return rbox(r);
      case 1: return roct(r).normalize();
      case 2: return TileShape.getInstance(rconvex(r, ri(3, 7)));
      default: {
        Line[] ls = new Line[ri(3, 6)];
        for (int i = 0; i < ls.length; i++) ls[i] = rline(r, false);
        return Simplex.getInstance(ls);
      }
    }
  }
  static String tin(TileShape t) { return tile(t); }

  public static void main(String[] args) {
    rnd = new Random(Long.parseLong(args[0]));
    int n = Integer.parseInt(args[1]);

    for (int k = 0; k < n; k++) {
      // Line intersection, side, compare
      int r = rnd.nextInt(4) == 0 ? 30000000 : 1000;
      Line l1 = rline(r, false), l2 = rline(r, false);
      IntPoint p = rp(r);
      double tr = ri(-20, 20) + 0.5;
      emit("LINE", ln(l1) + " " + ln(l2) + " " + pt(p) + " " + d(tr), () ->
          pt(l1.intersection(l2)) + " " + fp(l1.intersectionApprox(l2)) + " " + l1.sideOf(p) + " " + l1.compareTo(l2)
          + " " + l1.isParallel(l2) + " " + l1.equals(l2) + " " + dir(l1.direction()) + " " + d(l1.signedDistance(p.toFloat()))
          + " " + pt(p.perpendicularProjection(l1)) + " " + dir(l1.perpendicularDirection(p)) + " " + ln(l1.translate(tr)));
    }
    for (int k = 0; k < n; k++) {
      Line[] ls;
      if (rnd.nextInt(3) == 0) {
        int m = ri(1, 7);
        ls = new Line[m];
        boolean only45 = rnd.nextBoolean();
        for (int i = 0; i < m; i++) ls[i] = rline(200, only45);
      } else {
        Point[] cp = rconvex(200, ri(3, 7));
        int extra = ri(0, 2);
        ls = new Line[cp.length + extra];
        for (int i = 0; i < cp.length; i++) ls[i] = new Line(cp[i], cp[(i + 1) % cp.length]);
        for (int i = 0; i < extra; i++) ls[cp.length + i] = rline(200, rnd.nextBoolean());
        for (int i = ls.length - 1; i > 0; i--) { int j = rnd.nextInt(i + 1); Line t = ls[i]; ls[i] = ls[j]; ls[j] = t; }
        if (rnd.nextInt(6) == 0) ls[0] = ls[0].opposite();
      }
      final int m = ls.length;
      StringBuilder in = new StringBuilder("" + m);
      for (Line l : ls) in.append(' ').append(ln(l));
      emit("SIMPLEX", in.toString(), () -> {
        Simplex s = Simplex.getInstance(ls);
        StringBuilder sb = new StringBuilder(tile(s) + " " + s.dimension() + " " + s.isBounded() + " " + tile(s.simplify()));
        sb.append(" C");
        for (int i = 0; i < s.borderLineCount(); i++) sb.append(' ').append(pt(s.corner(i))).append(' ').append(fp(s.cornerApprox(i)));
        sb.append(" ").append(box(s.boundingBox()));
        IntOctagon bo = s.boundingOctagon();
        sb.append(" ").append(bo == null ? "N" : oct(bo));
        sb.append(" ").append(d(s.area())).append(" ").append(d(s.circumference()));
        return sb.toString();
      });
    }
    for (int k = 0; k < n; k++) {
      TileShape a = rtile(300), b = rtile(300);
      emit("TILE2", tin(a) + " " + tin(b), () -> {
        StringBuilder sb = new StringBuilder();
        sb.append(tile(a.intersection(b))).append(' ').append(tile(a.intersectionWithSimplify(b)));
        sb.append(' ').append(a.intersects(b)).append(' ').append(tiles(a.cutout(b)));
        sb.append(' ').append(a.contains(b)).append(' ').append(Arrays.toString(a.touchingSides(b)).replace(" ", ""));
        return sb.toString();
      });
    }
    for (int k = 0; k < n; k++) {
      TileShape a = rtile(300);
      IntPoint p = rp(400);
      double off = ri(-30, 30) + (rnd.nextBoolean() ? 0.5 : 0.0);
      int fac = ri(-5, 5);
      IntVector tv = new IntVector(ri(-9, 9), ri(-9, 9));
      emit("TILE1", tin(a) + " " + pt(p) + " " + d(off) + " " + fac + " " + tv.x + " " + tv.y, () -> {
        StringBuilder sb = new StringBuilder();
        sb.append(a.dimension()).append(' ').append(d(a.area())).append(' ').append(fp(a.centreOfGravity()));
        sb.append(' ').append(a.contains(p)).append(' ').append(a.containsInside(p)).append(' ').append(a.isOutside(p));
        sb.append(' ').append(a.containsOnBorderLineNo(p));
        sb.append(' ').append(pt(a.nearestBorderPoint(p)));
        FloatPoint[] nb = a.nearestBorderPointsApprox(p.toFloat(), 2);
        sb.append(" NB ").append(nb.length);
        for (FloatPoint f : nb) sb.append(' ').append(fp(f));
        sb.append(' ').append(d(a.distance(p.toFloat())));
        sb.append(' ').append(tile((TileShape) a.offset(off)));
        sb.append(' ').append(tile((TileShape) a.shrink(Math.abs(off))));
        sb.append(' ').append(tile((TileShape) a.enlarge(Math.abs(off))));
        sb.append(' ').append(tile(a.turn90Degree(fac, p)));
        sb.append(' ').append(tile(a.mirrorVertical(p))).append(' ').append(tile(a.mirrorHorizontal(p)));
        sb.append(' ').append(tile((TileShape) a.translateBy(tv)));
        sb.append(' ').append(d(a.length())).append(' ').append(d(a.maxWidth())).append(' ').append(d(a.minWidth()));
        sb.append(' ').append(tile(a.boundingBox())).append(' ').append(a.boundingOctagon() == null ? "N" : oct(a.boundingOctagon()));
        sb.append(' ').append(a.getId());
        return sb.toString();
      });
    }
    for (int k = 0; k < n; k++) {
      IntOctagon a = roct(100), b = roct(100);
      IntBox bx = rbox(100);
      double off = ri(-20, 20) + (rnd.nextBoolean() ? 0.5 : 0.0);
      IntPoint c = new IntPoint(ri(-100, 100), ri(-100, 100));
      emit("OCT", oct(a) + " " + oct(b) + " " + box(bx) + " " + d(off) + " " + pt(c), () -> {
        IntOctagon na = a.normalize(), nb = b.normalize();
        StringBuilder sb = new StringBuilder();
        sb.append(oct(na)).append(' ').append(oct(nb)).append(' ').append(oct(na.intersection(nb)));
        sb.append(' ').append(na.intersects(nb)).append(' ').append(na.overlaps(nb)).append(' ').append(oct(na.offset(off)));
        sb.append(' ').append(tiles(na.cutoutFrom(nb))).append(' ').append(tiles(na.cutoutFrom(bx)));
        sb.append(' ').append(tiles(bx.cutoutFrom(bx.intersection(na.boundingBox()))));
        sb.append(' ').append(tile(na.union(nb))).append(' ').append(na.isIntBox()).append(' ').append(d(na.area()));
        sb.append(' ').append(tile(na.toSimplex())).append(' ').append(a.isEmpty()).append(' ').append(a.dimension());
        IntPoint[] pr = na.nearestBorderProjections(c, 3);
        sb.append(" P ").append(pr.length);
        for (IntPoint q : pr) sb.append(' ').append(pt(q));
        for (FortyfiveDegreeDirection fd : FortyfiveDegreeDirection.values()) sb.append(' ').append(pt(na.borderPoint(c, fd)));
        return sb.toString();
      });
    }
    for (int k = 0; k < n; k++) {
      int m = ri(2, 8);
      Point[] pts = new Point[m];
      boolean only45 = rnd.nextBoolean();
      pts[0] = rp(500);
      for (int i = 1; i < m; i++) {
        if (only45) {
          int[] dd = DIRS45[rnd.nextInt(8)];
          int kk = ri(1, 80);
          IntPoint q = (IntPoint) pts[i - 1];
          pts[i] = new IntPoint(q.x + dd[0] * kk, q.y + dd[1] * kk);
        } else {
          pts[i] = rp(500);
        }
      }
      int hw = ri(1, 30);
      IntPoint q = rp(500);
      TileShape t = rtile(400);
      StringBuilder in = new StringBuilder("" + m);
      for (Point qq : pts) in.append(' ').append(pt(qq));
      emit("POLYLINE", in + " " + hw + " " + pt(q) + " " + tile(t), () -> {
        Polyline pl = new Polyline(pts);
        StringBuilder sb = new StringBuilder("" + pl.lines.length);
        for (Line l : pl.lines) sb.append(' ').append(ln(l));
        if (pl.lines.length >= 3) {
          sb.append(" C");
          for (int i = 0; i < pl.cornerCount(); i++) sb.append(' ').append(pt(pl.corner(i)));
          sb.append(' ').append(d(pl.lengthApprox())).append(' ').append(box(pl.boundingBox()));
          sb.append(' ').append(tiles(pl.offsetShapes(hw)));
          Polyline rev = pl.reverse();
          sb.append(" R ").append(rev.lines.length);
          for (Line l : rev.lines) sb.append(' ').append(ln(l));
          LineSegment ls = pl.projectionLine(q);
          sb.append(' ').append(ls == null ? "N" : pt(ls.startPoint()) + " " + pt(ls.endPoint()));
          sb.append(' ').append(fp(pl.nearestPointApprox(q.toFloat())));
          Polyline[] cut = t.cutout(pl);
          sb.append(" CUT ").append(cut.length);
          for (Polyline c : cut) { sb.append(" [").append(c.lines.length); for (Line l : c.lines) sb.append(' ').append(ln(l)); sb.append(" ]"); }
          int[][] ep = t.entrancePoints(pl);
          sb.append(" EP ").append(ep.length);
          for (int[] e : ep) sb.append(' ').append(e[0]).append(' ').append(e[1]);
        }
        return sb.toString();
      });
    }
    for (int k = 0; k < n; k++) {
      int m = ri(3, 9);
      Point[] pts = new Point[m];
      for (int i = 0; i < m; i++) pts[i] = rp(300);
      StringBuilder in = new StringBuilder("" + m);
      for (Point q : pts) in.append(' ').append(pt(q));
      emit("POLYGON", in.toString(), () -> {
        PolygonShape ps = new PolygonShape(pts);
        StringBuilder sb = new StringBuilder("" + ps.corners.length);
        for (Point q : ps.corners) sb.append(' ').append(pt(q));
        sb.append(' ').append(ps.isConvex()).append(' ').append(tiles(ps.splitToConvex()));
        PolygonShape h = ps.convexHull();
        sb.append(" H ").append(h.corners.length);
        for (Point q : h.corners) sb.append(' ').append(pt(q));
        sb.append(' ').append(tile(ps.boundingTile())).append(' ').append(new Polygon(pts).windingNumberAfterClosing());
        return sb.toString();
      });
    }
    for (int k = 0; k < n; k++) {
      double v;
      switch (rnd.nextInt(4)) {
        case 0: v = ri(-1000, 1000) + 0.5; break;
        case 1: v = (rnd.nextDouble() - 0.5) * 1e7; break;
        case 2: v = Math.nextUp(ri(-100, 100) + 0.5) * (rnd.nextBoolean() ? 1 : -1); break;
        default: v = (rnd.nextDouble() - 0.5) * 1e12;
      }
      double vv = v;
      emit("ROUND", d(vv), () -> Math.round(vv) + " " + (int) Math.round(vv) + " " + d(Math.rint(vv)) + " " + (int) vv);
    }
    for (int k = 0; k < n; k++) {
      IntPoint c = rp(1000);
      int rad = ri(0, 3000);
      int ms = ri(50, 2000);
      emit("CIRCLE", pt(c) + " " + rad + " " + ms, () -> {
        Circle ci = new Circle(c, rad);
        return oct(ci.boundingOctagon()) + " " + tile(ci.boundingTile(ms)) + " " + box(ci.boundingBox());
      });
    }
    for (int k = 0; k < n; k++) {
      int r = rnd.nextInt(3) == 0 ? 2000000000 : 100;
      IntVector v1 = new IntVector(ri(-r, r), ri(-r, r)), v2 = new IntVector(ri(-r, r), ri(-r, r));
      int tf = ri(-9, 9);
      emit("DIR", v1.x + " " + v1.y + " " + v2.x + " " + v2.y + " " + tf, () -> {
        Direction d1 = Direction.getInstance(v1), d2 = Direction.getInstance(v2);
        return dir(d1) + " " + dir(d2) + " " + d1.compareTo(d2) + " " + d1.equals(d2) + " " + d1.sideOf(d2) + " " + d1.projection(d2)
            + " " + dir(d1.turn45Degree(tf)) + " " + dir(d1.middleApprox(d2)) + " " + d(v1.angleApprox()) + " " + v1.sideOf(v2);
      });
    }
    for (int k = 0; k < n; k++) {
      Line s = rline(300, false), m = rline(300, false), e = rline(300, false);
      TileShape t = rtile(300);
      double w = ri(1, 40) + 0.25;
      boolean right = rnd.nextBoolean();
      emit("LSEG", ln(s) + " " + ln(m) + " " + ln(e) + " " + tile(t) + " " + d(w) + " " + right, () -> {
        LineSegment ls = new LineSegment(s, m, e);
        StringBuilder sb = new StringBuilder(pt(ls.startPoint()) + " " + pt(ls.endPoint()) + " " + box(ls.boundingBox()) + " " + oct(ls.boundingOctagon()));
        sb.append(" BI ").append(Arrays.toString(ls.borderIntersections(t)).replace(" ", ""));
        sb.append(' ').append(t.isIntersectedInteriorBy(ls));
        IntPoint[] st = ls.stairApproximation(w, right);
        sb.append(" ST ").append(st.length);
        for (IntPoint q : st) sb.append(' ').append(pt(q));
        IntPoint[] st45 = ls.stairApproximation45(w, right);
        sb.append(" ST45 ").append(st45.length);
        for (IntPoint q : st45) sb.append(' ').append(pt(q));
        sb.append(' ').append(tile(ls.toSimplex()));
        return sb.toString();
      });
    }
    for (int k = 0; k < n; k++) {
      int m = ri(2, 120);
      Line[] ls = new Line[m];
      for (int i = 0; i < m; i++) {
        int kind = rnd.nextInt(10);
        if (kind == 0) { IntPoint a = rp(100); ls[i] = new Line(a, a); }
        else if (kind == 1) ls[i] = new Line(rp(2000000000), rp(2000000000));
        else if (kind < 5) { IntPoint a = rp(100); int[] dd = DIRS45[rnd.nextInt(8)]; ls[i] = new Line(a, new IntPoint(a.x + dd[0], a.y + dd[1])); }
        else ls[i] = rline(1000, false);
      }
      StringBuilder in = new StringBuilder("" + m);
      for (Line l : ls) in.append(' ').append(ln(l));
      emit("SORT", in.toString(), () -> {
        Line[] c = ls.clone();
        Arrays.sort(c);
        StringBuilder sb = new StringBuilder();
        for (Line l : c) { int idx = -1; for (int i = 0; i < ls.length; i++) if (ls[i] == l) idx = i; sb.append(idx).append(' '); }
        return sb.toString().trim();
      });
    }
    System.out.print(out);
  }
}
