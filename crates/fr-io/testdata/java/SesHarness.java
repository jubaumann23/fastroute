// Ground truth for fr-io's SES writer / SES reader / post-load processing.
//
// Usage: java SesHarness OUT_DIR BASE_DIR ENTRY...
//   ENTRY = DSN            -> OUT/<name>.ses          (SesWriter after DsnReader.readBoard)
//                             OUT/<name>.post.dump    (board after HeadlessBoardManager load,
//                                                      default settings)
//   ENTRY = DSN::SES       -> OUT/<name>+<ses stem>.ses and .dump (DsnReader.readBoard,
//                             SesReader.read, then SesWriter / board dump; the dump's second
//                             line is "session <SES path as given>")
// <name> is the DSN path relative to BASE_DIR, '/' -> "__", without ".dsn". The design name
// passed to SesWriter is the DSN file name. Needs DumpBoard.class on the class path.

import app.freerouting.board.facade.BasicBoard;
import app.freerouting.board.actions.ItemIdGenerator;
import app.freerouting.core.RoutingJob;
import app.freerouting.io.BoardReadResult;
import app.freerouting.io.specctra.DsnReader;
import app.freerouting.io.specctra.SesReader;
import app.freerouting.io.specctra.SesWriter;
import app.freerouting.management.HeadlessBoardManager;
import app.freerouting.settings.sources.DefaultSettings;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;

public class SesHarness {
  public static void main(String[] args) throws Exception {
    Path out = Paths.get(args[0]);
    Files.createDirectories(out);
    Path base = Paths.get(args[1]).toAbsolutePath().normalize();
    PrintStream realOut = System.out;
    for (int a = 2; a < args.length; a++) {
      String entry = args[a];
      String dsnArg = entry;
      String sesArg = null;
      int sep = entry.indexOf("::");
      if (sep >= 0) {
        dsnArg = entry.substring(0, sep);
        sesArg = entry.substring(sep + 2);
      }
      Path in = Paths.get(dsnArg).toAbsolutePath().normalize();
      String rel = base.relativize(in).toString().replace('\\', '/');
      String name = rel.replaceAll("(?i)\\.dsn$", "").replace("/", "__");
      String designName = in.getFileName().toString();
      try {
        BoardReadResult r;
        try (InputStream is = new FileInputStream(in.toFile())) {
          r = DsnReader.readBoard(is, null, null, designName);
        }
        if (!(r instanceof BoardReadResult.Success s)) {
          realOut.println(name + ": not loaded");
          continue;
        }
        BasicBoard board = s.board();
        String suffix = "";
        if (sesArg != null) {
          suffix = "+" + Paths.get(sesArg).getFileName().toString().replaceAll("(?i)\\.ses$", "");
          try (InputStream sis = new FileInputStream(sesArg)) {
            SesReader.read(sis, board);
          }
          try (PrintWriter w =
              new PrintWriter(
                  Files.newBufferedWriter(out.resolve(name + suffix + ".dump"), StandardCharsets.UTF_8))) {
            w.println("source " + rel);
            w.println("session " + sesArg);
            w.println("result OK");
            DumpBoard.dump(board, w);
          }
        }
        try (OutputStream os = Files.newOutputStream(out.resolve(name + suffix + ".ses"))) {
          SesWriter.write(board, os, designName);
        }
        if (sesArg == null) {
          RoutingJob job = new RoutingJob();
          job.routerSettings = new DefaultSettings().getSettings();
          HeadlessBoardManager hm = new HeadlessBoardManager(job);
          try (InputStream is = new FileInputStream(in.toFile())) {
            hm.loadFromSpecctraDsn(is, null, new ItemIdGenerator());
          }
          try (PrintWriter w =
              new PrintWriter(
                  Files.newBufferedWriter(out.resolve(name + ".post.dump"), StandardCharsets.UTF_8))) {
            w.println("source " + rel);
            w.println("result OK");
            DumpBoard.dump(hm.getRoutingBoard(), w);
          }
        }
        realOut.println(name + suffix + ": ok");
      } catch (Throwable t) {
        realOut.println(name + ": EXCEPTION " + t);
      }
    }
    System.exit(0);
  }
}
