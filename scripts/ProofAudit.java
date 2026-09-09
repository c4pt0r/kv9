import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashSet;
import java.util.HexFormat;
import java.util.List;
import java.util.Set;
import tla2sany.drivers.SANY;
import tla2sany.modanalyzer.SpecObj;
import tla2sany.semantic.LeafProofNode;
import tla2sany.semantic.ModuleNode;
import tla2sany.semantic.SemanticNode;
import tla2sany.st.Location;
import util.SimpleFilenameToStream;

/** Audit parsed modules; indentation and comments cannot hide module axioms. */
public class ProofAudit {
    static String quote(String value) {
        return "\"" + value.replace("\\", "\\\\").replace("\"", "\\\"")
                .replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t") + "\"";
    }

    static String sourceHash(Path file, Location location) throws Exception {
        List<String> lines = Files.readAllLines(file, StandardCharsets.UTF_8);
        StringBuilder source = new StringBuilder();
        for (int line = location.beginLine(); line <= location.endLine(); line++) {
            String text = lines.get(line - 1);
            int first = line == location.beginLine() ? location.beginColumn() - 1 : 0;
            int last = line == location.endLine() ? location.endColumn() : text.length();
            source.append(text, first, last).append('\n');
        }
        return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256")
                .digest(source.toString().getBytes(StandardCharsets.UTF_8)));
    }

    static boolean hasHole(SemanticNode node, Set<SemanticNode> seen) {
        if (node == null) return true;
        if (!seen.add(node)) return false;
        if (node instanceof LeafProofNode leaf && leaf.getOmitted()) return true;
        SemanticNode[] children = node.getChildren();
        if (children != null) {
            for (SemanticNode child : children) {
                if (child != null && hasHole(child, seen)) return true;
            }
        }
        return false;
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 4) throw new IllegalArgumentException("root, proof dir, model dir, stdlib required");
        var resolver = new SimpleFilenameToStream(new String[]{args[1], args[2], args[3]});
        var spec = new SpecObj(args[0], resolver);
        int status = SANY.frontEndMain(spec, args[0], System.err);
        if (status != 0 || spec.getErrorLevel() != 0 || spec.getSemanticErrors().isFailure()) {
            throw new IllegalStateException("SANY rejected the proof inventory");
        }
        Set<ModuleNode> modules = new HashSet<>(spec.getRootModule().getExtendedModuleSet(true));
        modules.add(spec.getRootModule());
        List<String> records = new ArrayList<>();
        for (ModuleNode module : modules) {
            String name = module.getName().toString();
            Path file = resolver.resolve(name, true).toPath().toRealPath();
            List<String> assumptions = new ArrayList<>();
            for (var assumption : module.getAssumptions()) {
                if (assumption.getLocation().source().equals(name)) {
                    assumptions.add("{\"axiom\":" + assumption.getIsAxiom()
                            + ",\"sha256\":" + quote(sourceHash(file, assumption.getLocation())) + "}");
                }
            }
            List<String> theorems = new ArrayList<>();
            for (var theorem : module.getTheorems()) {
                if (theorem.getLocation().source().equals(name)) {
                    theorems.add("{\"name\":" + quote(theorem.getName().toString())
                            + ",\"hole\":" + hasHole(theorem.getProof(), new HashSet<>()) + "}");
                }
            }
            Collections.sort(theorems);
            records.add("{\"module\":" + quote(name) + ",\"file\":" + quote(file.toString())
                    + ",\"sha256\":" + quote(HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256")
                            .digest(Files.readAllBytes(file))))
                    + ",\"assumptions\":[" + String.join(",", assumptions) + "]"
                    + ",\"theorems\":[" + String.join(",", theorems) + "]"
                    + ",\"instances\":" + module.getInstances().length
                    + ",\"inner_modules\":" + module.getInnerModules().length + "}");
        }
        Collections.sort(records);
        System.out.println("AUDIT_JSON=[" + String.join(",", records) + "]");
    }
}
