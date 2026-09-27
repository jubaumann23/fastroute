// Ground-truth generator for fr-engine::board (porting unit U5), using the real Freerouting
// classes of reference/bin/freerouting-2.4.1.jar.
//
// Regenerate (from the workspace root):
//   J=reference/jdk25/bin; D=crates/fr-engine/testdata/board; F=reference/freerouting/fixtures
//   $J/javac -cp reference/bin/freerouting-2.4.1.jar -d /tmp/boardgen $D/BoardGen.java
//   run() { $J/java -cp reference/bin/freerouting-2.4.1.jar:/tmp/boardgen BoardGen "$@"; }
//   run $F/Issue313-FastTest.dsn - 45 1 full > $D/issue313.txt
//   run $F/Issue313-FastTest.dsn $F/Issue313-FastTest.ses 45c 2 brief > $D/issue313_ses_comp.txt
//   run $F/Issue229-display-8-digit-hc595.dsn - 90 3 brief > $D/issue229_90.txt
//   run $F/Issue413-test.dsn - none 4 full > $D/issue413_none.txt
//   run $F/Issue575-drc_BBD_Mars-64_6_track_1_hole_clearance_violations.dsn - 45 5 brief > $D/issue575_mars.txt
//   run $F/Issue110-testPCBSpecctraFile.dsn - keep 16 brief > $D/issue110.txt
// Larger boards (not committed; run the Rust test with FR_BOARD_VECTORS=<file>):
//   run $F/Issue103-Board-Routed.dsn - 45 7 brief > /tmp/issue103.txt
//   run $F/Issue723-CombineStackOverflow.dsn - 45 8 brief > /tmp/issue723.txt
//
// Arguments: <dsn> <ses or -> <angle: 45|90|none|keep, suffix c: clearance compensation> <seed> <full|brief> [expand]
// (full: tile shapes in the dumps; expand: all dumps expanded instead of hashed, for debugging)
//
// The board A is loaded with DsnReader (and optionally a SES). A fresh BasicBoard B is built with
// the same rules/library/components and a fresh id generator, and a script of operations is
// executed on B: the non-trace items of A in id order, the traces of A cut into single segments
// (3-line polylines) in shuffled order, the vias (which split the traces), normalizeAllTraces,
// queries, autoroute trees with completeShape, rooms, and further mutations. Every operation is
// written as one line; results are written as lines starting with "= ". The Rust test replays
// the script and compares the result lines.
//
// Encodings: point "I x y" | "R x y z"; line "L <pt> <pt>"; tile shapes "B llx lly urx ury",
// "O lx ly rx uy ulx lrx llx urx", "S n <line>*n"; shapes additionally "C x y r", "P n <pt>*n";
// areas: a shape or "A <border> nholes <hole>*"; doubles as hex of the IEEE bits; strings as hex
// of the UTF-8 bytes ("-" for null, "~" for empty).

import app.freerouting.autoroute.expansion.CompleteFreeSpaceExpansionRoom;
import app.freerouting.autoroute.expansion.IncompleteFreeSpaceExpansionRoom;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.items.*;
import app.freerouting.board.model.structure.*;
import app.freerouting.board.searchtree.SearchTreeObject;
import app.freerouting.board.searchtree.ShapeSearchTree;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.core.library.Package;
import app.freerouting.core.library.Padstack;
import app.freerouting.datastructures.ShapeTree;
import app.freerouting.geometry.planar.*;
import app.freerouting.geometry.planar.Vector;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.io.specctra.SesReader;
import app.freerouting.rules.*;
import java.io.*;
import java.lang.reflect.Field;
import java.math.BigInteger;
import java.nio.charset.StandardCharsets;
import java.util.*;

public class BoardGen {
  static PrintStream out;
  static BasicBoard B;
  static boolean full;
  static boolean expandAll;
  static Random rnd;
  static Map<Integer, CompleteFreeSpaceExpansionRoom> rooms = new HashMap<>();

  public static void main(String[] args) throws Exception {
    out = new PrintStream(new BufferedOutputStream(new FileOutputStream(FileDescriptor.out), 1 << 16), false, "UTF-8");
    // Freerouting logs to the console: keep stdout clean for the vectors
    System.setOut(new PrintStream(OutputStream.nullOutputStream()));
    String dsn = args[0];
    String ses = args[1];
    String angle = args[2];
    rnd = new Random(Long.parseLong(args[3]));
    full = args[4].equals("full");
    expandAll = args.length > 5 && args[5].equals("expand");
    BoardReadResult r = DsnReader.readBoard(new FileInputStream(dsn), null, null, new File(dsn).getName());
    BasicBoard a = ((BoardReadResult.Success) r).board();
    if (!ses.equals("-")) {
      SesReader.read(new FileInputStream(ses), a);
    }
    boolean forceCompensation = angle.endsWith("c");
    if (forceCompensation) {
      angle = angle.substring(0, angle.length() - 1);
    }
    switch (angle) {
      case "45" -> a.rules.setTraceAngleRestriction(AngleRestriction.FORTYFIVE_DEGREE);
      case "90" -> a.rules.setTraceAngleRestriction(AngleRestriction.NINETY_DEGREE);
      case "none" -> a.rules.setTraceAngleRestriction(AngleRestriction.NONE);
      default -> {}
    }
    List<Item> aItems = new ArrayList<>(a.getItems());
    aItems.sort(Comparator.comparingInt(Item::getId));
    op("MODE " + (full ? "full" : "brief"));
    emitSetup(a);

    // ---- board B
    setField(a.communication, "idGenerator", new ItemIdGenerator());
    BoardOutline ao = a.getOutline();
    PolylineShape[] outlineShapes = new PolylineShape[ao.shapeCount()];
    for (int i = 0; i < outlineShapes.length; i++) {
      outlineShapes[i] = ao.getShape(i);
    }
    B = new BasicBoard(a.boundingBox, a.layerStructure, outlineShapes, ao.clearanceClassIndex(), a.rules, a.communication);
    setField(B, "library", a.library);
    setField(B, "components", a.components);
    StringBuilder sb = new StringBuilder("BOARD ").append(outlineShapes.length).append(' ').append(ao.clearanceClassIndex());
    for (PolylineShape s : outlineShapes) {
      sb.append(' ').append(shape(s));
    }
    op(sb.toString());
    if (a.searchTreeManager.isClearanceCompensationUsed() || forceCompensation) {
      op("COMPENSATION");
      B.searchTreeManager.setClearanceCompensationUsed(true);
    }
    if (ao.keepoutOutsideOutlineGenerated()) {
      op("KEEPOUT");
      B.getOutline().generateKeepoutOutside(true);
    }

    // ---- non-trace, non-via items in id order
    for (Item it : aItems) {
      if (it instanceof BoardOutline || it instanceof Trace || it instanceof Via) {
        continue;
      }
      insertCopy(it);
    }
    // ---- traces cut into segments, shuffled
    List<Object[]> segments = new ArrayList<>();
    for (Item it : aItems) {
      if (it instanceof PolylineTrace t) {
        Line[] lines = t.polyline().lines;
        for (int i = 0; i + 2 < lines.length; i++) {
          segments.add(new Object[] {t, new Line[] {lines[i], lines[i + 1], lines[i + 2]}});
        }
      }
    }
    Collections.shuffle(segments, rnd);
    for (Object[] seg : segments) {
      PolylineTrace t = (PolylineTrace) seg[0];
      traceOp("TRACENC", (Line[]) seg[1], t.getLayer(), t.getHalfWidth(), t.netNumbers, t.clearanceClassIndex(), t.getFixedState());
    }
    dump("after traces", false);
    // ---- vias (split the traces)
    for (Item it : aItems) {
      if (it instanceof Via v) {
        op("VIA " + v.getPadstack().id + " " + pt(v.getCenter()) + " " + nets(v.netNumbers) + " " + v.clearanceClassIndex() + " " + v.getFixedState().ordinal() + " " + b(v.attachAllowed));
        B.insertVia(v.getPadstack(), v.getCenter(), v.netNumbers, v.clearanceClassIndex(), v.getFixedState(), v.attachAllowed);
      }
    }
    dump("after vias", false);
    op("NORMALIZE_ALL");
    res(b(B.normalizeAllTraces()));
    dump("after normalize", true);
    queries();
    autorouteTrees();
    mutations();
    dump("final", true);
    out.flush();
  }

  // ------------------------------------------------------------------------------------------
  // setup

  static void emitSetup(BasicBoard a) throws Exception {
    Layer[] layers = a.layerStructure.layers;
    StringBuilder sb = new StringBuilder("LAYERS ").append(layers.length);
    for (Layer l : layers) {
      sb.append(' ').append(str(l.name)).append(' ').append(b(l.isSignal));
    }
    op(sb.toString());
    op("BBOX " + box(a.boundingBox));
    String host = a.communication.specctraParserInfo == null ? null : a.communication.specctraParserInfo.hostCad;
    op("COMM " + a.communication.unit.name() + " " + a.communication.resolution + " " + str(host));
    op("FLIP " + b(a.components.getFlipStyleRotateFirst()));
    op("RULES " + a.rules.getTraceAngleRestriction().ordinal() + " " + a.rules.getHoleClearance() + " " + d(a.rules.getPinEdgeToTurnDist()));
    ClearanceMatrix m = a.rules.clearanceMatrix;
    int n = m.getClassCount();
    sb = new StringBuilder("CLASSES ").append(n);
    for (int i = 0; i < n; i++) {
      sb.append(' ').append(str(m.getName(i)));
    }
    op(sb.toString());
    for (int j = 0; j < n; j++) {
      for (int layer = 0; layer < layers.length; layer++) {
        sb = new StringBuilder("CLROW ").append(j).append(' ').append(layer).append(' ').append(m.maxValue(j, layer));
        for (int i = 0; i < n; i++) {
          sb.append(' ').append(m.getValue(i, j, layer, false));
        }
        op(sb.toString());
      }
    }
    List<NetClass> classes = new ArrayList<>();
    for (int i = 0; i < a.rules.netClasses.count(); i++) {
      classes.add(a.rules.netClasses.get(i));
    }
    for (int i = 1; i <= a.rules.nets.maxNetNumber(); i++) {
      NetClass nc = a.rules.nets.get(i).getNetClass();
      if (nc != null && !classes.contains(nc)) {
        classes.add(nc);
      }
    }
    for (NetClass nc : classes) {
      op("NETCLASS " + str(nc.getName()) + " " + b(nc.isShoveFixed()) + " " + b(nc.getPullTight()) + " " + b(nc.getIgnoreCyclesWithAreas()) + " " + b(nc.isIgnoredByAutorouter));
    }
    for (int i = 1; i <= a.rules.nets.maxNetNumber(); i++) {
      Net net = a.rules.nets.get(i);
      op("NET " + net.netNumber + " " + str(net.name) + " " + net.subnetNumber + " " + classes.indexOf(net.getNetClass()) + " " + b(net.containsPlane()));
    }
    for (int i = 1; i <= a.library.padstacks.count(); i++) {
      Padstack p = a.library.padstacks.get(i);
      sb = new StringBuilder("PADSTACK ").append(p.id).append(' ').append(str(p.name)).append(' ').append(b(p.attachAllowed)).append(' ')
          .append(b(p.placedAbsolute)).append(' ').append(b(p.holeOnly)).append(' ').append(layers.length);
      for (int l = 0; l < layers.length; l++) {
        ConvexShape s = p.getShape(l);
        sb.append(' ').append(s == null ? "N" : shape(s));
      }
      op(sb.toString());
    }
    for (int i = 1; i <= a.library.packages.count(); i++) {
      Package p = a.library.packages.get(i);
      sb = new StringBuilder("PACKAGE ").append(p.id).append(' ').append(str(p.name)).append(' ').append(b(p.isFront)).append(' ').append(p.pinCount());
      for (int k = 0; k < p.pinCount(); k++) {
        Package.Pin pin = p.getPin(k);
        IntVector v = (IntVector) pin.relativeLocation;
        sb.append(' ').append(str(pin.name)).append(' ').append(pin.padstackId).append(' ').append(v.x).append(' ').append(v.y).append(' ').append(d(pin.rotationInDegree));
      }
      op(sb.toString());
    }
    for (int i = 1; i <= a.components.count(); i++) {
      Component c = a.components.get(i);
      Package front = (Package) getField(c, "libPackageFront");
      Package back = (Package) getField(c, "libPackageBack");
      Point loc = c.getLocation();
      op("COMP " + c.id + " " + str(c.name) + " " + (loc == null ? "N" : pt(loc)) + " " + d(c.getRotationInDegree()) + " " + b(c.placedOnFront()) + " " + front.id + " " + back.id + " " + b(c.positionFixed));
    }
  }

  // ------------------------------------------------------------------------------------------
  // item insertion

  static void insertCopy(Item it) throws Exception {
    if (it instanceof Pin p) {
      op("PIN " + p.getComponentId() + " " + p.pinIndex + " " + nets(p.netNumbers) + " " + p.clearanceClassIndex() + " " + p.getFixedState().ordinal());
      B.insertPin(p.getComponentId(), p.pinIndex, p.netNumbers, p.clearanceClassIndex(), p.getFixedState());
    } else if (it instanceof ConductionArea c) {
      op("COND " + c.getLayer() + " " + vec(c.getTranslation()) + " " + d(c.getRotationInDegree()) + " " + b(c.getSideChanged()) + " " + nets(c.netNumbers) + " "
          + c.clearanceClassIndex() + " " + c.getComponentId() + " " + str(c.name) + " " + b(c.getIsObstacle()) + " " + c.getFixedState().ordinal() + " " + area(c.getRelativeArea()));
      ConductionArea nc = new ConductionArea(c.getRelativeArea(), c.getLayer(), c.getTranslation(), c.getRotationInDegree(), c.getSideChanged(), c.netNumbers,
          c.clearanceClassIndex(), 0, c.getComponentId(), c.name, c.getIsObstacle(), c.getFixedState(), B);
      B.insertItem(nc);
    } else if (it instanceof ObstacleArea o) {
      String kind = o instanceof ViaObstacleArea ? "V" : o instanceof ComponentObstacleArea ? "C" : "K";
      op("OBS " + kind + " " + o.getLayer() + " " + vec(o.getTranslation()) + " " + d(o.getRotationInDegree()) + " " + b(o.getSideChanged()) + " "
          + o.clearanceClassIndex() + " " + o.getComponentId() + " " + str(o.name) + " " + o.getFixedState().ordinal() + " " + area(o.getRelativeArea()));
      switch (kind) {
        case "V" -> B.insertViaObstacle(o.getRelativeArea(), o.getLayer(), o.getTranslation(), o.getRotationInDegree(), o.getSideChanged(), o.clearanceClassIndex(), o.getComponentId(), o.name, o.getFixedState());
        case "C" -> B.insertComponentObstacle(o.getRelativeArea(), o.getLayer(), o.getTranslation(), o.getRotationInDegree(), o.getSideChanged(), o.clearanceClassIndex(), o.getComponentId(), o.name, o.getFixedState());
        default -> B.insertObstacle(o.getRelativeArea(), o.getLayer(), o.getTranslation(), o.getRotationInDegree(), o.getSideChanged(), o.clearanceClassIndex(), o.getComponentId(), o.name, o.getFixedState());
      }
    } else if (it instanceof ComponentOutline co) {
      Area rel = (Area) getField(co, "relativeArea");
      Vector tr = (Vector) getField(co, "translation");
      double rot = (Double) getField(co, "rotationInDegree");
      op("COUTLINE " + b(co.isFront()) + " " + vec(tr) + " " + d(rot) + " " + co.getComponentId() + " " + b(co.isCourtyard()) + " " + b(co.isFabrication()) + " " + b(co.isClosed()) + " "
          + co.getFixedState().ordinal() + " " + area(rel));
      B.insertComponentOutline(rel, co.isFront(), tr, rot, co.getComponentId(), co.isCourtyard(), co.isFabrication(), co.isClosed(), co.getFixedState());
    } else {
      throw new IllegalStateException("unexpected item " + it);
    }
  }

  static void traceOp(String opName, Line[] lines, int layer, int hw, int[] netNos, int cl, FixedState fixed) {
    StringBuilder sb = new StringBuilder(opName).append(' ').append(layer).append(' ').append(hw).append(' ').append(nets(netNos)).append(' ').append(cl).append(' ')
        .append(fixed.ordinal()).append(' ').append(lines.length);
    for (Line l : lines) {
      sb.append(' ').append(ln(l));
    }
    op(sb.toString());
    Polyline p = new Polyline(lines);
    if (opName.equals("TRACENC")) {
      PolylineTrace t = B.insertTraceWithoutCleaning(p, layer, hw, netNos, cl, fixed);
      res(t == null ? "-" : String.valueOf(t.getId()));
    } else {
      B.insertTrace(p, layer, hw, netNos, cl, fixed);
    }
  }

  // ------------------------------------------------------------------------------------------
  // dumps

  static void dump(String label, boolean expand) {
    op("DUMP " + (expand || expandAll ? "expand" : "hash"));
    Collection<Item> items = B.getItems();
    String head = "ITEMS " + items.size() + " rev " + B.getRevision() + " maxhw " + B.getMaxTraceHalfWidth() + " minhw " + B.getMinTraceHalfWidth();
    List<String> lines = new ArrayList<>();
    for (Item it : items) {
      lines.add(itemLine(it));
    }
    emitBlock(head, lines, expand);
    for (ShapeSearchTree tree : trees()) {
      dumpTree(tree, expand);
    }
  }

  /** Emits the header and the lines, or the header with an FNV-1a 64 hash of the lines. */
  static void emitBlock(String head, List<String> lines, boolean expand) {
    if (expand || expandAll) {
      res(head);
      for (String l : lines) {
        res(l);
      }
    } else {
      long h = 0xcbf29ce484222325L;
      for (String l : lines) {
        for (byte x : (l + "\n").getBytes(StandardCharsets.UTF_8)) {
          h ^= (x & 0xff);
          h *= 0x100000001b3L;
        }
      }
      res(head + " hash " + Long.toHexString(h));
    }
  }

  static String itemLine(Item it) {
    StringBuilder sb = new StringBuilder("I ").append(it.getId()).append(' ').append(it.getClass().getSimpleName()).append(' ').append(nets(it.netNumbers)).append(' ')
        .append(it.clearanceClassIndex()).append(' ').append(it.getFixedState().ordinal()).append(' ').append(it.getComponentId()).append(' ').append(b(it.isOnTheBoard()))
        .append(' ').append(it.firstLayer()).append(' ').append(it.lastLayer()).append(' ').append(box(it.boundingBox())).append(" n ").append(it.tileShapeCount());
    if (it instanceof PolylineTrace t) {
      sb.append(" hw ").append(t.getHalfWidth()).append(' ').append(t.polyline().lines.length);
      for (Line l : t.polyline().lines) {
        sb.append(' ').append(ln(l));
      }
    } else if (it instanceof DrillItem di) {
      sb.append(" c ").append(pt(di.getCenter())).append(" ps ").append(di.getPadstack().id);
    }
    if (full) {
      for (int i = 0; i < it.tileShapeCount(); i++) {
        TileShape s = it.getTileShape(i);
        sb.append(" | ").append(it.shapeLayer(i)).append(' ').append(s == null ? "N" : shape(s));
      }
    }
    return sb.toString();
  }

  @SuppressWarnings("unchecked")
  static List<ShapeSearchTree> trees() {
    try {
      Field f = B.searchTreeManager.getClass().getDeclaredField("compensatedSearchTrees");
      f.setAccessible(true);
      return new ArrayList<>((Collection<ShapeSearchTree>) f.get(B.searchTreeManager));
    } catch (Exception e) {
      throw new RuntimeException(e);
    }
  }

  static void dumpTree(ShapeSearchTree tree, boolean expand) {
    op("TREEDUMP " + tree.compensatedClearanceClassNo + " " + (expand || expandAll ? "expand" : "hash"));
    ShapeTree.Leaf[] leaves = tree.toArray();
    Field bsField;
    try {
      bsField = Class.forName("app.freerouting.datastructures.ShapeTree$TreeNode").getField("boundingShape");
    } catch (Exception e) {
      throw new RuntimeException(e);
    }
    List<String> lines = new ArrayList<>();
    for (ShapeTree.Leaf leaf : leaves) {
      StringBuilder sb = new StringBuilder("L ").append(obj(leaf.object)).append(' ').append(leaf.shapeIndexInObject).append(' ').append(leaves.length == 1 ? 0 : leaf.distanceToRoot());
      try {
        sb.append(' ').append(shape((RegularTileShape) bsField.get(leaf)));
      } catch (Exception e) {
        throw new RuntimeException(e);
      }
      if (full) {
        TileShape s = ((SearchTreeObject) leaf.object).getTreeShape(tree, leaf.shapeIndexInObject);
        sb.append(' ').append(s == null ? "N" : shape(s));
      }
      lines.add(sb.toString());
    }
    emitBlock("TREE " + tree.key + " " + leaves.length, lines, expand);
  }

  static String obj(Object o) {
    if (o instanceof Item it) {
      return String.valueOf(it.getId());
    }
    return "R" + ((CompleteFreeSpaceExpansionRoom) o).getId();
  }

  // ------------------------------------------------------------------------------------------
  // queries

  static List<Item> sample(int max) {
    List<Item> items = new ArrayList<>(B.getItems());
    if (items.size() <= max) {
      return items;
    }
    List<Item> result = new ArrayList<>();
    for (int i = 0; i < max; i++) {
      result.add(items.get((int) ((long) i * items.size() / max)));
    }
    return result;
  }

  static String ids(Collection<? extends Item> items) {
    StringBuilder sb = new StringBuilder("[");
    boolean first = true;
    for (Item it : items) {
      if (!first) {
        sb.append(',');
      }
      first = false;
      sb.append(it.getId());
    }
    return sb.append(']').toString();
  }

  static void queries() {
    List<Item> items = sample(full ? 100000 : 400);
    for (Item it : items) {
      op("QCONN " + it.getId());
      int net = it.netCount() > 0 ? it.getNetNumber(0) : -1;
      StringBuilder sb = new StringBuilder();
      sb.append(ids(it.getNormalContacts())).append(' ').append(ids(it.getAllContacts())).append(' ').append(ids(it.getConnectedSet(net))).append(' ')
          .append(ids(it.getConnectedSet(-1, true))).append(' ').append(b(it.isTail())).append(' ').append(ids(it.getConnectionItems()))
          .append(' ').append(ids(it.getConnectionItems(Item.StopConnectionOption.VIA))).append(' ').append(ids(it.getConnectionItems(Item.StopConnectionOption.FANOUT_VIA)))
          .append(' ').append(ids(it.getUnconnectedSet(net))).append(' ').append(b(it.isConnected()));
      if (it instanceof PolylineTrace t) {
        sb.append(' ').append(ids(t.getStartContacts())).append(' ').append(ids(t.getEndContacts())).append(' ').append(b(t.isOverlap())).append(' ').append(b(t.isCycle()))
            .append(' ').append(ids(t.touchingPinsAtEndCorners()));
      }
      if (it instanceof Via v) {
        sb.append(' ').append(b(v.isFanoutVia(null)));
      }
      res(sb.toString());
    }
    for (Item it : items) {
      if (it instanceof Pin p) {
        op("QEXIT " + p.getId());
        StringBuilder sb = new StringBuilder();
        for (int l = p.firstLayer(); l <= p.lastLayer(); l++) {
          sb.append('[');
          for (Pin.TraceExitRestriction r : p.getTraceExitRestrictions(l)) {
            sb.append(dir(r.direction)).append(' ').append(d(r.minLength)).append(';');
          }
          sb.append("] ").append(d(p.getMinWidth(l))).append(' ').append(d(p.getMaxWidth(l))).append(' ').append(p.getTraceNeckdownHalfwidth(l)).append(' ');
        }
        sb.append(b(p.hasTraceExitRestrictions())).append(' ').append(ids(p.getSwappablePins()));
        res(sb.toString());
      } else if (it instanceof PolylineTrace t) {
        op("QCONPIN " + t.getId());
        res(b(t.checkConnectionToPin(true)) + " " + b(t.checkConnectionToPin(false)));
      }
    }
    ShapeSearchTree tree = B.searchTreeManager.getDefaultTree();
    int layerCount = B.getLayerCount();
    int classCount = B.rules.clearanceMatrix.getClassCount();
    for (Item it : sample(full ? 3000 : 300)) {
      for (int i = 0; i < it.tileShapeCount(); i++) {
        TileShape s = it.getTileShape(i);
        if (s == null || rnd.nextInt(3) != 0) {
          continue;
        }
        TileShape q = rnd.nextBoolean() ? s : (TileShape) s.enlarge(rnd.nextInt(2000));
        int layer = rnd.nextInt(4) == 0 ? -1 : it.shapeLayer(i);
        int cl = rnd.nextInt(classCount);
        int[] netNos = it.netCount() > 0 && rnd.nextBoolean() ? it.netNumbers : new int[0];
        op("QOV " + layer + " " + nets(netNos) + " " + shape(q));
        Collection<ShapeTree.TreeEntry> entries = new LinkedList<>();
        tree.overlappingTreeEntries(q, layer, netNos, entries);
        res(entries(entries));
        op("QCL " + layer + " " + cl + " " + nets(netNos) + " " + shape(q));
        entries = new LinkedList<>();
        tree.overlappingTreeEntriesWithClearance(q, layer, netNos, cl, entries);
        res(entries(entries));
        if (layer >= 0) {
          op("QCT " + layer + " " + cl + " " + nets(it.netNumbers) + " " + shape(q));
          res(b(B.checkTraceShape(q, layer, it.netNumbers, cl, null)));
          op("QCS " + layer + " " + cl + " " + nets(it.netNumbers) + " " + shape(q));
          res(b(B.checkShape(q, layer, it.netNumbers, cl)));
          op("QIWC " + layer + " " + cl + " " + nets(netNos) + " " + shape(q));
          res(ids(B.overlappingItemsWithClearance(q, layer, netNos, cl)));
        }
      }
      if (it instanceof DrillItem di) {
        Point c = di.getCenter();
        int layer = di.firstLayer() + rnd.nextInt(di.lastLayer() - di.firstLayer() + 1);
        op("QPK " + pt(c) + " " + layer);
        res(ids(B.pickItems(c, layer, null)));
        op("QTAIL " + pt(c) + " " + layer + " " + nets(di.netNumbers));
        Trace tail = B.getTraceTail(c, layer, di.netNumbers);
        res(tail == null ? "-" : String.valueOf(tail.getId()));
      }
      if (it instanceof PolylineTrace t && rnd.nextInt(2) == 0) {
        Point c = t.firstCorner();
        op("QTAIL " + pt(c) + " " + t.getLayer() + " " + nets(t.netNumbers));
        Trace tail = B.getTraceTail(c, t.getLayer(), t.netNumbers);
        res(tail == null ? "-" : String.valueOf(tail.getId()));
        Line[] lines = t.polyline().lines;
        int cl = rnd.nextInt(classCount);
        StringBuilder sb = new StringBuilder("QCP ").append(t.getLayer()).append(' ').append(t.getHalfWidth()).append(' ').append(cl).append(' ').append(nets(t.netNumbers)).append(' ').append(lines.length);
        for (Line l : lines) {
          sb.append(' ').append(ln(l));
        }
        op(sb.toString());
        res(b(B.checkPolylineTrace(t.polyline(), t.getLayer(), t.getHalfWidth(), t.netNumbers, cl)));
      }
    }
    for (int net = 1; net <= B.rules.nets.maxNetNumber(); net++) {
      if (!full && net % 7 != 0) {
        continue;
      }
      op("QSETS " + net);
      StringBuilder sb = new StringBuilder();
      for (Collection<Item> set : B.getConnectedSets(net)) {
        sb.append(ids(set));
      }
      res(sb.length() == 0 ? "-" : sb.toString());
    }
    op("QMISC");
    res(B.getConductionAreas().size() + " " + B.getPins().size() + " " + B.getSmdPins().size() + " " + B.getVias().size() + " " + B.getTraces().size() + " "
        + d(B.cumulativeTraceLength()) + " " + B.getNon45DegreeTraceCount());
  }

  static String entries(Collection<ShapeTree.TreeEntry> entries) {
    StringBuilder sb = new StringBuilder("[");
    boolean first = true;
    for (ShapeTree.TreeEntry e : entries) {
      if (!first) {
        sb.append(',');
      }
      first = false;
      sb.append(obj(e.object)).append(':').append(e.shapeIndexInObject);
    }
    return sb.append(']').toString();
  }

  // ------------------------------------------------------------------------------------------
  // autoroute trees, completeShape, rooms

  static void autorouteTrees() {
    AngleRestriction ar = B.rules.getTraceAngleRestriction();
    int classCount = B.rules.clearanceMatrix.getClassCount();
    int roomKey = 1;
    for (int cl = 0; cl < Math.min(classCount, 3); cl++) {
      op("AUTOTREE " + cl);
      ShapeSearchTree tree = B.searchTreeManager.getAutorouteTree(cl);
      dumpTree(tree, false);
      List<Item> items = sample(full ? 60 : 30);
      for (Item it : items) {
        if (it.tileShapeCount() == 0) {
          continue;
        }
        int idx = rnd.nextInt(it.tileShapeCount());
        TileShape ts = it.getTileShape(idx);
        if (ts == null) {
          continue;
        }
        int layer = it.shapeLayer(idx);
        int net = it.netCount() > 0 ? it.getNetNumber(0) : 0;
        FloatPoint c = ts.centreOfGravity();
        IntPoint p = c.round();
        TileShape contained;
        TileShape roomShape = null;
        if (ar == AngleRestriction.NINETY_DEGREE) {
          contained = new IntBox(p, p);
          if (rnd.nextBoolean()) {
            roomShape = new IntBox(p.x - 500000, p.y - 400000, p.x + 300000, p.y + 600000);
          }
        } else {
          contained = rnd.nextBoolean() ? TileShape.getInstance(p) : ts;
          if (rnd.nextBoolean()) {
            roomShape = new IntOctagon(p.x - 500000, p.y - 400000, p.x + 300000, p.y + 600000, p.x - p.y - 700000, p.x - p.y + 800000, p.x + p.y - 900000, p.x + p.y + 600000);
            roomShape = ((IntOctagon) roomShape).normalize();
          }
        }
        String ignore = "-";
        SearchTreeObject ignoreObject = null;
        if (rnd.nextInt(4) == 0) {
          ignoreObject = it;
          ignore = "I" + it.getId();
        }
        TileShape ignoreShape = null;
        if (!rooms.isEmpty() && rnd.nextInt(3) == 0) {
          ignoreShape = rooms.values().iterator().next().getShape();
        }
        op("QCOMPLETE " + cl + " " + layer + " " + net + " " + ignore + " " + (ignoreShape == null ? "N" : shape(ignoreShape)) + " " + (roomShape == null ? "N" : shape(roomShape)) + " " + shape(contained));
        Collection<IncompleteFreeSpaceExpansionRoom> result = tree.completeShape(new IncompleteFreeSpaceExpansionRoom(roomShape, layer, contained), net, ignoreObject, ignoreShape);
        StringBuilder sb = new StringBuilder().append(result.size());
        for (IncompleteFreeSpaceExpansionRoom room : result) {
          sb.append(" | ").append(shape(room.getShape())).append(" ; ").append(shape(room.getContainedShape()));
        }
        res(sb.toString());
        // insert some completed rooms into the tree
        for (IncompleteFreeSpaceExpansionRoom room : result) {
          if (rnd.nextInt(3) == 0 && room.getShape().dimension() == 2) {
            int id = 1000000 + roomKey * 7;
            op("ROOMADD " + cl + " " + roomKey + " " + id + " " + layer + " " + shape(room.getShape()));
            CompleteFreeSpaceExpansionRoom cr = new CompleteFreeSpaceExpansionRoom(room.getShape(), layer, id);
            tree.insert(cr);
            rooms.put(roomKey, cr);
            roomKey++;
          }
        }
        if (!rooms.isEmpty() && rnd.nextInt(5) == 0) {
          int k = rooms.keySet().iterator().next();
          op("ROOMDEL " + cl + " " + k);
          rooms.remove(k).removeFromTree(tree);
        }
      }
      dumpTree(tree, false);
      for (Map.Entry<Integer, CompleteFreeSpaceExpansionRoom> e : rooms.entrySet()) {
        op("ROOMDEL " + cl + " " + e.getKey());
        e.getValue().removeFromTree(tree);
      }
      rooms.clear();
      dumpTree(tree, false);
    }
  }

  // ------------------------------------------------------------------------------------------
  // mutations

  static void mutations() {
    // straight traces between pins of the same net (normalized when inserted)
    Map<Integer, List<Pin>> pinsByNet = new TreeMap<>();
    for (Pin p : B.getPins()) {
      if (p.netCount() == 1) {
        pinsByNet.computeIfAbsent(p.getNetNumber(0), k -> new ArrayList<>()).add(p);
      }
    }
    int count = 0;
    for (List<Pin> pins : pinsByNet.values()) {
      if (pins.size() < 2 || count >= (full ? 40 : 20)) {
        continue;
      }
      Pin p1 = pins.get(0);
      Pin p2 = pins.get(1 + rnd.nextInt(pins.size() - 1));
      int layer = Math.max(p1.firstLayer(), p2.firstLayer());
      if (layer > Math.min(p1.lastLayer(), p2.lastLayer()) || p1.getCenter().equals(p2.getCenter())) {
        continue;
      }
      IntPoint c1 = (IntPoint) p1.getCenter();
      IntPoint c2 = (IntPoint) p2.getCenter();
      IntPoint mid = new IntPoint(c1.x, c2.y);
      Point[] corners = mid.equals(c1) || mid.equals(c2) ? new Point[] {c1, c2} : new Point[] {c1, mid, c2};
      Polyline pl = new Polyline(corners);
      if (pl.lines.length < 3) {
        continue;
      }
      traceOp("TRACE", pl.lines, layer, 50 + rnd.nextInt(200), p1.netNumbers, 1, FixedState.UNFIXED);
      count++;
    }
    // pin exit restrictions (pull tight helpers)
    for (Trace t : B.getTraces()) {
      PolylineTrace pt = (PolylineTrace) t;
      for (int end = 0; end < 2; end++) {
        if (!pt.isOnTheBoard()) {
          break;
        }
        boolean atStart = end == 0;
        if (!pt.checkConnectionToPin(atStart)) {
          op("CORRECTPIN " + pt.getId() + " " + b(atStart) + " " + B.rules.getTraceAngleRestriction().ordinal());
          res(b(pt.correctConnectionToPin(atStart, B.rules.getTraceAngleRestriction())));
        } else if (rnd.nextInt(3) == 0) {
          op("SWAPPIN " + pt.getId() + " " + b(atStart));
          res(b(pt.swapConnectionToPin(atStart)));
        }
      }
    }
    dump("after inserted traces", false);
    // remove some traces and vias
    List<Item> items = new ArrayList<>(B.getItems());
    for (int i = 0; i < items.size(); i += 1 + rnd.nextInt(9)) {
      Item it = items.get(i);
      if (it instanceof Trace || it instanceof Via) {
        op("REMOVE " + it.getId());
        B.removeItem(it);
      }
    }
    // split traces at pin centers
    for (Pin p : B.getPins()) {
      if (rnd.nextInt(10) == 0 && p.netCount() > 0) {
        op("SPLIT " + pt(p.getCenter()) + " " + p.firstLayer() + " " + p.getNetNumber(0));
        res(b(B.splitTraces(p.getCenter(), p.firstLayer(), p.getNetNumber(0))));
      }
    }
    // split traces at their corners (creates pieces that combine again)
    for (Trace t : B.getTraces()) {
      if (rnd.nextInt(6) == 0 && t.isOnTheBoard() && ((PolylineTrace) t).cornerCount() > 2) {
        Point c = ((PolylineTrace) t).polyline().corner(1);
        op("SPLITAT " + t.getId() + " " + pt(c));
        Trace[] pieces = t.split(c);
        res(pieces == null ? "-" : (pieces[0] == null ? "-" : String.valueOf(pieces[0].getId())) + " " + (pieces[1] == null ? "-" : String.valueOf(pieces[1].getId())));
      }
    }
    dump("after split", false);
    op("COMBINE -1");
    res(b(B.combineTraces(-1)));
    for (int net = 1; net <= B.rules.nets.maxNetNumber(); net += 1 + rnd.nextInt(4)) {
      op("NORMALIZE " + net);
      res(b(B.normalizeTraces(net)));
    }
  }

  // ------------------------------------------------------------------------------------------
  // encoding helpers

  static void op(String s) {
    out.println(s);
  }

  static void res(String s) {
    out.print("= ");
    out.println(s);
  }

  static String b(boolean v) {
    return v ? "1" : "0";
  }

  static String d(double v) {
    return Long.toHexString(Double.doubleToRawLongBits(v));
  }

  static String str(String s) {
    if (s == null) {
      return "-";
    }
    if (s.isEmpty()) {
      return "~";
    }
    StringBuilder sb = new StringBuilder();
    for (byte x : s.getBytes(StandardCharsets.UTF_8)) {
      sb.append(String.format("%02x", x & 0xff));
    }
    return sb.toString();
  }

  static String nets(int[] n) {
    StringBuilder sb = new StringBuilder().append(n.length);
    for (int x : n) {
      sb.append(',').append(x);
    }
    return sb.toString();
  }

  static String pt(Point p) {
    if (p instanceof IntPoint ip) {
      return "I " + ip.x + " " + ip.y;
    }
    try {
      RationalPoint r = (RationalPoint) p;
      return "R " + getField(r, "x") + " " + getField(r, "y") + " " + getField(r, "z");
    } catch (Exception e) {
      throw new RuntimeException(e);
    }
  }

  static String dir(Direction dd) {
    if (dd instanceof IntDirection id) {
      return "D " + id.x + " " + id.y;
    }
    return "BD " + dd;
  }

  static String vec(Vector v) {
    IntVector iv = (IntVector) v;
    return iv.x + " " + iv.y;
  }

  static String ln(Line l) {
    return "L " + pt(l.a) + " " + pt(l.b);
  }

  static String box(IntBox bx) {
    return "B " + bx.ll.x + " " + bx.ll.y + " " + bx.ur.x + " " + bx.ur.y;
  }

  static String shape(Object s) {
    if (s instanceof IntBox bx) {
      return box(bx);
    }
    if (s instanceof IntOctagon o) {
      return "O " + o.leftX + " " + o.bottomY + " " + o.rightX + " " + o.topY + " " + o.upperLeftDiagonalX + " " + o.lowerRightDiagonalX + " " + o.lowerLeftDiagonalX + " "
          + o.upperRightDiagonalX;
    }
    if (s instanceof Simplex sx) {
      StringBuilder sb = new StringBuilder("S ").append(sx.borderLineCount());
      for (int i = 0; i < sx.borderLineCount(); i++) {
        sb.append(' ').append(ln(sx.borderLine(i)));
      }
      return sb.toString();
    }
    if (s instanceof Circle c) {
      return "C " + c.center.x + " " + c.center.y + " " + c.radius;
    }
    if (s instanceof PolygonShape p) {
      StringBuilder sb = new StringBuilder("P ").append(p.corners.length);
      for (Point c : p.corners) {
        sb.append(' ').append(pt(c));
      }
      return sb.toString();
    }
    throw new IllegalArgumentException("shape " + s.getClass());
  }

  static String area(Area a) {
    if (a instanceof PolylineArea pa) {
      Shape[] holes = pa.getHoles();
      StringBuilder sb = new StringBuilder("A ").append(shape(pa.getBorder())).append(' ').append(holes.length);
      for (Shape h : holes) {
        sb.append(' ').append(shape(h));
      }
      return sb.toString();
    }
    return shape(a);
  }

  static Object getField(Object o, String name) throws Exception {
    Class<?> c = o.getClass();
    while (c != null) {
      try {
        Field f = c.getDeclaredField(name);
        f.setAccessible(true);
        return f.get(o);
      } catch (NoSuchFieldException e) {
        c = c.getSuperclass();
      }
    }
    throw new NoSuchFieldException(name);
  }

  static void setField(Object o, String name, Object value) throws Exception {
    Class<?> c = o.getClass();
    while (c != null) {
      try {
        Field f = c.getDeclaredField(name);
        f.setAccessible(true);
        f.set(o, value);
        return;
      } catch (NoSuchFieldException e) {
        c = c.getSuperclass();
      }
    }
    throw new NoSuchFieldException(name);
  }
}
