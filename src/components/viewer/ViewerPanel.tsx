import { Button, Tag, Typography } from "antd";
import { CloseOutlined, DiffOutlined, FileTextOutlined } from "@ant-design/icons";
import { highlightCode, languageForPath } from "../../lib/highlight";
import { useViewerStore } from "../../stores/viewerStore";
import { DiffView } from "../explorer/DiffView";

/** 文件/diff 查看器：位于主交互区与右栏之间 */
export function ViewerPanel() {
  const { content, close } = useViewerStore();
  if (!content) return null;

  return (
    <div className="viewer-panel">
      <div className="viewer-header">
        {content.type === "diff" ? <DiffOutlined /> : <FileTextOutlined />}
        <span className="viewer-path" title={content.path}>
          {content.path}
        </span>
        {content.type === "diff" && <Tag color="orange">diff</Tag>}
        <Button type="text" size="small" icon={<CloseOutlined />} onClick={close} />
      </div>
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
    </div>
  );
}
