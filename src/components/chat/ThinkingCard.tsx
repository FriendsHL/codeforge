import { useEffect, useRef, useState } from "react";
import { CaretDownOutlined, CaretRightOutlined } from "@ant-design/icons";

/** 思考过程卡片：流式时实时展开滚动，结束后自动折叠成一行 */
export function ThinkingCard({ text, live }: { text: string; live: boolean }) {
  const [open, setOpen] = useState(live);
  const bodyRef = useRef<HTMLDivElement>(null);

  // 思考结束 → 自动折叠
  useEffect(() => {
    if (!live) setOpen(false);
  }, [live]);

  // 流式期间自动滚到最新
  useEffect(() => {
    if (live && open) {
      bodyRef.current?.scrollTo({ top: bodyRef.current.scrollHeight });
    }
  }, [text, live, open]);

  return (
    <div className={`thinking-card ${live ? "thinking-live" : ""}`}>
      <div className="thinking-header" onClick={() => setOpen((o) => !o)}>
        {open ? <CaretDownOutlined /> : <CaretRightOutlined />}
        <span>💭 思考过程</span>
        <span className="thinking-meta">
          {live ? "思考中…" : `${text.length} 字`}
        </span>
      </div>
      {open && (
        <div className="thinking-body" ref={bodyRef}>
          {text}
        </div>
      )}
    </div>
  );
}
