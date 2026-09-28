// Ground-truth generator for fr-engine::board::{routing_board, optimize, actions, undo} (porting
// unit U7), using the real Freerouting classes of reference/bin/freerouting-parity.jar (built from
// the reference source). Extends ../board/BoardGen.java (same default package, compile together).
//
// Regenerate (from the workspace root; time limits disabled like the Rust side):
//   J=reference/jdk25/bin; D=crates/fr-engine/testdata; F=reference/freerouting/fixtures; P=reference/parity-baseline
//   $J/javac -cp reference/bin/freerouting-parity.jar -d /tmp/routinggen $D/board/BoardGen.java $D/routing/RoutingGen.java
//   run() { $J/java -Dfreerouting.parity.disableTimeLimits=true -cp reference/bin/freerouting-parity.jar:/tmp/routinggen RoutingGen "$@"; }
//   (see crates/fr-engine/testdata/routing/README.txt for the vector list)
//
// Arguments: <dsn> <ses or -> <angle: 45|90|none|keep> <seed> <scale: number of operations per phase>
//            [expand]
//
// Board A is loaded with DsnReader (+ SesReader). A RoutingBoard B with the same rules, library and
// components is built like in BoardGen, the routed traces of A are inserted as whole polylines
// (TRACENC), the vias with insertVia, and normalizeAllTraces is called. Then phases of routing
// board operations are executed (pull tight, smoothen, via optimization, optChangedArea, forced
// traces / vias, shove checks, drill item moves, tails, snapshots and undo). Results are written as
// "= " lines; board dumps (items + all search trees, hashed) follow the mutating operations.

import app.freerouting.autoroute.maze.AutorouteControl.ExpansionCostFactor;
import app.freerouting.board.actions.DrillItemMover;
import app.freerouting.board.actions.ForcedPadRouter;
import app.freerouting.board.actions.ForcedViaInserter;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.*;
import app.freerouting.board.model.structure.*;
import app.freerouting.board.optimize.TraceShover;
import app.freerouting.board.optimize.ViaOptimizer;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.geometry.planar.*;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.io.specctra.SesReader;
import app.freerouting.rules.*;
import java.io.*;
import java.util.*;

public class RoutingGen {
  static RoutingBoard R;
  static Random rnd;
  static int scale;
  static int opsSinceDump = 0;
  static int snapDepth = 0;

  public static void main(String[] args) throws Exception {
    BoardGen.out = new PrintStream(new BufferedOutputStream(new FileOutputStream(FileDescriptor.out), 1 << 16), false, "UTF-8");
    System.setOut(new PrintStream(OutputStream.nullOutputStream()));
    String dsn = args[0];
    String ses = args[1];
    String angle = args[2];
    rnd = new Random(Long.parseLong(args[3]));
    BoardGen.rnd = rnd;
    scale = Integer.parseInt(args[4]);
    BoardGen.full = false;
    BoardGen.expandAll = args.length > 5 && args[5].equals("expand");
    BoardReadResult r = DsnReader.readBoard(new FileInputStream(dsn), null, null, new File(dsn).getName());
    app.freerouting.board.facade.BasicBoard a = ((BoardReadResult.Success) r).board();
    if (!ses.equals("-")) {
      SesReader.read(new FileInputStream(ses), a);
    }
    switch (angle) {
      case "45" -> a.rules.setTraceAngleRestriction(AngleRestriction.FORTYFIVE_DEGREE);
      case "90" -> a.rules.setTraceAngleRestriction(AngleRestriction.NINETY_DEGREE);
      case "none" -> a.rules.setTraceAngleRestriction(AngleRestriction.NONE);
      default -> {}
    }
    List<Item> aItems = new ArrayList<>(a.getItems());
    aItems.sort(Comparator.comparingInt(Item::getId));
    op("MODE brief");
    op("VARIANT source");
    BoardGen.emitSetup(a);
    op("TRACEHW " + a.rules.getMaxTraceHalfWidth() + " " + a.rules.getMinTraceHalfWidth());

    BoardGen.setField(a.communication, "idGenerator", new ItemIdGenerator());
    BoardOutline ao = a.getOutline();
    PolylineShape[] outlineShapes = new PolylineShape[ao.shapeCount()];
    for (int i = 0; i < outlineShapes.length; i++) {
      outlineShapes[i] = ao.getShape(i);
    }
    R = new RoutingBoard(a.boundingBox, a.layerStructure, outlineShapes, ao.clearanceClassIndex(), a.rules, a.communication);
    BoardGen.setField(R, "library", a.library);
    BoardGen.setField(R, "components", a.components);
    BoardGen.B = R;
    StringBuilder sb = new StringBuilder("BOARD ").append(outlineShapes.length).append(' ').append(ao.clearanceClassIndex());
    for (PolylineShape s : outlineShapes) {
      sb.append(' ').append(BoardGen.shape(s));
    }
    op(sb.toString());
    if (a.searchTreeManager.isClearanceCompensationUsed()) {
      op("COMPENSATION");
      R.searchTreeManager.setClearanceCompensationUsed(true);
    }
    if (ao.keepoutOutsideOutlineGenerated()) {
      op("KEEPOUT");
      R.getOutline().generateKeepoutOutside(true);
    }
    for (Item it : aItems) {
      if (it instanceof BoardOutline || it instanceof Trace || it instanceof Via) {
        continue;
      }
      BoardGen.insertCopy(it);
    }
    for (Item it : aItems) {
      if (it instanceof PolylineTrace t) {
        // routed wires of a SES are user fixed: insert them unfixed like autorouted traces
        FixedState fs = t.getFixedState() == FixedState.SYSTEM_FIXED ? FixedState.SYSTEM_FIXED : FixedState.UNFIXED;
        BoardGen.traceOp("TRACENC", t.polyline().lines, t.getLayer(), t.getHalfWidth(), t.netNumbers, t.clearanceClassIndex(), fs);
      }
    }
    for (Item it : aItems) {
      if (it instanceof Via v) {
        FixedState fs = v.getFixedState() == FixedState.SYSTEM_FIXED ? FixedState.SYSTEM_FIXED : FixedState.UNFIXED;
        op("VIA " + v.getPadstack().id + " " + BoardGen.pt(v.getCenter()) + " " + BoardGen.nets(v.netNumbers) + " " + v.clearanceClassIndex() + " "
            + fs.ordinal() + " " + BoardGen.b(v.attachAllowed));
        R.insertVia(v.getPadstack(), v.getCenter(), v.netNumbers, v.clearanceClassIndex(), fs, v.attachAllowed);
      }
    }
    op("NORMALIZE_ALL");
    res(BoardGen.b(R.normalizeAllTraces()));
    // an autoroute search tree (45 / 90 degree tree compensated for the clearance class of the
    // traces) like during routing: all tree changes and undo also apply to it
    List<PolylineTrace> initialTraces = traces();
    int autorouteCl = initialTraces.isEmpty() ? 1 : initialTraces.get(0).clearanceClassIndex();
    op("AUTOTREE " + autorouteCl);
    R.searchTreeManager.getAutorouteTree(autorouteCl);
    dump();

    phaseWiggle();
    phasePullTight();
    phaseVias();
    phaseOptArea();
    phaseChecks();
    phaseForcedTraces();
    phaseForcedVias();
    phaseDrill();
    phaseVias();
    phaseMisc();
    // final: optimize everything
    op("MARKALL");
    R.markAllChangedArea();
    ExpansionCostFactor[] costs = randomCosts(true);
    opt(500, new int[0], null, costs);
    dump();
    BoardGen.out.flush();
  }

  // ------------------------------------------------------------------------------------------
  // helpers

  static void op(String s) {
    BoardGen.op(s);
  }

  static void res(String s) {
    BoardGen.res(s);
  }

  static void dump() {
    BoardGen.dump("", false);
    opsSinceDump = 0;
  }

  static void maybeDump() {
    opsSinceDump++;
    // (the draw is always made: -Ddumpall / -Dstepped do not change the random sequence)
    boolean draw = rnd.nextInt(4) == 0;
    if (opsSinceDump >= 3 || draw || Boolean.getBoolean("dumpall")) {
      dump();
    }
  }

  static <T> List<T> sample(Collection<T> items, int max) {
    List<T> list = new ArrayList<>(items);
    if (list.size() <= max) {
      return list;
    }
    List<T> result = new ArrayList<>();
    for (int i = 0; i < max; i++) {
      result.add(list.get(rnd.nextInt(list.size())));
    }
    return result;
  }

  static String costs(ExpansionCostFactor[] c) {
    if (c == null) {
      return "N";
    }
    StringBuilder sb = new StringBuilder("C ").append(c.length);
    for (ExpansionCostFactor f : c) {
      sb.append(' ').append(BoardGen.d(f.horizontal())).append(' ').append(BoardGen.d(f.vertical()));
    }
    return sb.toString();
  }

  static ExpansionCostFactor[] randomCosts(boolean nonNull) {
    if (!nonNull && rnd.nextInt(3) == 0) {
      return null;
    }
    double[] choices = {1.0, 1.5, 2.0, 3.0, 5.0};
    ExpansionCostFactor[] result = new ExpansionCostFactor[R.getLayerCount()];
    for (int i = 0; i < result.length; i++) {
      result[i] = new ExpansionCostFactor(choices[rnd.nextInt(choices.length)], choices[rnd.nextInt(choices.length)]);
    }
    return result;
  }

  static int randomAccuracy() {
    int[] acc = {100, 500, 500, 2000};
    return acc[rnd.nextInt(acc.length)];
  }

  static void snapMaybe() {
    if (rnd.nextInt(3) == 0) {
      op("SNAP");
      R.generateSnapshot();
      snapDepth++;
    }
  }

  static void unsnapMaybe(boolean failed) {
    if (snapDepth == 0) {
      return;
    }
    if (failed || rnd.nextBoolean()) {
      op("UNDO");
      res(BoardGen.b(R.undo(null)));
    } else {
      op("POPSNAP");
      res(BoardGen.b(R.popSnapshot()));
    }
    snapDepth--;
    dump();
  }

  static void shoveFailure() {
    Item f = R.getShoveFailingObstacle();
    op("QSF");
    res((f == null ? "-" : String.valueOf(f.getId())) + " " + R.getShoveFailingLayer());
  }

  static List<PolylineTrace> traces() {
    List<PolylineTrace> result = new ArrayList<>();
    for (Trace t : R.getTraces()) {
      result.add((PolylineTrace) t);
    }
    return result;
  }

  static List<Via> vias() {
    List<Via> result = new ArrayList<>();
    for (Via v : R.getVias()) {
      result.add(v);
    }
    return result;
  }

  static int layerCount() {
    return R.getLayerCount();
  }

  /** A shove vector in a direction allowed by the angle restriction. */
  static IntPoint target(Point from, int unit) {
    IntPoint p = from.toFloat().round();
    int len = unit * (1 + rnd.nextInt(30));
    AngleRestriction ar = R.rules.getTraceAngleRestriction();
    int dx;
    int dy;
    if (ar == AngleRestriction.NONE) {
      double angle = rnd.nextDouble() * 2 * Math.PI;
      dx = (int) Math.round(len * Math.cos(angle));
      dy = (int) Math.round(len * Math.sin(angle));
    } else {
      int dir = rnd.nextInt(8);
      if (ar == AngleRestriction.NINETY_DEGREE) {
        dir = 2 * rnd.nextInt(4);
      }
      int[] ddx = {1, 1, 0, -1, -1, -1, 0, 1};
      int[] ddy = {0, 1, 1, 1, 0, -1, -1, -1};
      dx = ddx[dir] * len;
      dy = ddy[dir] * len;
    }
    return new IntPoint(p.x + dx, p.y + dy);
  }

  /**
   * RoutingBoard.optChangedArea. With -Dstepped=true the loop of TraceTightener.optChangedArea is
   * executed here step by step (same calls, public API) with a board dump after every step.
   */
  static void opt(int acc, int[] onlyNets, IntOctagon clip, ExpansionCostFactor[] c) {
    if (!Boolean.getBoolean("stepped")) {
      op("OPT " + acc + " " + BoardGen.nets(onlyNets) + " " + (clip == null ? "N" : BoardGen.shape(clip)) + " " + costs(c));
      R.optChangedArea(onlyNets, clip, acc, c, null, 0);
      return;
    }
    if (R.changedArea == null) {
      return;
    }
    op("TTNEW " + acc + " " + BoardGen.nets(onlyNets) + " " + (clip == null ? "N" : BoardGen.shape(clip)));
    app.freerouting.board.optimize.TraceTightener algo =
        app.freerouting.board.optimize.TraceTightener.getInstance(R, onlyNets, clip, acc, null, 0, null, 0);
    boolean somethingChanged = true;
    while (somethingChanged) {
      somethingChanged = false;
      for (int i = 0; i < R.getLayerCount(); i++) {
        IntOctagon changedRegion = R.changedArea.getArea(i);
        if (changedRegion.isEmpty()) {
          continue;
        }
        op("TTEMPTY " + i);
        R.changedArea.setEmpty(i);
        double changedAreaOffset = 1.5 * (R.rules.clearanceMatrix.maxValue(i) + 2 * R.rules.getMaxTraceHalfWidth());
        changedRegion = changedRegion.enlarge(changedAreaOffset);
        for (app.freerouting.board.searchtree.SearchTreeObject o : R.overlappingObjects(changedRegion, i)) {
          if (o instanceof PolylineTrace t) {
            op("TTPULL " + t.getId());
            if (String.valueOf(t.getId()).equals(System.getProperty("debugtrace"))) {
              debugPull90(algo, t);
            }
            boolean r = t.pullTight(algo);
            res(BoardGen.b(r));
            dump();
            if (r) {
              somethingChanged = true;
              op("TTKEEP");
              boolean k = algo.splitTracesAtKeepPoint();
              res(BoardGen.b(k));
              if (k) {
                break;
              }
            } else {
              op("TTSMOOTH " + t.getId());
              boolean sm = algo.smoothenEndCornersAtTrace(t);
              res(BoardGen.b(sm));
              dump();
              if (sm) {
                somethingChanged = true;
                break;
              }
            }
          } else if (o instanceof Via v && c != null) {
            op("OPTVIA " + v.getId() + " " + Math.max(acc, 100) + " " + costs(c));
            boolean r = ViaOptimizer.optViaLocation(R, v, c, Math.max(acc, 100), 10);
            res(BoardGen.b(r));
            dump();
            if (r) {
              somethingChanged = true;
            }
          }
        }
      }
    }
    op("ENDMARK");
    R.changedArea = null;
  }

  static String ident(Polyline orig, Polyline p) {
    StringBuilder sb = new StringBuilder("[");
    for (Line l : p.lines) {
      int k = -1;
      for (int i = 0; i < orig.lines.length; i++) {
        if (orig.lines[i] == l) {
          k = i;
        }
      }
      sb.append(k < 0 ? "n" : String.valueOf(k)).append(' ');
    }
    return sb.append(']').toString();
  }

  /** Debug: the stages of TraceTightener90.pullTight with the line identities (stderr). */
  static void debugPull90(app.freerouting.board.optimize.TraceTightener algo, PolylineTrace t) {
    try {
      Class<?> base = app.freerouting.board.optimize.TraceTightener.class;
      java.lang.reflect.Field f;
      f = base.getDeclaredField("currentLayer"); f.setAccessible(true); f.set(algo, t.getLayer());
      f = base.getDeclaredField("currentHalfWidth"); f.setAccessible(true); f.set(algo, t.getHalfWidth());
      f = base.getDeclaredField("currentNetNumbers"); f.setAccessible(true); f.set(algo, t.netNumbers);
      f = base.getDeclaredField("currentClearanceClassIndex"); f.setAccessible(true); f.set(algo, t.clearanceClassIndex());
      f = base.getDeclaredField("contactPins"); f.setAccessible(true); f.set(algo, t.touchingPinsAtEndCorners());
      java.lang.reflect.Method m1 = algo.getClass().getDeclaredMethod("trySkipSecondCorner", Polyline.class); m1.setAccessible(true);
      java.lang.reflect.Method m2 = algo.getClass().getDeclaredMethod("trySkipCorners", Polyline.class); m2.setAccessible(true);
      java.lang.reflect.Method m3 = base.getDeclaredMethod("repositionLines", Polyline.class); m3.setAccessible(true);
      Polyline orig = t.polyline();
      Polyline nr = orig;
      Polyline prev = null;
      while (nr != prev) {
        prev = nr;
        Polyline a = (Polyline) m1.invoke(algo, prev);
        System.err.println("DBG skip2 " + ident(orig, a));
        Polyline b = (Polyline) m2.invoke(algo, a);
        System.err.println("DBG skipc " + ident(orig, b));
        nr = (Polyline) m3.invoke(algo, b);
        System.err.println("DBG repos " + ident(orig, nr));
      }
    } catch (Exception e) {
      throw new RuntimeException(e);
    }
  }

  // ------------------------------------------------------------------------------------------
  // phases

  /** An offset vector in a direction allowed by the angle restriction. */
  static int[] offset(int len) {
    AngleRestriction ar = R.rules.getTraceAngleRestriction();
    if (ar == AngleRestriction.NONE) {
      double angle = rnd.nextDouble() * 2 * Math.PI;
      return new int[] {(int) Math.round(len * Math.cos(angle)), (int) Math.round(len * Math.sin(angle))};
    }
    int dir = ar == AngleRestriction.NINETY_DEGREE ? 2 * rnd.nextInt(4) : rnd.nextInt(8);
    int[] ddx = {1, 1, 0, -1, -1, -1, 0, 1};
    int[] ddy = {0, 1, 1, 1, 0, -1, -1, -1};
    return new int[] {ddx[dir] * len, ddy[dir] * len};
  }

  /** Replaces some traces by detoured copies (bumps on some segments) to give pull tight work. */
  static void phaseWiggle() {
    int unit = Math.max(R.getMinTraceHalfWidth(), 100);
    for (PolylineTrace t : sample(traces(), 3 * scale)) {
      if (!t.isOnTheBoard() || t.isUserFixed() || t.isShoveFixed()) {
        continue;
      }
      Point[] corners = t.polyline().corners();
      boolean allInt = true;
      for (Point c : corners) {
        allInt &= c instanceof IntPoint;
      }
      if (!allInt) {
        continue;
      }
      List<Point> newCorners = new ArrayList<>();
      newCorners.add(corners[0]);
      for (int i = 0; i + 1 < corners.length; i++) {
        IntPoint p1 = (IntPoint) corners[i];
        IntPoint p2 = (IntPoint) corners[i + 1];
        if (rnd.nextInt(3) != 0) {
          int[] d = offset(unit * (2 + rnd.nextInt(12)));
          newCorners.add(new IntPoint(p1.x + d[0], p1.y + d[1]));
          newCorners.add(new IntPoint(p2.x + d[0], p2.y + d[1]));
        }
        newCorners.add(p2);
      }
      Polyline pl = new Polyline(newCorners.toArray(new Point[0]));
      if (pl.lines.length < 3) {
        continue;
      }
      op("REMOVE " + t.getId());
      R.removeItem(t);
      BoardGen.traceOp("TRACE", pl.lines, t.getLayer(), t.getHalfWidth(), t.netNumbers, t.clearanceClassIndex(), t.getFixedState());
      maybeDump();
    }
    dump();
  }

  static void phasePullTight() {
    List<PolylineTrace> byId = traces();
    byId.sort(Comparator.comparingInt((PolylineTrace x) -> -x.getId()));
    List<PolylineTrace> todo = new ArrayList<>(byId.subList(0, Math.min(byId.size(), 4 * scale)));
    todo.addAll(sample(byId, scale));
    for (PolylineTrace t : todo) {
      if (!t.isOnTheBoard()) {
        continue;
      }
      snapMaybe();
      boolean own = rnd.nextBoolean();
      int acc = randomAccuracy();
      if (rnd.nextInt(4) == 0) {
        op("SMOOTHFORK " + t.getId() + " " + BoardGen.b(own) + " " + acc);
        res(BoardGen.b(t.smoothenEndCornersFork(own, acc, null)));
      } else {
        op("PULLTIGHT " + t.getId() + " " + BoardGen.b(own) + " " + acc);
        res(BoardGen.b(t.pullTight(own, acc, null)));
      }
      maybeDump();
      unsnapMaybe(false);
    }
    dump();
  }

  static void phaseVias() {
    for (Via v : sample(vias(), 2 * scale)) {
      if (!v.isOnTheBoard()) {
        continue;
      }
      snapMaybe();
      ExpansionCostFactor[] c = randomCosts(false);
      int acc = randomAccuracy();
      op("OPTVIA " + v.getId() + " " + acc + " " + costs(c));
      res(BoardGen.b(ViaOptimizer.optViaLocation(R, v, c, acc, 10)));
      maybeDump();
      unsnapMaybe(false);
    }
    dump();
  }

  static void phaseOptArea() {
    // whole board, then regions around items with net filters and clip shapes
    op("SNAP");
    R.generateSnapshot();
    op("MARKALL");
    R.markAllChangedArea();
    int acc = randomAccuracy();
    ExpansionCostFactor[] c = randomCosts(false);
    opt(acc, new int[0], null, c);
    dump();
    op("UNDO");
    res(BoardGen.b(R.undo(null)));
    dump();
    for (PolylineTrace t : sample(traces(), scale)) {
      if (!t.isOnTheBoard()) {
        continue;
      }
      snapMaybe();
      op("STARTMARK");
      R.startMarkingChangedArea();
      for (int i = 0; i < t.polyline().cornerCount(); i++) {
        FloatPoint p = t.polyline().cornerApprox(i);
        op("JOINAREA " + BoardGen.d(p.x) + " " + BoardGen.d(p.y) + " " + t.getLayer());
        R.joinChangedArea(p, t.getLayer());
      }
      int[] onlyNets = rnd.nextBoolean() ? new int[0] : t.netNumbers;
      IntOctagon clip = null;
      if (rnd.nextInt(3) == 0) {
        clip = t.boundingBox().toIntOctagon().enlarge(rnd.nextInt(20000));
      }
      acc = randomAccuracy();
      c = randomCosts(false);
      opt(acc, onlyNets, clip, c);
      maybeDump();
      unsnapMaybe(false);
    }
    dump();
  }

  static void phaseChecks() {
    int unit = Math.max(R.getMinTraceHalfWidth(), 100);
    for (PolylineTrace t : sample(traces(), scale)) {
      Point from = rnd.nextBoolean() ? t.firstCorner() : t.lastCorner();
      if (!(from instanceof IntPoint)) {
        continue;
      }
      IntPoint to = target(from, unit);
      int layer = t.getLayer();
      int[] nets = t.netNumbers;
      int hw = t.getHalfWidth();
      int cl = t.clearanceClassIndex();
      boolean onlyNotShovable = rnd.nextBoolean();
      op("CHECKSEG " + BoardGen.pt(from) + " " + BoardGen.pt(to) + " " + layer + " " + BoardGen.nets(nets) + " " + hw + " " + cl + " " + BoardGen.b(onlyNotShovable));
      res(BoardGen.d(R.checkTraceSegment(from, to, layer, nets, hw, cl, onlyNotShovable)));
      if (from.equals(to)) {
        continue;
      }
      boolean left = rnd.nextBoolean();
      int maxRec = rnd.nextInt(6);
      int maxVia = rnd.nextInt(4);
      op("SHOVESEG " + BoardGen.pt(from) + " " + BoardGen.pt(to) + " " + BoardGen.b(left) + " " + layer + " " + BoardGen.nets(nets) + " " + hw + " " + cl + " " + maxRec + " " + maxVia);
      LineSegment seg = new LineSegment(new Polyline(from, to), 1);
      res(BoardGen.d(TraceShover.check(R, seg, left, layer, nets, hw, cl, maxRec, maxVia)));
      // a segment crossing a trace of another net on the layer (the maze shove check situation)
      List<PolylineTrace> others = new ArrayList<>();
      for (PolylineTrace o : traces()) {
        if (o.getLayer() == layer && !o.sharesNet(t) && o.polyline().lines.length >= 3) {
          others.add(o);
        }
      }
      if (!others.isEmpty()) {
        PolylineTrace o = others.get(rnd.nextInt(others.size()));
        int segNo = 1 + rnd.nextInt(o.polyline().lines.length - 2);
        FloatPoint m = o.polyline().cornerApprox(segNo - 1).middlePoint(o.polyline().cornerApprox(segNo));
        IntPoint mid = m.round();
        int[] d = offset(unit * (3 + rnd.nextInt(20)));
        IntPoint a1 = new IntPoint(mid.x - d[0], mid.y - d[1]);
        IntPoint a2 = new IntPoint(mid.x + d[0], mid.y + d[1]);
        if (!a1.equals(a2)) {
          maxRec = rnd.nextInt(6);
          maxVia = rnd.nextInt(4);
          op("SHOVESEG " + BoardGen.pt(a1) + " " + BoardGen.pt(a2) + " " + BoardGen.b(left) + " " + layer + " " + BoardGen.nets(nets) + " " + hw + " " + cl + " " + maxRec + " "
              + maxVia);
          res(BoardGen.d(TraceShover.check(R, new LineSegment(new Polyline(a1, a2), 1), left, layer, nets, hw, cl, maxRec, maxVia)));
          boolean onlyNot = rnd.nextBoolean();
          op("CHECKSEG " + BoardGen.pt(a1) + " " + BoardGen.pt(a2) + " " + layer + " " + BoardGen.nets(nets) + " " + hw + " " + cl + " " + BoardGen.b(onlyNot));
          res(BoardGen.d(R.checkTraceSegment(a1, a2, layer, nets, hw, cl, onlyNot)));
        }
      }
      IntPoint to2 = target(to, unit);
      Point[] corners = to2.equals(to) ? new Point[] {from, to} : new Point[] {from, to, to2};
      Polyline pl = new Polyline(corners);
      if (pl.lines.length < 3) {
        continue;
      }
      int maxSpring = rnd.nextInt(6);
      StringBuilder sb = new StringBuilder("FPOLYCHECK ").append(corners.length);
      for (Point p : corners) {
        sb.append(' ').append(BoardGen.pt(p));
      }
      sb.append(' ').append(hw).append(' ').append(layer).append(' ').append(BoardGen.nets(nets)).append(' ').append(cl).append(' ').append(maxRec).append(' ').append(maxVia)
          .append(' ').append(maxSpring);
      op(sb.toString());
      res(BoardGen.b(R.checkForcedTracePolyline(pl, hw, layer, nets, cl, maxRec, maxVia, maxSpring)));
      shoveFailure();
    }
    // segments running into vias of other nets (partial shove lengths)
    for (Via v : sample(vias(), scale)) {
      List<PolylineTrace> ts = traces();
      if (ts.isEmpty() || !(v.getCenter() instanceof IntPoint c)) {
        continue;
      }
      PolylineTrace t = ts.get(rnd.nextInt(ts.size()));
      if (t.sharesNet(v) || !v.isOnLayer(t.getLayer())) {
        continue;
      }
      int[] d = offset(unit * (5 + rnd.nextInt(20)));
      int side = rnd.nextInt(3) - 1;
      IntPoint a1 = new IntPoint(c.x - d[0] + side * d[1] / 8, c.y - d[1] - side * d[0] / 8);
      IntPoint a2 = new IntPoint(c.x + d[0] + side * d[1] / 8, c.y + d[1] - side * d[0] / 8);
      if (a1.equals(a2)) {
        continue;
      }
      boolean left = rnd.nextBoolean();
      int maxRec = rnd.nextInt(4);
      int maxVia = rnd.nextInt(3);
      op("SHOVESEG " + BoardGen.pt(a1) + " " + BoardGen.pt(a2) + " " + BoardGen.b(left) + " " + t.getLayer() + " " + BoardGen.nets(t.netNumbers) + " " + t.getHalfWidth() + " "
          + t.clearanceClassIndex() + " " + maxRec + " " + maxVia);
      res(BoardGen.d(TraceShover.check(R, new LineSegment(new Polyline(a1, a2), 1), left, t.getLayer(), t.netNumbers, t.getHalfWidth(), t.clearanceClassIndex(), maxRec, maxVia)));
    }
    // checks do not change the board, except the ids consumed by substitute traces
    dump();
  }

  static void phaseForcedTraces() {
    int unit = Math.max(R.getMinTraceHalfWidth(), 100);
    List<Pin> pins = new ArrayList<>(R.getPins());
    for (int k = 0; k < 3 * scale; k++) {
      Point from;
      int layer;
      int[] nets;
      int hw;
      int cl;
      List<PolylineTrace> ts = traces();
      if (ts.isEmpty()) {
        break;
      }
      PolylineTrace t = ts.get(rnd.nextInt(ts.size()));
      hw = t.getHalfWidth();
      cl = t.clearanceClassIndex();
      if (rnd.nextInt(3) == 0 && !pins.isEmpty()) {
        Pin p = pins.get(rnd.nextInt(pins.size()));
        if (p.netCount() != 1 || !p.isOnTheBoard()) {
          continue;
        }
        from = p.getCenter();
        layer = p.firstLayer() + rnd.nextInt(p.lastLayer() - p.firstLayer() + 1);
        nets = p.netNumbers;
      } else {
        from = rnd.nextBoolean() ? t.firstCorner() : t.lastCorner();
        layer = t.getLayer();
        nets = t.netNumbers;
      }
      if (!(from instanceof IntPoint)) {
        continue;
      }
      IntPoint to = target(from, unit);
      int maxRec = rnd.nextInt(3) == 0 ? rnd.nextInt(4) : 20;
      int maxVia = rnd.nextInt(3) == 0 ? rnd.nextInt(3) : 8;
      int maxSpring = rnd.nextInt(3) == 0 ? rnd.nextInt(3) : 8;
      int tidy = rnd.nextInt(4) == 0 ? rnd.nextInt(10000) : Integer.MAX_VALUE;
      int acc = randomAccuracy();
      boolean withCheck = rnd.nextInt(8) != 0;
      boolean snapped = rnd.nextInt(3) != 0;
      if (snapped) {
        op("SNAP");
        R.generateSnapshot();
        snapDepth++;
      }
      op("FTRACE " + BoardGen.pt(from) + " " + BoardGen.pt(to) + " " + hw + " " + layer + " " + BoardGen.nets(nets) + " " + cl + " " + maxRec + " " + maxVia + " " + maxSpring + " "
          + tidy + " " + acc + " " + BoardGen.b(withCheck));
      Point result = R.insertForcedTraceSegment(from, to, hw, layer, nets, cl, maxRec, maxVia, maxSpring, tidy, acc, withCheck, null);
      String rs;
      if (result == null) {
        rs = "N";
      } else if (result == from) {
        rs = "F";
      } else if (result == to) {
        rs = "T";
      } else {
        rs = "O " + BoardGen.pt(result);
      }
      res(rs);
      shoveFailure();
      maybeDump();
      if (snapped) {
        unsnapMaybe(result == null);
      }
    }
    dump();
  }

  static void phaseForcedVias() {
    if (R.rules.viaInfos.count() == 0) {
      return;
    }
    int unit = Math.max(R.getMinTraceHalfWidth(), 100);
    for (int k = 0; k < 2 * scale; k++) {
      List<PolylineTrace> ts = traces();
      if (ts.isEmpty()) {
        break;
      }
      PolylineTrace t = ts.get(rnd.nextInt(ts.size()));
      ViaInfo vi = R.rules.viaInfos.get(rnd.nextInt(R.rules.viaInfos.count()));
      Point base = rnd.nextBoolean() ? t.firstCorner() : t.lastCorner();
      if (!(base instanceof IntPoint)) {
        continue;
      }
      IntPoint loc = rnd.nextBoolean() ? (IntPoint) base : target(base, unit / 2 + 1);
      int[] nets = t.netNumbers;
      if (rnd.nextInt(4) == 0) {
        // a foreign net: forces shoving
        int n = 1 + rnd.nextInt(Math.max(R.rules.nets.maxNetNumber(), 1));
        nets = new int[] {n};
      }
      int traceCl = t.clearanceClassIndex();
      int[] pen = new int[layerCount()];
      for (int i = 0; i < pen.length; i++) {
        pen[i] = rnd.nextInt(3) == 0 ? 0 : t.getHalfWidth();
      }
      int maxRec = rnd.nextInt(3) == 0 ? rnd.nextInt(4) : 20;
      int maxVia = rnd.nextInt(3) == 0 ? rnd.nextInt(3) : 8;
      StringBuilder penS = new StringBuilder().append(pen.length);
      for (int p : pen) {
        penS.append(',').append(p);
      }
      String viaS = vi.getPadstack().id + " " + vi.getClearanceClassIndex() + " " + BoardGen.b(vi.attachSmdAllowed());
      int kind = rnd.nextInt(3);
      if (kind == 0) {
        op("FVIACHECK " + viaS + " " + BoardGen.pt(loc) + " " + BoardGen.nets(nets) + " " + maxRec + " " + maxVia + " " + penS + " " + traceCl);
        res(BoardGen.b(ForcedViaInserter.check(vi, loc, nets, maxRec, maxVia, R, pen, traceCl)));
        shoveFailure();
      } else if (kind == 1) {
        int layer = rnd.nextInt(layerCount());
        ConvexShape viaShape = vi.getPadstack().getShape(layer);
        double radius = viaShape == null ? 0 : 0.5 * viaShape.maxWidth();
        IntBox room = new IntBox(loc.x - 50 * unit, loc.y - 50 * unit, loc.x + 50 * unit, loc.y + 50 * unit);
        if (rnd.nextBoolean()) {
          room = new IntBox(loc.x - 5 * unit, loc.y - unit, loc.x + unit, loc.y + 3 * unit);
        }
        op("FVIALAYER " + BoardGen.d(radius) + " " + vi.getClearanceClassIndex() + " " + BoardGen.b(vi.attachSmdAllowed()) + " " + BoardGen.shape(room) + " " + BoardGen.pt(loc) + " "
            + layer + " " + BoardGen.nets(nets) + " " + maxRec + " " + maxVia + " " + t.getHalfWidth() + " " + traceCl);
        ForcedPadRouter.CheckDrillResult cr = ForcedViaInserter.checkLayer(radius, vi.getClearanceClassIndex(), vi.attachSmdAllowed(), room, loc, layer, nets, maxRec, maxVia, R,
            t.getHalfWidth(), traceCl);
        res(String.valueOf(cr.ordinal()));
        shoveFailure();
      } else {
        boolean snapped = rnd.nextInt(3) != 0;
        if (snapped) {
          op("SNAP");
          R.generateSnapshot();
          snapDepth++;
        }
        op("FVIA " + viaS + " " + BoardGen.pt(loc) + " " + BoardGen.nets(nets) + " " + traceCl + " " + penS + " " + maxRec + " " + maxVia);
        boolean ok = ForcedViaInserter.insert(vi, loc, nets, traceCl, pen, maxRec, maxVia, R);
        res(BoardGen.b(ok));
        shoveFailure();
        maybeDump();
        if (snapped) {
          unsnapMaybe(!ok);
        }
      }
    }
    dump();
  }

  static void phaseDrill() {
    int unit = Math.max(R.getMinTraceHalfWidth(), 100);
    for (Via v : sample(vias(), 2 * scale)) {
      if (!v.isOnTheBoard()) {
        continue;
      }
      IntPoint c = v.getCenter().toFloat().round();
      IntPoint t = target(c, unit / 2 + 1);
      int dx = t.x - c.x;
      int dy = t.y - c.y;
      int maxRec = rnd.nextInt(3) == 0 ? rnd.nextInt(4) : 9;
      int maxVia = rnd.nextInt(3) == 0 ? rnd.nextInt(3) : 9;
      if (rnd.nextBoolean()) {
        op("DRILLCHECK " + v.getId() + " " + dx + " " + dy + " " + maxRec + " " + maxVia);
        res(BoardGen.b(DrillItemMover.check(v, new IntVector(dx, dy), maxRec, maxVia, null, R, null)));
        shoveFailure();
      } else {
        boolean snapped = rnd.nextInt(3) != 0;
        if (snapped) {
          op("SNAP");
          R.generateSnapshot();
          snapDepth++;
        }
        op("STARTMARK");
        R.startMarkingChangedArea();
        op("DRILLMOVE " + v.getId() + " " + dx + " " + dy + " " + maxRec + " " + maxVia);
        boolean ok = DrillItemMover.insert(v, new IntVector(dx, dy), maxRec, maxVia, null, R);
        res(BoardGen.b(ok));
        shoveFailure();
        maybeDump();
        if (snapped) {
          unsnapMaybe(!ok);
        }
      }
    }
    dump();
  }

  static void phaseMisc() {
    int unit = Math.max(R.getMinTraceHalfWidth(), 100);
    for (PolylineTrace t : sample(traces(), scale)) {
      if (!t.isOnTheBoard()) {
        continue;
      }
      Point corner = t.polyline().corner(t.polyline().cornerCount() / 2);
      IntPoint from = target(corner, unit);
      op("CONNTRACE " + from.x + " " + from.y + " " + t.getId() + " " + t.getHalfWidth() + " " + t.clearanceClassIndex());
      res(BoardGen.b(R.connectToTrace(from, t, t.getHalfWidth(), t.clearanceClassIndex())));
      maybeDump();
    }
    op("REDUCENETS");
    res(BoardGen.b(R.reduceNetsOfRouteItems()));
    for (int k = 0; k < 2; k++) {
      int net = rnd.nextBoolean() ? -1 : 1 + rnd.nextInt(Math.max(R.rules.nets.maxNetNumber(), 1));
      Item.StopConnectionOption opt = Item.StopConnectionOption.values()[rnd.nextInt(3)];
      op("RMTAILS " + net + " " + opt.ordinal());
      res(BoardGen.b(R.removeTraceTails(net, opt)));
      dump();
    }
    // serialization round trip: search trees rebuilt, transient state reset
    op("DEEPCOPY");
    R = R.deepCopy();
    BoardGen.B = R;
    dump();
    op("PLANEOBST 1");
    R.changePlaneAsObstacle(true);
    dump();
    op("PLANEOBST 0");
    R.changePlaneAsObstacle(false);
    dump();
  }
}
