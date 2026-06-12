import { useEffect, useState } from "react";
import { Button, Input, Tag, Typography } from "antd";
import {
  CloseOutlined,
  DiffOutlined,
  FileTextOutlined,
  GlobalOutlined,
  ReloadOutlined,
} from "@ant-design/icons";
import { highlightCode, languageForPath } from "../../lib/highlight";
import { useViewerStore } from "../../stores/viewerStore";
import { DiffView } from "../explorer/DiffView";

function normalizeUrl(input: string): string {
  const trimmed = input.trim();
  if (!trimmed) return "";
  return /^https?:\/\//.test(trimmed) ? trimmed : `http://${trimmed}`;
}

function BrowserView({ url }: { url: string }) {
  const [draft, setDraft] = useState(url);
  const [current, setCurrent] = useState(url);
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    setDraft(url);
    setCurrent(url);
  }, [url]);

  const go = () => {
    const next = normalizeUrl(draft);
    if (!next) return;
    setDraft(next);
    setCurrent(next);
    localStorage.setItem("codeforge.browser.url", next);
  };

  return (
    <div className="browser-view">
      <div className="browser-bar">
        <Input
          size="small"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onPressEnter={go}
          placeholder="http://localhost:3000"
        />
        <Button size="small" type="primary" onClick={go}>
          打开
        </Button>
        <Button
          size="small"
          icon={<ReloadOutlined />}
          onClick={() => setReloadKey((k) => k + 1)}
        />
      </div>
      <iframe
        key={`${current}-${reloadKey}`}
        src={current}
        className="browser-frame"
        title="browser"
        sandbox="allow-scripts allow-same-origin allow-forms"
      />
      <div className="browser-hint">
        适合预览本地 dev server；部分外部网站（设置了 X-Frame-Options）会拒绝内嵌显示。
      </div>
    </div>
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
              <pre className="viewer-code">
                <code
                  dangerouslySetInnerHTML={{
                    __html:
                      content.content.length > 150_000
                        ? content.content.replace(/&/g, "&amp;").replace(/</g, "&lt;")
                        : highlightCode(content.content, languageForPath(content.path)),
                  }}
                />
              </pre>
            </>
          ) : (
            <DiffView path={content.path} diff={content.diff} />
          )}
        </div>
      )}
    </div>
  );
}
