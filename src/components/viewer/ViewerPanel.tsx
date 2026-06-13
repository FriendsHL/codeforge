import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Alert, Button, Input, Segmented, Typography } from "antd";
import type { ReactElement } from "react";
import {
  CloseOutlined,
  DiffOutlined,
  FileTextOutlined,
  GlobalOutlined,
  LeftOutlined,
  ReloadOutlined,
  RightOutlined,
} from "@ant-design/icons";
import { CodeOutlined } from "@ant-design/icons";
import { highlightCode, languageForPath } from "../../lib/highlight";
import { useViewerStore, type ViewerTab } from "../../stores/viewerStore";
import { DiffView } from "../explorer/DiffView";
import { TerminalPanel } from "../terminal/TerminalPanel";

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

/** 文件/diff 内容体 */
function ContentBody({ content }: { content: NonNullable<ReturnType<typeof useViewerStore.getState>["content"]> }) {
  return (
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
  );
}

/** 多 tab 查看器：文件/diff、浏览器、终端同栏切换。位于主交互区与右栏之间 */
export function ViewerPanel() {
  const { content, browserUrl, terminalOpen, activeTab, setTab, closeTab } = useViewerStore();

  // 组装当前存在的 tab
  const tabs: { key: ViewerTab; icon: ReactElement; label: string }[] = [];
  if (content) {
    tabs.push({
      key: "content",
      icon: content.type === "diff" ? <DiffOutlined /> : <FileTextOutlined />,
      label: content.type === "diff" ? `diff: ${baseName(content.path)}` : baseName(content.path),
    });
  }
  if (browserUrl !== null) tabs.push({ key: "browser", icon: <GlobalOutlined />, label: "浏览器" });
  if (terminalOpen) tabs.push({ key: "terminal", icon: <CodeOutlined />, label: "终端" });

  if (tabs.length === 0) return null;

  // activeTab 可能指向已关闭的 tab，兜底到第一个存在的
  const active = tabs.some((t) => t.key === activeTab) ? activeTab : tabs[0].key;

  return (
    <div className="viewer-panel">
      <div className="viewer-tabs">
        {tabs.map((t) => (
          <div
            key={t.key}
            className={`viewer-tab${t.key === active ? " active" : ""}`}
            onClick={() => setTab(t.key)}
            title={t.label}
          >
            {t.icon}
            <span className="viewer-tab-label">{t.label}</span>
            <CloseOutlined
              className="viewer-tab-close"
              onClick={(e) => {
                e.stopPropagation();
                closeTab(t.key);
              }}
            />
          </div>
        ))}
      </div>

      {/* 文件/diff：仅当前激活时显示 */}
      {content && active === "content" && <ContentBody content={content} />}

      {/* 终端：常驻挂载（输出会持续累积），非激活时 CSS 隐藏，切回不丢历史 */}
      {terminalOpen && (
        <div className="viewer-tab-pane" style={{ display: active === "terminal" ? "flex" : "none" }}>
          <TerminalPanel />
        </div>
      )}

      {/* 浏览器：原生子 webview，非激活时必须卸载（CSS 隐藏不掉原生层），切回重新导航 */}
      {browserUrl !== null && active === "browser" && <BrowserView url={browserUrl} />}
    </div>
  );
}

function baseName(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1] || path;
}
