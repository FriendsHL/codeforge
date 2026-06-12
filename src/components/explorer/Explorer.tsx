import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { App, Empty, Tree } from "antd";
import type { TreeDataNode } from "antd";
import { readDirTree } from "../../lib/ipc";
import { useGitStore } from "../../stores/gitStore";
import { useViewerStore } from "../../stores/viewerStore";
import { useWorkspaceStore } from "../../stores/workspaceStore";

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

export function Explorer({ view }: { view: "files" | "changes" }) {
  const { message } = App.useApp();
  const { version } = useWorkspaceStore();
  const changes = useGitStore((s) => s.changes);
  const { openFile, openDiff } = useViewerStore();
  const [treeData, setTreeData] = useState<TreeDataNode[]>([]);
  // 已加载过的目录（含根 "."）。fs 变化时全部重新拉取，新增/删除的文件就能出现
  const loadedDirs = useRef<Set<string>>(new Set(["."]));

  const rebuild = useCallback(async () => {
    const keys = [...loadedDirs.current];
    const childrenMap = new Map<string, TreeDataNode[]>();
    await Promise.all(
      keys.map(async (key) => {
        try {
          childrenMap.set(key, toTreeNodes(await readDirTree(key)));
        } catch {
          loadedDirs.current.delete(key); // 目录已被删除
        }
      }),
    );
    const build = (nodes: TreeDataNode[]): TreeDataNode[] =>
      nodes.map((node) => {
        const kids = childrenMap.get(String(node.key));
        return kids ? { ...node, children: build(kids) } : node;
      });
    setTreeData(build(childrenMap.get(".") ?? []));
  }, []);

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

  // 切换工作区：重置已加载集合并拉根目录
  useEffect(() => {
    loadedDirs.current = new Set(["."]);
    void rebuild().catch((e) => message.error(String(e)));
  }, [version, message, rebuild]);

  // 文件变化（agent 新建/删除文件、外部编辑）→ 刷新所有已加载目录
  useEffect(() => {
    const unlisten = listen("workspace-fs-changed", () => void rebuild());
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [rebuild]);

  const loadChildren = useCallback(
    async (node: TreeDataNode) => {
      loadedDirs.current.add(String(node.key));
      await rebuild().catch((e) => message.error(String(e)));
    },
    [message, rebuild],
  );

  const openChange = useCallback(
    (path: string, status: string) => {
      // 未跟踪/新增文件没有 diff，直接看内容
      const action = status === "?" || status === "A" ? openFile(path) : openDiff(path);
      void action.catch((e) => message.error(String(e)));
    },
    [message, openFile, openDiff],
  );

  return (
    <div className="explorer">
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
              if (info.node.isLeaf) {
                void openFile(String(info.node.key)).catch((e) => message.error(String(e)));
              }
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
              onClick={() => openChange(c.path, c.status)}
              title={c.path}
            >
              <StatusBadge status={c.status} />
              <span className="changes-path">{c.path}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
