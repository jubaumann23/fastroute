// Ground-truth dump of a board loaded by the real Freerouting DsnReader.
//
// Build/run: see ../run_java_dump.sh. Usage: java DumpBoard OUT_DIR BASE_DIR DSN...
// For every DSN file writes OUT_DIR/<name>.dump, where <name> is the path relative to BASE_DIR
// with '/' replaced by "__" and without ".dsn" (format documented in src/dump.rs of fr-io;
// both sides must stay in sync). The first line is "source <relative path>".

import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.model.items.*;
import app.freerouting.board.model.structure.BoardOutline;
import app.freerouting.board.model.structure.Component;
import app.freerouting.board.trace.PolylineTrace;
import app.freerouting.core.library.Package;
import app.freerouting.core.library.Padstack;
import app.freerouting.geometry.planar.*;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.rules.*;
import java.io.*;
import java.lang.reflect.Field;
import java.math.BigInteger;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import app.freerouting.geometry.planar.Vector;

public class DumpBoard {

  static String b(boolean v) {
    return v ? "1" : "0";
  }

  static String d(double v) {
    return Double.toString(v);
  }

  static Object field(Object o, String name) throws Exception {
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

  static String pt(Point p) throws Exception {
    if (p == null) return "null";
    if (p instanceof IntPoint ip) return ip.x + "," + ip.y;
    RationalPoint rp = (RationalPoint) p;
    return "r(" + field(rp, "x") + "," + field(rp, "y") + "," + field(rp, "z") + ")";
  }

  static String vec(Vector v) throws Exception {
    if (v == null) return "null";
    if (v instanceof IntVector iv) return iv.x + "," + iv.y;
    RationalVector rv = (RationalVector) v;
    return "r(" + rv.x + "," + rv.y + "," + rv.z + ")";
  }

  static String line(Line l) throws Exception {
    return pt(l.a) + ";" + pt(l.b);
  }

  static String shape(Object s) throws Exception {
    if (s == null) return "null";
    if (s instanceof IntBox bx)
      return "box(" + bx.ll.x + "," + bx.ll.y + "," + bx.ur.x + "," + bx.ur.y + ")";
    if (s instanceof IntOctagon o)
      return "oct(" + o.leftX + "," + o.bottomY + "," + o.rightX + "," + o.topY + ","
          + o.upperLeftDiagonalX + "," + o.lowerRightDiagonalX + "," + o.lowerLeftDiagonalX + ","
          + o.upperRightDiagonalX + ")";
    if (s instanceof Simplex sx) {
      StringBuilder sb = new StringBuilder("simplex[");
      for (int i = 0; i < sx.borderLineCount(); i++) {
        if (i > 0) sb.append(" ");
        sb.append(line(sx.borderLine(i)));
      }
      return sb.append("]").toString();
    }
    if (s instanceof Circle c) return "circle(" + c.center.x + "," + c.center.y + "," + c.radius + ")";
    if (s instanceof PolygonShape p) {
      StringBuilder sb = new StringBuilder("polygon[");
      for (int i = 0; i < p.corners.length; i++) {
        if (i > 0) sb.append(" ");
        sb.append(pt(p.corners[i]));
      }
      return sb.append("]").toString();
    }
    if (s instanceof PolylineArea a) {
      StringBuilder sb = new StringBuilder("area(");
      sb.append(shape(a.getBorder()));
      for (PolylineShape h : a.getHoles()) sb.append(" hole ").append(shape(h));
      return sb.append(")").toString();
    }
    return "unknown:" + s.getClass().getSimpleName();
  }

  static String nets(Item it) {
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < it.netNumbers.length; i++) {
      if (i > 0) sb.append(",");
      sb.append(it.netNumbers[i]);
    }
    return sb.toString();
  }

  static void dump(BasicBoard board, PrintWriter w) throws Exception {
    var ls = board.layerStructure;
    for (int i = 0; i < ls.layers.length; i++)
      w.println("layer " + i + " " + ls.layers[i].name + " " + b(ls.layers[i].isSignal));
    var cm = board.rules.clearanceMatrix;
    for (int i = 0; i < cm.getClassCount(); i++) w.println("cclass " + i + " " + cm.getName(i));
    for (int i = 0; i < cm.getClassCount(); i++)
      for (int j = 0; j < cm.getClassCount(); j++) {
        StringBuilder sb = new StringBuilder("cval " + i + " " + j);
        for (int l = 0; l < cm.getLayerCount(); l++) sb.append(" ").append(cm.getValue(i, j, l, false));
        w.println(sb);
      }
    var rules = board.rules;
    for (int i = 0; i < rules.netClasses.count(); i++) {
      NetClass c = rules.netClasses.get(i);
      StringBuilder sb = new StringBuilder("netclass " + i + " " + c.getName());
      sb.append(" tcl=").append(c.getTraceClearanceClass());
      sb.append(" via_rule=").append(c.getViaRule() == null ? "-" : c.getViaRule().name);
      sb.append(" hw=");
      for (int l = 0; l < c.layerCount(); l++) sb.append(l > 0 ? "," : "").append(c.getTraceHalfWidth(l));
      sb.append(" items=");
      var ics = DefaultItemClearanceClasses.ItemClass.values();
      for (int k = 0; k < ics.length; k++)
        sb.append(k > 0 ? "," : "").append(c.defaultItemClearanceClasses.get(ics[k]));
      sb.append(" active=");
      for (int l = 0; l < c.layerCount(); l++) sb.append(l > 0 ? "," : "").append(b(c.isActiveRoutingLayer(l)));
      sb.append(" shove=").append(b(c.isShoveFixed()));
      sb.append(" pull=").append(b(c.getPullTight()));
      sb.append(" minlen=").append(d(c.getMinimumTraceLength()));
      sb.append(" maxlen=").append(d(c.getMaximumTraceLength()));
      sb.append(" ignored=").append(b(c.isIgnoredByAutorouter));
      w.println(sb);
    }
    for (int i = 1; i <= rules.nets.maxNetNumber(); i++) {
      Net n = rules.nets.get(i);
      w.println("net " + n.netNumber + " " + n.name + " " + n.subnetNumber + " class=" + n.getNetClass().getName()
          + " plane=" + b(n.containsPlane()));
    }
    for (int i = 0; i < rules.viaInfos.count(); i++) {
      ViaInfo v = rules.viaInfos.get(i);
      w.println("viainfo " + v.getName() + " " + v.getPadstack().name + " cl=" + v.getClearanceClassIndex()
          + " attach=" + b(v.attachSmdAllowed()));
    }
    for (ViaRule r : rules.viaRules) {
      StringBuilder sb = new StringBuilder("viarule " + r.name + " ");
      for (int i = 0; i < r.viaCount(); i++) sb.append(i > 0 ? "," : "").append(r.getVia(i).getName());
      w.println(sb);
    }
    w.println("rules min_hw=" + rules.getMinTraceHalfWidth() + " max_hw=" + rules.getMaxTraceHalfWidth()
        + " pin_edge=" + d(rules.getPinEdgeToTurnDist()) + " angle=" + rules.getTraceAngleRestriction());
    var lib = board.library;
    for (int i = 1; lib.padstacks != null && i <= lib.padstacks.count(); i++) {
      Padstack p = lib.padstacks.get(i);
      StringBuilder sb = new StringBuilder("padstack " + p.id + " " + p.name + " attach=" + b(p.attachAllowed)
          + " abs=" + b(p.placedAbsolute));
      for (int l = 0; l < p.boardLayerCount(); l++) sb.append(" ").append(l).append(":").append(shape(p.getShape(l)));
      w.println(sb);
    }
    StringBuilder vp = new StringBuilder("viapadstacks ");
    for (int i = 0; i < lib.viaPadstackCount(); i++) vp.append(i > 0 ? "," : "").append(lib.getViaPadstack(i).name);
    w.println(vp);
    for (int i = 1; lib.packages != null && i <= lib.packages.count(); i++) {
      Package p = lib.packages.get(i);
      w.println("package " + p.id + " " + p.name + " front=" + b(p.isFront));
      for (int k = 0; k < p.pinCount(); k++) {
        Package.Pin pin = p.getPin(k);
        w.println("  pin " + pin.name + " " + pin.padstackId + " " + vec(pin.relativeLocation) + " "
            + d(pin.rotationInDegree));
      }
      if (p.outline != null)
        for (int k = 0; k < p.outline.length; k++)
          w.println("  outline " + k + " " + shape(p.outline[k]) + " w=" + d(p.outlineWidths[k])
              + " closed=" + b(p.outlineIsClosed[k]));
      Package.Keepout[][] kos = {p.keepouts, p.viaKeepouts, p.placeKeepoutArr};
      for (int k = 0; k < 3; k++)
        for (Package.Keepout ko : kos[k])
          w.println("  keepout " + k + " " + ko.name + " " + ko.layer + " " + shape(ko.area));
    }
    for (int i = 1; i <= board.components.count(); i++) {
      Component c = board.components.get(i);
      w.println("component " + c.id + " " + c.name + " loc=" + pt(c.getLocation()) + " rot="
          + d(c.getRotationInDegree()) + " front=" + b(c.placedOnFront()) + " pkg=" + c.getPackage().id
          + " fixed=" + b(c.positionFixed) + " pn=" + c.getPartNumber());
    }
    w.println("bbox " + shape(board.getBoundingBox()));
    w.println("flip " + b(board.components.getFlipStyleRotateFirst()));

    List<Item> items = new ArrayList<>(board.getItems());
    items.sort(Comparator.comparingInt(Item::getId));
    for (Item it : items) {
      StringBuilder sb = new StringBuilder("item " + it.getId() + " " + it.getClass().getSimpleName());
      sb.append(" nets=").append(nets(it));
      sb.append(" cl=").append(it.clearanceClassIndex());
      sb.append(" fixed=").append(it.getFixedState());
      sb.append(" comp=").append(it.getComponentId());
      if (it instanceof BoardOutline o) {
        sb.append(" shapes=[");
        for (int i = 0; i < o.shapeCount(); i++) sb.append(i > 0 ? "|" : "").append(shape(o.getShape(i)));
        sb.append("]");
      } else if (it instanceof ConductionArea c) {
        sb.append(" layer=").append(c.getLayer()).append(" obstacle=").append(b(c.getIsObstacle()));
        sb.append(" rel=").append(shape(c.getRelativeArea()));
        sb.append(" abs=").append(shape(c.getArea()));
      } else if (it instanceof ObstacleArea o) {
        sb.append(" layer=").append(o.getLayer()).append(" name=").append(o.name);
        sb.append(" rel=").append(shape(o.getRelativeArea()));
        sb.append(" tr=").append(vec(o.getTranslation())).append(" rot=").append(d(o.getRotationInDegree()));
        sb.append(" side=").append(b(o.getSideChanged()));
        sb.append(" abs=").append(shape(o.getArea()));
      } else if (it instanceof ComponentOutline o) {
        sb.append(" front=").append(b(o.isFront())).append(" court=").append(b(o.isCourtyard()));
        sb.append(" fab=").append(b(o.isFabrication())).append(" closed=").append(b(o.isClosed()));
        sb.append(" rel=").append(shape(field(o, "relativeArea")));
        sb.append(" tr=").append(vec((Vector) field(o, "translation")));
        sb.append(" rot=").append(d((Double) field(o, "rotationInDegree")));
        sb.append(" abs=").append(shape(o.getArea()));
      } else if (it instanceof Pin p) {
        sb.append(" pin=").append(p.getPinIndex()).append(" first=").append(p.firstLayer());
        sb.append(" last=").append(p.lastLayer()).append(" center=").append(pt(p.getCenter()));
        sb.append(" shapes=[");
        int n = p.getPadstack().toLayer() - p.getPadstack().fromLayer() + 1;
        for (int i = 0; i < n; i++) sb.append(i > 0 ? "|" : "").append(shape(p.getShape(i)));
        sb.append("]");
      } else if (it instanceof Via v) {
        sb.append(" padstack=").append(v.getPadstack().name).append(" center=").append(pt(v.getCenter()));
        sb.append(" attach=").append(b(v.attachAllowed));
      } else if (it instanceof PolylineTrace t) {
        sb.append(" layer=").append(t.getLayer()).append(" hw=").append(t.getHalfWidth());
        sb.append(" corners=[");
        Polyline pl = t.polyline();
        for (int i = 0; i < pl.cornerCount(); i++) sb.append(i > 0 ? " " : "").append(pt(pl.corner(i)));
        sb.append("]");
      }
      w.println(sb);
    }
  }

  public static void main(String[] args) throws Exception {
    Path out = Paths.get(args[0]);
    Files.createDirectories(out);
    PrintStream realOut = System.out;
    Path base = Paths.get(args[1]).toAbsolutePath().normalize();
    for (int a = 2; a < args.length; a++) {
      Path in = Paths.get(args[a]).toAbsolutePath().normalize();
      String rel = base.relativize(in).toString().replace('\\', '/');
      String name = rel.replaceAll("(?i)\\.dsn$", "").replace("/", "__");
      Path target = out.resolve(name + ".dump");
      try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(target, StandardCharsets.UTF_8))) {
        w.println("source " + rel);
        BoardReadResult r;
        try (InputStream is = new FileInputStream(in.toFile())) {
          r = DsnReader.readBoard(is, null, null, name);
        } catch (Throwable t) {
          w.println("result EXCEPTION " + t.getClass().getSimpleName());
          realOut.println(name + ": EXCEPTION " + t);
          continue;
        }
        if (r instanceof BoardReadResult.Success s) {
          w.println("result OK");
          try {
            dump(s.board(), w);
          } catch (Throwable t) {
            w.println("dump EXCEPTION " + t);
          }
        } else if (r instanceof BoardReadResult.OutlineMissing) {
          w.println("result OUTLINE_MISSING");
        } else {
          w.println("result ERROR");
        }
        realOut.println(name + ": " + r.getClass().getSimpleName());
      }
    }
  }
}
