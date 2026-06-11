import { useCallback, useEffect, useMemo, useState } from "react";
import { App, Badge, Empty, Modal, Segmented, Tree, Typography } from "antd";
import type { TreeDataNode } from "antd";
import { gitFileDiff, readDirTree, readFilePreview } from "../../lib/ipc";
import { highlightCode, languageForPath } from "../../lib/highlight";
import { useGitStore } from "../../stores/gitStore";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { DiffView } from "./DiffView";

const STATUS_COLORS: Record<string, string> = {
  M: "#d46b08",
  A: "#389e0d",
  D: "#cf1322",
  R: "#1677ff",
  "?": "#8c8c8c",
};

function StatusBadge({ status }: { status?: string }) {
  if (!status) return null;
  return (
    <span className="git-badge" style={{ color: STATUS_COLORS[status] ?? "#8c8c8c" }}>
      {status}
    </span>
  );
}

function toTreeNodes(nodes: { path: string; name: string; isDir: boolean }[]): TreeDataNode[] {
  return nodes.map((n) => ({
    key: n.path,
    title: n.name,
    isLeaf: !n.isDir,
  }));
}

/** 把懒加载的子节点挂到树上对应位置 */
function attachChildren(
  tree: TreeDataNode[],
  key: string,
  children: TreeDataNode[],
): TreeDataNode[] {
  return tree.map((node) => {
    if (node.key === key) return { ...node, children };
    if (node.children) {
      return { ...node, children: attachChildren(node.children, key, children) };
    }
    return node;
  });
}

export function Explorer() {
  const { message } = App.useApp();
  const { version } = useWorkspaceStore();
  const changes = useGitStore((s) => s.changes);
  const [view, setView] = useState<"files" | "changes">("files");
  const [treeData, setTreeData] = useState<TreeDataNode[]>([]);
  const [preview, setPreview] = useState<{ path: string; content: string; truncated: boolean } | null>(null);
  const [diff, setDiff] = useState<{ path: string; text: string } | null>(null);

  // 文件状态表 + 含改动的目录前缀集合（目录上显示圆点）
  const { fileStatus, dirtyDirs } = useMemo(() => {
    const fileStatus = new Map<string, string>();
    const dirtyDirs = new Set<string>();
    for (const c of changes) {
      fileStatus.set(c.path, c.status);
      const parts = c.path.split("/");
      for (let i = 1; i < parts.length; i++) {
        dirtyDirs.add(parts.slice(0, i).join("/"));
      }
    }
    return { fileStatus, dirtyDirs };
  }, [changes]);

  useEffect(() => {
    readDirTree(".")
      .then((nodes) => setTreeData(toTreeNodes(nodes)))
      .catch((e) => message.error(String(e)));
  }, [version, message]);

  const loadChildren = useCallback(
    async (node: TreeDataNode) => {
      try {
        const children = await readDirTree(String(node.key));
        setTreeData((tree) => attachChildren(tree, String(node.key), toTreeNodes(children)));
      } catch (e) {
        message.error(String(e));
      }
    },
    [message],
  );

  const openFile = useCallback(
    async (path: string) => {
      try {
        const file = await readFilePreview(path);
        setPreview({ path, ...file });
      } catch (e) {
        message.error(String(e));
      }
    },
    [message],
  );

  const openChange = useCallback(
    async (path: string, status: string) => {
      // 未跟踪/新增文件没有 diff，直接看内容
      if (status === "?" || status === "A") {
        void openFile(path);
        return;
      }
      try {
        const text = await gitFileDiff(path);
        setDiff({ path, text });
      } catch (e) {
        message.error(String(e));
      }
    },
    [message, openFile],
  );

  return (
    <div className="explorer">
      <div className="explorer-switch">
        <Segmented
          block
          size="small"
          value={view}
          onChange={(v) => setView(v as "files" | "changes")}
          options={[
            { label: "文件", value: "files" },
            {
              label: (
                <span>
                  改动
                  <Badge
                    count={changes.length}
                    size="small"
                    color="#d46b08"
                    style={{ marginLeft: 4 }}
                  />
                </span>
              ),
              value: "changes",
            },
          ]}
        />
      </div>

      {view === "files" ? (
        <div className="explorer-tree">
          <Tree.DirectoryTree
            treeData={treeData}
            loadData={loadChildren}
            titleRender={(node) => {
              const key = String(node.key);
              return (
                <span>
                  {String(node.title)}
                  {node.isLeaf ? (
                    <StatusBadge status={fileStatus.get(key)} />
                  ) : dirtyDirs.has(key) ? (
                    <span className="git-dot" />
                  ) : null}
                </span>
              );
            }}
            onSelect={(_, info) => {
              if (info.node.isLeaf) void openFile(String(info.node.key));
            }}
          />
        </div>
      ) : (
        <div className="changes-panel">
          {changes.length === 0 && (
            <Empty
              image={Empty.PRESENTED_IMAGE_SIMPLE}
              description="工作区干净，无改动"
              style={{ marginTop: 32 }}
            />
          )}
          {changes.map((c) => (
            <div
              key={c.path}
              className="changes-item"
              onClick={() => void openChange(c.path, c.status)}
              title={c.path}
            >
              <StatusBadge status={c.status} />
              <span className="changes-path">{c.path}</span>
            </div>
          ))}
        </div>
      )}

      <Modal
        title={preview?.path}
        open={preview !== null}
        onCancel={() => setPreview(null)}
        footer={null}
        width="72vw"
      >
        {preview?.truncated && (
          <Typography.Text type="warning">文件过大，仅显示前 200KB</Typography.Text>
        )}
        {preview && (
          <pre className="file-preview">
            <code
              dangerouslySetInnerHTML={{
                __html:
                  preview.content.length > 150_000
                    ? preview.content.replace(/&/g, "&amp;").replace(/</g, "&lt;")
                    : highlightCode(preview.content, languageForPath(preview.path)),
              }}
            />
          </pre>
        )}
      </Modal>

      <Modal
        title={`diff: ${diff?.path ?? ""}`}
        open={diff !== null}
        onCancel={() => setDiff(null)}
        footer={null}
        width="72vw"
      >
        {diff && <DiffView path={diff.path} diff={diff.text} />}
      </Modal>
    </div>
  );
}
