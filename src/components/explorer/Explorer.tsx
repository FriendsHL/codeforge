import { useCallback, useEffect, useState } from "react";
import { App, Modal, Tree, Typography } from "antd";
import type { TreeDataNode } from "antd";
import { readDirTree, readFilePreview } from "../../lib/ipc";
import { useWorkspaceStore } from "../../stores/workspaceStore";

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
  const [treeData, setTreeData] = useState<TreeDataNode[]>([]);
  const [preview, setPreview] = useState<{ path: string; content: string; truncated: boolean } | null>(null);

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

  return (
    <div className="explorer">
      <Tree.DirectoryTree
        treeData={treeData}
        loadData={loadChildren}
        onSelect={(_, info) => {
          if (info.node.isLeaf) void openFile(String(info.node.key));
        }}
      />
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
        <pre className="file-preview">{preview?.content}</pre>
      </Modal>
    </div>
  );
}
