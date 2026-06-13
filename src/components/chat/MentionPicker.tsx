import { useEffect, useRef, useState } from "react";
import { FileOutlined } from "@ant-design/icons";
import { searchFiles } from "../../lib/ipc";

/** @ 文件选择浮层：输入框打 @ 时弹出，键盘上下选择、Enter/点击确认 */
export function MentionPicker({
  query,
  onPick,
  onClose,
}: {
  query: string;
  onPick: (path: string) => void;
  onClose: () => void;
}) {
  const [files, setFiles] = useState<string[]>([]);
  const [active, setActive] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let alive = true;
    void searchFiles(query)
      .then((r) => {
        if (alive) {
          setFiles(r);
          setActive(0);
        }
      })
      .catch(() => setFiles([]));
    return () => {
      alive = false;
    };
  }, [query]);

  // 键盘导航：捕获阶段拦截，避免与输入框冲突
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (files.length === 0) {
        if (e.key === "Escape") onClose();
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActive((a) => Math.min(a + 1, files.length - 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setActive((a) => Math.max(a - 1, 0));
      } else if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        onPick(files[active]);
      } else if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [files, active, onPick, onClose]);

  useEffect(() => {
    listRef.current?.querySelector(".mention-active")?.scrollIntoView({ block: "nearest" });
  }, [active]);

  if (files.length === 0) return null;

  return (
    <div className="mention-picker" ref={listRef}>
      {files.map((f, i) => {
        const name = f.split("/").pop() ?? f;
        const dir = f.slice(0, f.length - name.length);
        return (
          <div
            key={f}
            className={`mention-item ${i === active ? "mention-active" : ""}`}
            onMouseEnter={() => setActive(i)}
            onMouseDown={(e) => {
              e.preventDefault();
              onPick(f);
            }}
          >
            <FileOutlined />
            <span className="mention-name">{name}</span>
            <span className="mention-dir">{dir}</span>
          </div>
        );
      })}
    </div>
  );
}
