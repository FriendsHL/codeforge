import { useMemo } from "react";
import { highlightCode, languageForPath } from "../../lib/highlight";

interface DiffRow {
  kind: "hunk" | "add" | "del" | "ctx";
  oldNo?: number;
  newNo?: number;
  text: string;
}

/** 解析 unified diff：去掉 diff --git/index 等头部噪音，算出双侧行号 */
function parseDiff(diff: string): DiffRow[] {
  const rows: DiffRow[] = [];
  let oldNo = 0;
  let newNo = 0;

  for (const line of diff.split("\n")) {
    if (
      line.startsWith("diff --git") ||
      line.startsWith("index ") ||
      line.startsWith("--- ") ||
      line.startsWith("+++ ") ||
      line.startsWith("new file") ||
      line.startsWith("deleted file") ||
      line.startsWith("old mode") ||
      line.startsWith("new mode") ||
      line.startsWith("similarity") ||
      line.startsWith("rename ") ||
      line.startsWith("\\") // "\ No newline at end of file"
    ) {
      continue;
    }
    if (line.startsWith("@@")) {
      const m = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@ ?(.*)$/.exec(line);
      if (m) {
        oldNo = Number(m[1]);
        newNo = Number(m[2]);
        rows.push({ kind: "hunk", text: m[3] ?? "" });
      }
      continue;
    }
    if (line.startsWith("+")) {
      rows.push({ kind: "add", newNo: newNo++, text: line.slice(1) });
    } else if (line.startsWith("-")) {
      rows.push({ kind: "del", oldNo: oldNo++, text: line.slice(1) });
    } else {
      rows.push({
        kind: "ctx",
        oldNo: oldNo++,
        newNo: newNo++,
        text: line.startsWith(" ") ? line.slice(1) : line,
      });
    }
  }
  return rows;
}

export function DiffView({ path, diff }: { path: string; diff: string }) {
  const lang = languageForPath(path);
  const rows = useMemo(() => parseDiff(diff), [diff]);

  return (
    <div className="diff-view">
      {rows.map((row, i) =>
        row.kind === "hunk" ? (
          <div key={i} className="diff-row diff-hunk-row">
            <span className="diff-gutter">⋯</span>
            <span className="diff-gutter">⋯</span>
            <span className="diff-marker" />
            <span className="diff-code">{row.text}</span>
          </div>
        ) : (
          <div key={i} className={`diff-row diff-row-${row.kind}`}>
            <span className="diff-gutter">{row.oldNo ?? ""}</span>
            <span className="diff-gutter">{row.newNo ?? ""}</span>
            <span className="diff-marker">
              {row.kind === "add" ? "+" : row.kind === "del" ? "-" : " "}
            </span>
            <span
              className="diff-code"
              dangerouslySetInnerHTML={{ __html: highlightCode(row.text, lang) || " " }}
            />
          </div>
        ),
      )}
    </div>
  );
}
