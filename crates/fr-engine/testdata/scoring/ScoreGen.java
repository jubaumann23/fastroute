// Ground-truth generator for fr-engine::drc and fr-engine::scoring (porting unit U6), using the real
// Freerouting classes of reference/bin/freerouting-parity.jar (built from the reference source).
//
// Reuses testdata/board/BoardGen.java (setup and item encodings, compiled together with it). The
// loaded board A is copied into a fresh BasicBoard B with the replay operations of BoardGen: the
// non-trace items in id order, the vias, then the traces unchanged (insertTraceWithoutCleaning) in
// id order. Then DRC and statistics are dumped for several board states ("SCORE" operations):
//   1. the loaded (e.g. routed) board, with some vias marked as escape vias,
//   2. after removing a random part of the traces and vias and inserting some straight traces
//      between pins of different nets (clearance violations),
//   3. without any traces and vias (unrouted).
// The Rust test (tests/board_replay.rs with tests/common/scoring_ops.rs) replays the operations and compares every result line.
//
// Regenerate (from the workspace root):
//   J=reference/jdk25/bin; D=crates/fr-engine/testdata; F=reference/freerouting/fixtures; P=reference/parity-baseline
//   $J/javac -cp reference/bin/freerouting-parity.jar -d /tmp/scoregen $D/board/BoardGen.java $D/scoring/ScoreGen.java
//   run() { $J/java -cp reference/bin/freerouting-parity.jar:/tmp/scoregen ScoreGen "$@"; }
//   (see crates/fr-engine/testdata/scoring/generate.sh for the list of committed vectors)
//
// Arguments: <dsn> <ses or -> <seed> [c (clearance compensation)] [tol=<clearance tolerance um>]
//   [expand (hashed blocks written in full, replay with FR_SCORING_EXPAND=1)]

import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.items.*;
import app.freerouting.board.model.structure.*;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.core.scoring.BoardStatistics;
import app.freerouting.drc.ClearanceViolation;
import app.freerouting.drc.DesignRulesChecker;
import app.freerouting.drc.NetIncompletes;
import app.freerouting.geometry.planar.*;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.io.specctra.SesReader;
import app.freerouting.rules.*;
import app.freerouting.settings.OptimizerScoringVersion;
import app.freerouting.settings.RouterScoringVersion;
import app.freerouting.settings.RouterSettings;
import app.freerouting.settings.sources.DefaultSettings;
import java.io.*;
import java.util.*;

public class ScoreGen {

  public static void main(String[] args) throws Exception {
    BoardGen.out = new PrintStream(new BufferedOutputStream(new FileOutputStream(FileDescriptor.out), 1 << 16), false, "UTF-8");
    System.setOut(new PrintStream(OutputStream.nullOutputStream()));
    String dsn = args[0];
    String ses = args[1];
    BoardGen.rnd = new Random(Long.parseLong(args[2]));
    BoardGen.full = false;
    boolean compensation = false;
    Double tolerance = null;
    for (int i = 3; i < args.length; i++) {
      if (args[i].equals("c")) {
        compensation = true;
      } else if (args[i].equals("expand")) {
        BoardGen.expandAll = true;
      } else if (args[i].startsWith("tol=")) {
        tolerance = Double.parseDouble(args[i].substring(4));
      }
    }
    Random rnd = BoardGen.rnd;
    BoardReadResult r = DsnReader.readBoard(new FileInputStream(dsn), null, null, new File(dsn).getName());
    BasicBoard a = ((BoardReadResult.Success) r).board();
    if (!ses.equals("-")) {
      SesReader.read(new FileInputStream(ses), a);
    }
    List<Item> aItems = new ArrayList<>(a.getItems());
    aItems.sort(Comparator.comparingInt(Item::getId));
    BoardGen.op("MODE brief");
    BoardGen.op("VARIANT source");
    BoardGen.emitSetup(a);

    // ---- board B (as in BoardGen)
    BoardGen.setField(a.communication, "idGenerator", new ItemIdGenerator());
    BoardOutline ao = a.getOutline();
    PolylineShape[] outlineShapes = new PolylineShape[ao.shapeCount()];
    for (int i = 0; i < outlineShapes.length; i++) {
      outlineShapes[i] = ao.getShape(i);
    }
    BasicBoard b = new BasicBoard(a.boundingBox, a.layerStructure, outlineShapes, ao.clearanceClassIndex(), a.rules, a.communication);
    BoardGen.B = b;
    BoardGen.setField(b, "library", a.library);
    BoardGen.setField(b, "components", a.components);
    StringBuilder sb = new StringBuilder("BOARD ").append(outlineShapes.length).append(' ').append(ao.clearanceClassIndex());
    for (PolylineShape s : outlineShapes) {
      sb.append(' ').append(BoardGen.shape(s));
    }
    BoardGen.op(sb.toString());
    if (a.searchTreeManager.isClearanceCompensationUsed() || compensation) {
      BoardGen.op("COMPENSATION");
      b.searchTreeManager.setClearanceCompensationUsed(true);
    }
    if (ao.keepoutOutsideOutlineGenerated()) {
      BoardGen.op("KEEPOUT");
      b.getOutline().generateKeepoutOutside(true);
    }
    emitScoringSetup(a, tolerance);

    // ---- items
    for (Item it : aItems) {
      if (it instanceof BoardOutline || it instanceof Trace || it instanceof Via) {
        continue;
      }
      BoardGen.insertCopy(it);
    }
    for (Item it : aItems) {
      if (it instanceof Via v) {
        BoardGen.op("VIA " + v.getPadstack().id + " " + BoardGen.pt(v.getCenter()) + " " + BoardGen.nets(v.netNumbers) + " " + v.clearanceClassIndex() + " "
            + v.getFixedState().ordinal() + " " + BoardGen.b(v.attachAllowed));
        b.insertVia(v.getPadstack(), v.getCenter(), v.netNumbers, v.clearanceClassIndex(), v.getFixedState(), v.attachAllowed);
      }
    }
    for (Item it : aItems) {
      if (it instanceof PolylineTrace t) {
        BoardGen.traceOp("TRACENC", t.polyline().lines, t.getLayer(), t.getHalfWidth(), t.netNumbers, t.clearanceClassIndex(), t.getFixedState());
      }
    }

    // ---- state 1: as loaded, some escape vias, pre-existing violation count
    for (Via v : b.getVias()) {
      if (rnd.nextInt(4) == 0) {
        int layer = v.firstLayer() + rnd.nextInt(v.lastLayer() - v.firstLayer() + 1);
        BoardGen.op("ESCVIA " + v.getId() + " " + layer);
        v.isEscapeVia = true;
        v.escapeViaSmdLayer = layer;
      }
    }
    int pre = rnd.nextInt(4);
    BoardGen.op("PREEXIST " + pre);
    b.preExistingClearanceViolationsCount = pre;
    score(b);

    // ---- state 2: part of the routing removed, conflicting traces added
    List<Item> items = new ArrayList<>(b.getItems());
    for (Item it : items) {
      if ((it instanceof Trace || it instanceof Via) && rnd.nextInt(3) == 0) {
        BoardGen.op("REMOVE " + it.getId());
        b.removeItem(it);
      }
    }
    List<Pin> pins = new ArrayList<>(b.getPins());
    int added = 0;
    for (int tries = 0; tries < 200 && added < 12 && pins.size() >= 2; tries++) {
      Pin p1 = pins.get(rnd.nextInt(pins.size()));
      Pin p2 = pins.get(rnd.nextInt(pins.size()));
      if (p1.netCount() != 1 || p1.sharesNet(p2)) {
        continue;
      }
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
      FixedState fs = rnd.nextInt(4) == 0 ? FixedState.SHOVE_FIXED : FixedState.UNFIXED;
      BoardGen.traceOp("TRACENC", pl.lines, layer, 100 + rnd.nextInt(2000), p1.netNumbers, 1, fs);
      added++;
    }
    score(b);

    // ---- state 3: unrouted
    for (Item it : new ArrayList<>(b.getItems())) {
      if (it instanceof Trace || it instanceof Via) {
        BoardGen.op("REMOVE " + it.getId());
        b.removeItem(it);
      }
    }
    score(b);
    BoardGen.out.flush();
  }

  /** The rules the replay setup does not contain: via infos and rules, net class via rules and length limits. */
  static void emitScoringSetup(BasicBoard a, Double tolerance) {
    BoardRules rules = a.rules;
    if (tolerance != null) {
      rules.clearanceToleranceUm = tolerance;
    }
    BoardGen.op("TOLERANCE " + BoardGen.d(rules.clearanceToleranceUm));
    String hv = a.communication.specctraParserInfo == null ? null : a.communication.specctraParserInfo.hostVersion;
    BoardGen.op("HOSTVERSION " + BoardGen.str(hv));
    List<ViaInfo> infos = new ArrayList<>();
    for (int i = 0; i < rules.viaInfos.count(); i++) {
      ViaInfo vi = rules.viaInfos.get(i);
      infos.add(vi);
      BoardGen.op("VIAINFO " + BoardGen.str(vi.getName()) + " " + vi.getPadstack().id + " " + vi.getClearanceClassIndex() + " " + BoardGen.b(vi.attachSmdAllowed()));
    }
    for (ViaRule rule : rules.viaRules) {
      StringBuilder sb = new StringBuilder("VIARULE ").append(BoardGen.str(rule.name)).append(' ').append(rule.viaCount());
      for (int i = 0; i < rule.viaCount(); i++) {
        sb.append(' ').append(infos.indexOf(rule.getVia(i)));
      }
      BoardGen.op(sb.toString());
    }
    // net classes in the order of the replay setup (see BoardGen.emitSetup)
    List<NetClass> classes = new ArrayList<>();
    for (int i = 0; i < rules.netClasses.count(); i++) {
      classes.add(rules.netClasses.get(i));
    }
    for (int i = 1; i <= rules.nets.maxNetNumber(); i++) {
      NetClass nc = rules.nets.get(i).getNetClass();
      if (nc != null && !classes.contains(nc)) {
        classes.add(nc);
      }
    }
    for (int i = 0; i < classes.size(); i++) {
      NetClass nc = classes.get(i);
      BoardGen.op("NCRULES " + i + " " + rules.viaRules.indexOf(nc.getViaRule()) + " " + BoardGen.d(nc.getMinimumTraceLength()) + " " + BoardGen.d(nc.getMaximumTraceLength()));
    }
  }

  // ------------------------------------------------------------------------------------------

  static String f(Float v) {
    return v == null ? "-" : Integer.toHexString(Float.floatToRawIntBits(v));
  }

  static String f(float v) {
    return Integer.toHexString(Float.floatToRawIntBits(v));
  }

  static String dd(Double v) {
    return v == null ? "-" : BoardGen.d(v);
  }

  static String i(Integer v) {
    return v == null ? "-" : v.toString();
  }

  static String rect(java.awt.geom.Rectangle2D.Float r) {
    return r == null ? "-" : f(r.x) + "," + f(r.y) + "," + f(r.width) + "," + f(r.height);
  }

  static String violation(ClearanceViolation v) {
    return v.firstItem.getId() + " " + v.secondItem.getId() + " " + v.layer + " " + BoardGen.d(v.expectedClearance) + " " + BoardGen.d(v.actualClearance) + " "
        + BoardGen.shape(v.shape) + " " + BoardGen.b(v.isUnfixable()) + " " + v.getCategory().ordinal();
  }

  static void score(BasicBoard b) {
    BoardGen.op("SCORE");
    // clearance violations
    DesignRulesChecker drc = new DesignRulesChecker(b, null);
    Collection<ClearanceViolation> all = drc.getAllClearanceViolations();
    BoardGen.res("V " + all.size());
    for (ClearanceViolation v : all) {
      BoardGen.res("v " + violation(v));
    }
    List<String> raw = new ArrayList<>();
    for (Item it : b.getItems()) {
      for (ClearanceViolation v : it.clearanceViolations()) {
        raw.add(violation(v));
      }
    }
    BoardGen.emitBlock("RAW " + raw.size(), raw, false);
    // incompletes
    drc.calculateAllIncompletes();
    int total = drc.getIncompleteCount();
    StringBuilder sb = new StringBuilder("I " + drc.maxConnections + " " + total + " " + drc.getLengthViolationCount() + " [");
    for (int net = 1; net <= b.rules.nets.maxNetNumber(); net++) {
      NetIncompletes ni = drc.getNetIncompletes(net);
      if (ni.count() > 0 || ni.getConnectedGroupCount() > 1) {
        sb.append(' ').append(net).append(':').append(ni.count()).append('/').append(ni.getConnectedGroupCount());
      }
    }
    BoardGen.res(sb.append(" ] ").append(drc.incompleteNetNumbers()).toString());
    // only the number of airlines: which airlines are created depends on identity hash order
    BoardGen.res("AIRLINES " + drc.getAllAirlines().length);
    // fanout details
    List<String> escaped = new ArrayList<>();
    for (Pin p : b.getSmdPins()) {
      escaped.add(p.getId() + " " + BoardGen.b(BoardStatistics.isPinEscaped(p)));
    }
    BoardGen.emitBlock("ESCAPED " + escaped.size(), escaped, false);
    // statistics
    RouterSettings defaults = new DefaultSettings().getSettings();
    RouterSettings v1 = defaults.clone();
    v1.routerScoring.version = RouterScoringVersion.V1_LEGACY;
    v1.optimizerScoring.version = OptimizerScoringVersion.V1_LEGACY;
    RouterSettings v2 = defaults.clone();
    v2.routerScoring.version = RouterScoringVersion.V2_CONTINUOUS;
    v2.optimizerScoring.version = OptimizerScoringVersion.V2_LOWER_BOUND;
    BoardStatistics full = new BoardStatistics(b);
    stats("full", full);
    scores(full, defaults, v1, v2);
    BoardStatistics noViolations = new BoardStatistics(b, null, false);
    stats("nocv", noViolations);
    scores(noViolations, defaults, v1, v2);
    BoardStatistics inch = new BoardStatistics(b, Unit.INCH, false, false);
    stats("inch", inch);
    BoardGen.res("SC inch " + f(inch.getOptimizerScore(v2)) + " " + f(inch.getRouterScore(v2)));
  }

  static void scores(BoardStatistics s, RouterSettings defaults, RouterSettings v1, RouterSettings v2) {
    BoardGen.res("SC " + f(s.getRouterScore(defaults)) + " " + f(s.getOptimizerScore(defaults)) + " " + f(s.getRouterScore(v1)) + " " + f(s.getOptimizerScore(v1)) + " "
        + f(s.getRouterScore(v2)) + " " + f(s.getOptimizerScore(v2)) + " " + f(s.getRouterScore((RouterSettings) null)) + " " + f(s.getOptimizerScore((RouterSettings) null))
        + " " + f(s.calculateScore(defaults.scoring)) + " " + f(s.getMaximumScore(defaults.scoring)));
  }

  static void stats(String label, BoardStatistics s) {
    BoardGen.res("S " + label + " " + BoardGen.str(s.host) + " " + s.unit);
    BoardGen.res("S board " + rect(s.board.boundingBox) + " " + rect(s.board.size) + " " + f(s.board.areaCm2));
    BoardGen.res("S layers " + i(s.layers.totalCount) + " " + i(s.layers.signalCount));
    BoardGen.res("S items " + i(s.items.totalCount) + " " + i(s.items.traceCount) + " " + i(s.items.viaCount) + " " + i(s.items.conductionAreaCount) + " " + i(s.items.drillItemCount)
        + " " + i(s.items.pinCount) + " " + i(s.items.componentOutlineCount) + " " + i(s.items.otherCount));
    BoardGen.res("S counts " + i(s.components.totalCount) + " " + i(s.pads.totalCount) + " " + i(s.nets.totalCount) + " " + i(s.nets.classCount));
    BoardGen.res("S connections " + i(s.connections.maximumCount) + " " + i(s.connections.incompleteCount));
    BoardGen.res("S traces " + i(s.traces.totalCount) + " " + i(s.traces.totalSegmentCount) + " " + f(s.traces.totalLength) + " " + f(s.traces.totalLengthMm) + " "
        + f(s.traces.totalWeightedLength) + " " + f(s.traces.averageLength) + " " + f(s.traces.totalVerticalLength) + " " + f(s.traces.totalHorizontalLength) + " "
        + f(s.traces.totalAngledLength));
    BoardGen.res("S bends " + i(s.bends.totalCount) + " " + i(s.bends.ninetyDegreeCount) + " " + i(s.bends.fortyFiveDegreeCount) + " " + i(s.bends.otherAngleCount));
    BoardGen.res("S vias " + i(s.vias.totalCount) + " " + i(s.vias.throughHoleCount) + " " + i(s.vias.blindCount) + " " + i(s.vias.buriedCount));
    var cv = s.clearanceViolations;
    BoardGen.res("S cv " + i(cv.totalCount) + " " + i(cv.preExistingCount) + " " + i(cv.unfixableCount) + " " + i(cv.routerIntroducedCount) + " " + dd(cv.totalViolationUm) + " "
        + dd(cv.minViolationUm) + " " + dd(cv.maxViolationUm) + " " + dd(cv.avgViolationUm));
    var d = s.difficulty;
    BoardGen.res("S difficulty " + i(d.pinCount) + " " + i(d.signalLayerCount) + " " + i(d.complexityC) + " " + f(d.difficultyD) + " " + f(d.boardAreaCm2));
    BoardGen.res("S bounds " + f(s.bounds.minTraceLengthMm) + " " + i(s.bounds.minViaCount) + " " + i(s.bounds.minBendCount));
    BoardGen.res("S fanout " + s.fanout.totalSmdPins + " " + s.fanout.pinsToEscape + " " + s.fanout.escapedCount);
  }
}
