// Ground-truth generator for fr-engine::autoroute (porting unit U8), using the real Freerouting
// classes of reference/bin/freerouting-parity.jar (built from the reference source). Extends
// ../board/BoardGen.java and ../routing/RoutingGen.java (same default package, compile together).
//
// Regenerate (from the workspace root; time limits disabled like the Rust side):
//   J=reference/jdk25/bin; D=crates/fr-engine/testdata; F=reference/freerouting/fixtures; P=reference/parity-baseline
//   $J/javac -cp reference/bin/freerouting-parity.jar -d /tmp/autoroutegen $D/board/BoardGen.java $D/routing/RoutingGen.java $D/autoroute/AutorouteGen.java
//   run() { $J/java -Dfreerouting.parity.disableTimeLimits=true -cp reference/bin/freerouting-parity.jar:/tmp/autoroutegen AutorouteGen "$@"; }
//   (see crates/fr-engine/testdata/autoroute/README.txt for the vector list)
//
// Arguments: <dsn> <ses or -> <angle: 45|90|none|keep> <seed> <searches> <connections per pass>
//            <passes> <remove fraction in percent> [expand|steps]
//
// The board is built like in RoutingGen (a RoutingBoard with the rules, library and components of
// the loaded board; the routed traces and vias of the SES, of which a random part is removed). The
// via infos, via rules and net class routing data are emitted as AR* operations. Then:
//  * ARSEARCH: the maze search of AutorouteEngine.autorouteConnection for one item (without the
//    insertion): a hash of the queue state after every step of the search, the found destination,
//    the complete rooms, the connection located by FoundConnectionLocator and the ripped items;
//    then the database is cleared like autorouteConnection does.
//  * ARCONN: AutorouteConnectionRouter.route (without the strict DRC check) for one item: the
//    result state and the ripped items, followed by a board dump.
// Options: "expand" writes all dumps expanded; "steps" writes every maze step of ARSEARCH.

import app.freerouting.autoroute.AutorouteAttemptResult;
import app.freerouting.autoroute.AutorouteAttemptState;
import app.freerouting.autoroute.expansion.*;
import app.freerouting.autoroute.maze.AutorouteControl;
import app.freerouting.autoroute.maze.AutorouteEngine;
import app.freerouting.autoroute.maze.MazeSearchEngine;
import app.freerouting.autoroute.path.FoundConnectionLocator;
import app.freerouting.autoroute.drill.DrillPage;
import app.freerouting.autoroute.drill.ExpansionDrill;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.board.facade.RoutingBoard;
import app.freerouting.board.model.items.*;
import app.freerouting.board.model.structure.*;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.geometry.planar.*;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.io.specctra.SesReader;
import app.freerouting.rules.*;
import app.freerouting.settings.RouterSettings;
import java.io.*;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.util.*;

public class AutorouteGen {
  static RoutingBoard R;
  static Random rnd;
  static RouterSettings S;
  static boolean steps;
  static boolean retain = Boolean.getBoolean("retain");

  public static void main(String[] args) throws Exception {
    BoardGen.out = new PrintStream(new BufferedOutputStream(new FileOutputStream(FileDescriptor.out), 1 << 16), false, "UTF-8");
    System.setOut(new PrintStream(OutputStream.nullOutputStream()));
    String dsn = args[0];
    String ses = args[1];
    String angle = args[2];
    rnd = new Random(Long.parseLong(args[3]));
    BoardGen.rnd = rnd;
    RoutingGen.rnd = rnd;
    int searches = Integer.parseInt(args[4]);
    int connections = Integer.parseInt(args[5]);
    int passes = Integer.parseInt(args[6]);
    int removePercent = Integer.parseInt(args[7]);
    BoardGen.full = false;
    BoardGen.expandAll = args.length > 8 && args[8].equals("expand");
    steps = args.length > 8 && args[8].equals("steps");
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
    RoutingGen.R = R;
    StringBuilder sb = new StringBuilder("BOARD ").append(outlineShapes.length).append(' ').append(ao.clearanceClassIndex());
    for (PolylineShape s : outlineShapes) {
      sb.append(' ').append(BoardGen.shape(s));
    }
    op(sb.toString());
    emitRules(a);
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
    // remove a part of the routed traces and vias
    if (removePercent > 0) {
      List<Item> routeItems = new ArrayList<>();
      for (Item it : R.getItems()) {
        if ((it instanceof Trace || it instanceof Via) && !it.isUserFixed()) {
          routeItems.add(it);
        }
      }
      routeItems.sort(Comparator.comparingInt(Item::getId));
      for (Item it : routeItems) {
        if (rnd.nextInt(100) < removePercent && it.isOnTheBoard()) {
          op("REMOVE " + it.getId());
          R.removeItem(it);
        }
      }
    }
    S = new RouterSettings(R);
    S.viasAllowed = true;
    S.automaticNeckdown = true;
    S.setViaCosts(50);
    S.setPlaneViaCosts(5);
    S.setStartRipupCosts(100);
    S.tracePullTightAccuracy = 500;
    S.fanout.minEscapeLengthMm = 2.5;
    S.fanout.maxEscapeLengthMm = 4.5;
    S.fanout.fallbackToBoardVias = true;
    if (Boolean.getBoolean("neck")) {
      S.neckWidthUm = 100.0;
    }
    emitSettings();
    op("ARMODE " + (steps ? "steps" : BoardGen.expandAll ? "expand" : "hash") + " " + BoardGen.b(retain));
    RoutingGen.dump();
    int fanouts = Integer.getInteger("fanouts", 0);
    for (int i = 0; i < fanouts; i++) {
      Pin pin = pickFanoutPin();
      if (pin == null) {
        break;
      }
      int ripupCosts = rnd.nextBoolean() ? -1 : S.getStartRipupCosts() * (1 + rnd.nextInt(3));
      op("ARFANOUT " + pin.getId() + " " + ripupCosts);
      AutorouteAttemptResult result = R.fanout(pin, S, ripupCosts, null, null);
      res(String.valueOf(result.state));
      RoutingGen.dump();
    }

    for (int i = 0; i < searches; i++) {
      Item item = pickItem();
      if (item == null) {
        break;
      }
      search(item, 1 + rnd.nextInt(3), false);
    }
    for (int pass = 1; pass <= passes; pass++) {
      for (int i = 0; i < connections; i++) {
        Item item = pickItem();
        if (item == null) {
          break;
        }
        route(item, pass);
      }
    }
    RoutingGen.dump();
    BoardGen.out.flush();
  }

  static void op(String s) {
    BoardGen.op(s);
  }

  static void res(String s) {
    BoardGen.res(s);
  }

  // ------------------------------------------------------------------------------------------
  // setup

  static void emitRules(app.freerouting.board.facade.BasicBoard a) {
    BoardRules rules = a.rules;
    for (int i = 0; i < rules.viaInfos.count(); i++) {
      ViaInfo v = rules.viaInfos.get(i);
      op("ARVIAINFO " + BoardGen.str(v.getName()) + " " + v.getPadstack().id + " " + v.getClearanceClassIndex() + " " + BoardGen.b(v.attachSmdAllowed()));
    }
    for (ViaRule vr : rules.viaRules) {
      StringBuilder sb = new StringBuilder("ARVIARULE ").append(BoardGen.str(vr.name)).append(' ').append(vr.viaCount());
      for (int i = 0; i < vr.viaCount(); i++) {
        sb.append(' ').append(BoardGen.str(vr.getVia(i).getName()));
      }
      op(sb.toString());
    }
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
    int layers = a.layerStructure.layers.length;
    for (NetClass nc : classes) {
      StringBuilder sb = new StringBuilder("ARNETCLASS ").append(BoardGen.str(nc.getName())).append(' ').append(nc.getTraceClearanceClass()).append(' ')
          .append(nc.getViaRule() == null ? "-" : BoardGen.str(nc.getViaRule().name)).append(' ').append(layers);
      for (int l = 0; l < layers; l++) {
        sb.append(' ').append(nc.getTraceHalfWidth(l)).append(' ').append(BoardGen.b(nc.isActiveRoutingLayer(l)));
      }
      op(sb.toString());
    }
  }

  static void emitSettings() {
    int n = R.getLayerCount();
    StringBuilder sb = new StringBuilder("ARSETTINGS ").append(n);
    AutorouteControl.ExpansionCostFactor[] costs = S.getTraceCosts();
    for (int i = 0; i < n; i++) {
      sb.append(' ').append(BoardGen.b(S.getLayerActive(i))).append(' ').append(BoardGen.d(costs[i].horizontal())).append(' ').append(BoardGen.d(costs[i].vertical()))
          .append(' ').append(BoardGen.d(S.getBendCost(i)));
    }
    sb.append(' ').append(BoardGen.b(S.getViasAllowed())).append(' ').append(BoardGen.b(S.getAutomaticNeckdown())).append(' ').append(S.getViaCosts()).append(' ')
        .append(S.getPlaneViaCosts()).append(' ').append(S.getStartRipupCosts()).append(' ').append(S.tracePullTightAccuracy).append(' ')
        .append(BoardGen.d(S.fanout.minEscapeLengthMm)).append(' ').append(BoardGen.d(S.fanout.maxEscapeLengthMm)).append(' ').append(BoardGen.d(S.getNeckWidthUm()));
    op(sb.toString());
  }

  /** A random SMD pin with one net for the fanout. */
  static Pin pickFanoutPin() {
    List<Pin> candidates = new ArrayList<>();
    for (Item it : R.getItems()) {
      if (it instanceof Pin p && p.netCount() == 1 && p.firstLayer() == p.lastLayer()) {
        candidates.add(p);
      }
    }
    candidates.sort(Comparator.comparingInt(Item::getId));
    if (candidates.isEmpty()) {
      return null;
    }
    return candidates.get(rnd.nextInt(candidates.size()));
  }

  /** A random connectable item with one net that is not connected to all items of its net. */
  static Item pickItem() {
    List<Item> candidates = new ArrayList<>();
    for (Item it : R.getItems()) {
      if (it instanceof Connectable && it.netCount() == 1 && !(it instanceof ConductionArea)) {
        candidates.add(it);
      }
    }
    candidates.sort(Comparator.comparingInt(Item::getId));
    for (int tries = 0; tries < 20 && !candidates.isEmpty(); tries++) {
      Item it = candidates.get(rnd.nextInt(candidates.size()));
      if (!it.getUnconnectedSet(it.getNetNumber(0)).isEmpty()) {
        return it;
      }
    }
    return null;
  }

  // ------------------------------------------------------------------------------------------
  // operations

  static AutorouteControl control(int netNo, int pass) {
    Net net = R.rules.nets.get(netNo);
    boolean plane = net != null && net.containsPlane();
    int viaCosts = plane ? S.getPlaneViaCosts() : S.getViaCosts();
    AutorouteControl ctrl = new AutorouteControl(R, netNo, S, viaCosts, S.getTraceCosts());
    ctrl.ripupAllowed = true;
    ctrl.ripupCosts = S.getStartRipupCosts() * pass;
    ctrl.removeUnconnectedVias = true;
    return ctrl;
  }

  static void route(Item item, int pass) throws Exception {
    int netNo = item.getNetNumber(0);
    if (Boolean.getBoolean("detail")) {
      search(item, pass, true);
      return;
    }
    op("ARCONN " + item.getId() + " " + netNo + " " + pass);
    SortedSet<Item> ripped = new TreeSet<>();
    AutorouteAttemptResult result = routeImpl(item, netNo, pass, ripped);
    res(result.state + " " + BoardGen.ids(ripped));
    RoutingGen.dump();
  }

  /** AutorouteConnectionRouter.route without the strict DRC check. */
  static AutorouteAttemptResult routeImpl(Item item, int routeNetNo, int pass, SortedSet<Item> ripped) {
    Net routeNet = R.rules.nets.get(routeNetNo);
    boolean containsPlane = routeNet != null && routeNet.containsPlane();
    AutorouteControl ctrl = control(routeNetNo, pass);
    Set<Item> unconnectedSet = item.getUnconnectedSet(routeNetNo);
    if (unconnectedSet.isEmpty()) {
      return new AutorouteAttemptResult(AutorouteAttemptState.NO_UNCONNECTED_NETS);
    }
    Set<Item> connectedSet = item.getConnectedSet(routeNetNo);
    Set<Item> start;
    Set<Item> dest;
    if (containsPlane) {
      for (Item it : connectedSet) {
        if (it instanceof ConductionArea) {
          return new AutorouteAttemptResult(AutorouteAttemptState.CONNECTED_TO_PLANE);
        }
      }
      start = connectedSet;
      dest = unconnectedSet;
    } else {
      start = unconnectedSet;
      dest = connectedSet;
    }
    AutorouteEngine engine = R.initAutoroute(routeNetNo, ctrl.traceClearanceClassIndex, null, null, retain);
    AutorouteAttemptResult result = engine.autorouteConnection(start, dest, ctrl, ripped, null);
    if (result.state == AutorouteAttemptState.ROUTED) {
      R.optChangedArea(new int[0], null, S.tracePullTightAccuracy, ctrl.traceCosts, null, 1000);
    }
    if ((result.state == AutorouteAttemptState.FAILED || result.state == AutorouteAttemptState.INSERT_ERROR) && S.getNeckWidthUm() > 0) {
      AutorouteAttemptResult necked = retryNecked(routeNetNo, ctrl, pass, start, dest, ripped);
      if (necked != null) {
        return necked;
      }
    }
    return result;
  }

  /** AutorouteConnectionRouter.retryConnectionNecked. */
  static AutorouteAttemptResult retryNecked(int routeNetNo, AutorouteControl originalControl, int pass, Set<Item> start, Set<Item> dest, SortedSet<Item> ripped) {
    int boardResolution = Math.max(1, R.communication.resolution);
    int neckWidth = (int) Math.round(Unit.scale(S.getNeckWidthUm() * boardResolution, Unit.UM, R.communication.unit));
    int neckHalfWidth = Math.max(1, neckWidth / 2);
    boolean narrowerSomewhere = false;
    for (int i = 0; i < originalControl.layerCount; i++) {
      if (originalControl.layerActive[i] && originalControl.traceHalfWidth[i] > neckHalfWidth) {
        narrowerSomewhere = true;
        break;
      }
    }
    if (!narrowerSomewhere) {
      return null;
    }
    AutorouteControl neckControl = control(routeNetNo, pass);
    for (int i = 0; i < neckControl.layerCount; i++) {
      int compensation = neckControl.compensatedTraceHalfWidth[i] - neckControl.traceHalfWidth[i];
      neckControl.traceHalfWidth[i] = Math.min(neckControl.traceHalfWidth[i], neckHalfWidth);
      neckControl.compensatedTraceHalfWidth[i] = neckControl.traceHalfWidth[i] + compensation;
    }
    AutorouteEngine neckEngine = R.initAutoroute(routeNetNo, neckControl.traceClearanceClassIndex, null, null, retain);
    AutorouteAttemptResult neckResult = neckEngine.autorouteConnection(start, dest, neckControl, ripped, null);
    if (neckResult.state != AutorouteAttemptState.ROUTED) {
      return null;
    }
    R.optChangedArea(new int[0], null, S.tracePullTightAccuracy, neckControl.traceCosts, null, 1000);
    return neckResult;
  }

  static void search(Item item, int pass, boolean insert) throws Exception {
    int netNo = item.getNetNumber(0);
    op((insert ? "ARDETAIL " : "ARSEARCH ") + item.getId() + " " + netNo + " " + pass);
    if (Boolean.getBoolean("infos")) {
      List<Item> items = new ArrayList<>(R.getItems());
      items.sort(Comparator.comparingInt(Item::getId));
      StringBuilder sb = new StringBuilder("INFOS");
      for (Item it : items) {
        if (it.getAutorouteInfoPur() != null) {
          sb.append(' ').append(it.getId());
        }
      }
      res(sb.toString());
    }
    Net routeNet = R.rules.nets.get(netNo);
    boolean containsPlane = routeNet != null && routeNet.containsPlane();
    AutorouteControl ctrl = control(netNo, pass);
    Set<Item> unconnectedSet = item.getUnconnectedSet(netNo);
    Set<Item> connectedSet = item.getConnectedSet(netNo);
    Set<Item> start;
    Set<Item> dest;
    if (containsPlane) {
      for (Item it : connectedSet) {
        if (it instanceof ConductionArea) {
          res("PLANE");
          return;
        }
      }
      start = connectedSet;
      dest = unconnectedSet;
    } else {
      start = unconnectedSet;
      dest = connectedSet;
    }
    AutorouteEngine engine = R.initAutoroute(netNo, ctrl.traceClearanceClassIndex, null, null, retain);
    MazeSearchEngine maze = MazeSearchEngine.getInstance(start, dest, engine, ctrl);
    if (maze == null) {
      res("NOINSTANCE");
      rooms(engine);
      RoutingGen.dump();
      return;
    }
    Field queueField = MazeSearchEngine.class.getDeclaredField("mazeExpansionList");
    queueField.setAccessible(true);
    SortedSet<?> queue = (SortedSet<?>) queueField.get(maze);
    List<String> stepLines = new ArrayList<>();
    stepLines.add(queueState(queue));
    boolean added = Boolean.getBoolean("added");
    for (;;) {
      IdentityHashMap<Object, Boolean> before = new IdentityHashMap<>();
      if (added) {
        for (Object o : queue) {
          before.put(o, Boolean.TRUE);
        }
      }
      boolean more = maze.occupyNextElement();
      stepLines.add(queueState(queue));
      if (added) {
        for (Object o : queue) {
          if (!before.containsKey(o)) {
            stepLines.add("+ " + elementDesc(o));
          }
        }
      }
      if (!more) {
        break;
      }
    }
    if (steps) {
      for (String l : stepLines) {
        res(l);
      }
    } else {
      BoardGen.emitBlock("STEPS " + stepLines.size(), stepLines, false);
    }
    MazeSearchEngine.Result result = maze.findConnection();
    if (result == null) {
      res("RESULT -");
    } else {
      res("RESULT " + kind(result.destinationDoor) + " " + result.sectionNoOfDoor + " " + result.destinationDoor.getId());
    }
    rooms(engine);
    SortedSet<Item> ripped = new TreeSet<>();
    FoundConnectionLocator loc = null;
    if (result != null) {
      try {
        loc = FoundConnectionLocator.getInstance(result, ctrl, engine.autorouteSearchTree, R.rules.getTraceAngleRestriction(), ripped, null);
      } catch (Exception e) {
        loc = null;
      }
      if (loc == null) {
        res("LOCATOR -");
      } else {
        StringBuilder sb = new StringBuilder("LOCATOR ").append(loc.startItem == null ? "-" : String.valueOf(loc.startItem.getId())).append(' ').append(loc.startLayer)
            .append(' ').append(loc.targetItem == null ? "-" : String.valueOf(loc.targetItem.getId())).append(' ').append(loc.targetLayer).append(' ')
            .append(loc.connectionItems.size());
        for (Object ri : loc.connectionItems) {
          IntPoint[] corners = (IntPoint[]) BoardGen.getField(ri, "corners");
          int layer = (Integer) BoardGen.getField(ri, "layer");
          sb.append(" | ").append(layer).append(' ').append(corners.length);
          for (IntPoint c : corners) {
            sb.append(' ').append(c.x).append(' ').append(c.y);
          }
        }
        res(sb.toString());
      }
    }
    res("RIPPED " + BoardGen.ids(ripped));
    if (retain) {
      java.lang.reflect.Method m = AutorouteEngine.class.getDeclaredMethod("resetAllDoors");
      m.setAccessible(true);
      m.invoke(engine);
    } else {
      engine.clear();
    }
    if (insert) {
      AutorouteAttemptState state;
      if (result == null || loc == null || !ctrl.layerActive[loc.startLayer] || !ctrl.layerActive[loc.targetLayer]) {
        state = AutorouteAttemptState.FAILED;
      } else {
        SortedSet<Item> rippedConnections = new TreeSet<>();
        Set<Integer> changedNets = new TreeSet<>();
        Item.StopConnectionOption opt = ctrl.removeUnconnectedVias ? Item.StopConnectionOption.NONE : Item.StopConnectionOption.FANOUT_VIA;
        for (Item it : ripped) {
          rippedConnections.addAll(it.getConnectionItems(opt));
          for (int i = 0; i < it.netCount(); i++) {
            changedNets.add(it.getNetNumber(i));
          }
        }
        R.removeItems(rippedConnections);
        for (int n : changedNets) {
          R.removeTraceTails(n, opt);
        }
        state = app.freerouting.autoroute.path.FoundConnectionInserter.getInstance(loc, R, ctrl) == null ? AutorouteAttemptState.FAILED : AutorouteAttemptState.ROUTED;
      }
      if (state == AutorouteAttemptState.ROUTED) {
        R.optChangedArea(new int[0], null, S.tracePullTightAccuracy, ctrl.traceCosts, null, 1000);
      }
      res("STATE " + state);
    }
    RoutingGen.dump();
  }

  static String kind(Object door) {
    if (door instanceof ExpansionDoor) {
      return "D";
    }
    if (door instanceof TargetItemExpansionDoor) {
      return "T";
    }
    if (door instanceof ExpansionDrill) {
      return "X";
    }
    if (door instanceof DrillPage) {
      return "P";
    }
    return "?";
  }

  static String elementDesc(Object e) throws Exception {
    double sv = (Double) BoardGen.getField(e, "sortingValue");
    double ev = (Double) BoardGen.getField(e, "expansionValue");
    int section = (Integer) BoardGen.getField(e, "sectionNoOfDoor");
    ExpandableObject door = (ExpandableObject) BoardGen.getField(e, "door");
    Object next = BoardGen.getField(e, "nextRoom");
    String nr = next == null ? "-" : String.valueOf(((ExpansionRoom) next).getId());
    return BoardGen.d(sv) + " " + BoardGen.d(ev) + " " + kind(door) + " " + door.getId() + " " + section + " " + nr;
  }

  static String queueState(SortedSet<?> queue) throws Exception {
    if (queue.isEmpty()) {
      return "0";
    }
    Object first = queue.first();
    double sv = (Double) BoardGen.getField(first, "sortingValue");
    double ev = (Double) BoardGen.getField(first, "expansionValue");
    int section = (Integer) BoardGen.getField(first, "sectionNoOfDoor");
    ExpandableObject door = (ExpandableObject) BoardGen.getField(first, "door");
    return queue.size() + " " + BoardGen.d(sv) + " " + BoardGen.d(ev) + " " + kind(door) + " " + door.getId() + " " + section;
  }

  @SuppressWarnings("unchecked")
  static void rooms(AutorouteEngine engine) throws Exception {
    List<CompleteFreeSpaceExpansionRoom> list = (List<CompleteFreeSpaceExpansionRoom>) BoardGen.getField(engine, "completeExpansionRooms");
    List<String> lines = new ArrayList<>();
    if (list != null) {
      for (CompleteFreeSpaceExpansionRoom room : list) {
        lines.add(room.getId() + " " + room.getLayer() + " " + BoardGen.shape(room.getShape()) + " d " + room.getDoors().size() + " t " + room.getTargetDoors().size());
      }
    }
    BoardGen.emitBlock("ROOMS " + lines.size(), lines, steps);
  }
}
