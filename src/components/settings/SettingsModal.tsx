import { useEffect, useState } from "react";
import { App, Input, Modal, Select, Tag, Typography } from "antd";
import { invoke } from "@tauri-apps/api/core";
import { MODEL_GROUPS, useChatStore } from "../../stores/chatStore";

interface KeyStatus {
  keychain: boolean;
  env: boolean;
}

const PROVIDERS = [
  { id: "ark", label: "火山方舟 Ark" },
  { id: "xiaomi-mimo", label: "小米 MiMo" },
  { id: "claude", label: "Anthropic Claude" },
];

interface Props {
  open: boolean;
  onClose: () => void;
}

export function SettingsModal({ open, onClose }: Props) {
  const { message } = App.useApp();
  const { model, setModel } = useChatStore();
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [status, setStatus] = useState<Record<string, KeyStatus>>({});
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!open) return;
    setDrafts({});
    for (const p of PROVIDERS) {
      void invoke<KeyStatus>("provider_key_status", { provider: p.id }).then((s) =>
        setStatus((prev) => ({ ...prev, [p.id]: s })),
      );
    }
  }, [open]);

  const save = async () => {
    setSaving(true);
    try {
      for (const [provider, key] of Object.entries(drafts)) {
        if (key.trim()) {
          await invoke("set_provider_key", { provider, key: key.trim() });
        }
      }
      message.success("设置已保存");
      onClose();
    } catch (e) {
      message.error(String(e));
    } finally {
      setSaving(false);
    }
  };

  const placeholderFor = (p: { id: string }) => {
    const s = status[p.id];
    if (s?.keychain) return "已保存在 macOS 钥匙串（输入新值可覆盖）";
    if (s?.env) return "已从环境变量读取（填写后以钥匙串为准）";
    return p.id === "claude" ? "sk-ant-..." : "填写 API key";
  };

  return (
    <Modal
      title="设置"
      open={open}
      onOk={save}
      onCancel={onClose}
      okText="保存"
      cancelText="取消"
      confirmLoading={saving}
    >
      <Typography.Paragraph strong style={{ marginTop: 16 }}>
        模型
      </Typography.Paragraph>
      <Select value={model} onChange={setModel} options={MODEL_GROUPS} style={{ width: "100%" }} />

      <Typography.Paragraph strong style={{ marginTop: 16 }}>
        API Keys
      </Typography.Paragraph>
      {PROVIDERS.map((p) => (
        <div key={p.id} style={{ marginBottom: 12 }}>
          <div style={{ marginBottom: 4, fontSize: 13 }}>
            {p.label}
            {status[p.id]?.keychain && <Tag color="green" style={{ marginLeft: 8 }}>钥匙串</Tag>}
            {status[p.id]?.env && <Tag style={{ marginLeft: 4 }}>env</Tag>}
          </div>
          <Input.Password
            value={drafts[p.id] ?? ""}
            onChange={(e) => setDrafts((d) => ({ ...d, [p.id]: e.target.value }))}
            placeholder={placeholderFor(p)}
          />
        </div>
      ))}
      <Typography.Paragraph type="secondary" style={{ fontSize: 12 }}>
        密钥仅存储在 macOS Keychain，不写入磁盘明文；留空表示不修改。
      </Typography.Paragraph>
    </Modal>
  );
}
