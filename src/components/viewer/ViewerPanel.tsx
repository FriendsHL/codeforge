import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Alert, Button, Input, Segmented, Tag, Typography } from "antd";
import {
  CloseOutlined,
  DiffOutlined,
  FileTextOutlined,
  GlobalOutlined,
  LeftOutlined,
  ReloadOutlined,
  RightOutlined,
} from "@ant-design/icons";
import { highlightCode, languageForPath } from "../../lib/highlight";
import { useViewerStore } from "../../stores/viewerStore";
import { DiffView } from "../explorer/DiffView";

function normalizeUrl(input: string): string {
  const trimmed = input.trim();
  if (!trimmed) return "";
  return /^https?:\/\//.test(trimmed) ? trimmed : `http://${trimmed}`;
}

/** 原生子 WebView 浏览器：占位 div 量尺寸，Rust 端把真 WebView 叠在这块区域上 */
function BrowserView({ url }: { url: string }) {
  const [draft, setDraft] = useState(url);
  const [current, setCurrent] = useState(url);
  const [unreachable, setUnreachable] = useState(false);
  const holderRef = useRef<HTMLDivElement>(null);

  const rect = () => {
    const r = holderRef.current?.getBoundingClientRect();
    return r ? { x: r.x, y: r.y, width: r.width, height: r.height } : null;
  };

  const show = (target: string) => {
    const bounds = rect();
    if (!bounds) return;
    void invoke<boolean>("probe_url", { url: target }).then((ok) => setUnreachable(!ok));
    void invoke("browser_show", { url: target, ...bounds }).catch(() => setUnreachable(true));
  };

  // url 变化（含 agent 的 browser_open）→ 导航
  useEffect(() => {
    setDraft(url);
    setCurrent(url);
    const timer = setTimeout(() => show(url), 60); // 等布局稳定后再挂子 webview
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [url]);

  // 面板生命周期：位置同步 + 卸载时销毁子 webview
  useEffect(() => {
    const sync = () => {
      const bounds = rect();
      if (bounds) void invoke("browser_bounds", bounds);
    };
    const observer = new ResizeObserver(sync);
    if (holderRef.current) observer.observe(holderRef.current);
    window.addEventListener("resize", sync);

    return () => {
      observer.disconnect();
      window.removeEventListener("resize", sync);
      void invoke("browser_close");
    };
  }, []);

  const go = (target?: string) => {
    const next = normalizeUrl(target ?? draft);
    if (!next) return;
    setDraft(next);
    setCurrent(next);
    localStorage.setItem("codeforge.browser.url", next);
    show(next);
  };

  return (
    <div className="browser-view">
      <div className="browser-bar">
        <Button
          size="small"
          icon={<LeftOutlined />}
          onClick={() => void invoke("browser_history", { forward: false })}
        />
        <Button
          size="small"
          icon={<RightOutlined />}
          onClick={() => void invoke("browser_history", { forward: true })}
        />
        <Input
          size="small"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onPressEnter={() => go()}
          placeholder="http://localhost:3000"
        />
        <Button size="small" type="primary" onClick={() => go()}>
          打开
        </Button>
        <Button size="small" icon={<ReloadOutlined />} onClick={() => go(current)} />
      </div>
      {unreachable && (
        <Alert
          type="warning"
          showIcon
          banner
          message={`连不上 ${current} —— 请确认服务已启动，再点刷新`}
        />
      )}
      <div ref={holderRef} className="browser-frame" />
      <div className="browser-hint">
        原生 WebView 渲染，不受 X-Frame-Options 限制；外部网站也可以打开。
      </div>
    </div>
  );
}

/** markdown 文件：渲染效果 / 源码 双视图 */
function FileBody({ path, content }: { path: string; content: string }) {
  const isMarkdown = /\.(md|markdown)$/i.test(path);
  const [mode, setMode] = useState<"rendered" | "source">("rendered");

  if (isMarkdown) {
    return (
      <>
        <Segmented
          size="small"
          value={mode}
          onChange={(v) => setMode(v as "rendered" | "source")}
          options={[
            { label: "渲染", value: "rendered" },
            { label: "源码", value: "source" },
          ]}
          style={{ marginBottom: 8 }}
        />
        {mode === "rendered" ? (
          <div className="md-rendered">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{content}</ReactMarkdown>
          </div>
        ) : (
          <pre className="viewer-code">
            <code
              dangerouslySetInnerHTML={{ __html: highlightCode(content, "markdown") }}
            />
          </pre>
        )}
      </>
    );
  }

  return (
    <pre className="viewer-code">
      <code
        dangerouslySetInnerHTML={{
          __html:
            content.length > 150_000
              ? content.replace(/&/g, "&amp;").replace(/</g, "&lt;")
              : highlightCode(content, languageForPath(path)),
        }}
      />
    </pre>
  );
}

/** 文件 / diff / 浏览器查看器：位于主交互区与右栏之间 */
export function ViewerPanel() {
  const { content, close } = useViewerStore();
  if (!content) return null;

  const icon =
    content.type === "diff" ? (
      <DiffOutlined />
    ) : content.type === "browser" ? (
      <GlobalOutlined />
    ) : (
      <FileTextOutlined />
    );
  const title = content.type === "browser" ? "浏览器" : content.path;

  return (
    <div className="viewer-panel">
      <div className="viewer-header">
        {icon}
        <span className="viewer-path" title={title}>
          {title}
        </span>
        {content.type === "diff" && <Tag color="orange">diff</Tag>}
        <Button type="text" size="small" icon={<CloseOutlined />} onClick={close} />
      </div>
      {content.type === "browser" ? (
        <BrowserView url={content.url} />
      ) : (
        <div className="viewer-body">
          {content.type === "file" ? (
            <>
              {content.truncated && (
                <Typography.Text type="warning" style={{ fontSize: 12 }}>
                  文件过大，仅显示前 200KB
                </Typography.Text>
              )}
              <FileBody path={content.path} content={content.content} />
            </>
          ) : (
            <DiffView path={content.path} diff={content.diff} />
          )}
        </div>
      )}
    </div>
  );
}
