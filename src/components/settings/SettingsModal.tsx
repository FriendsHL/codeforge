import { useEffect, useState } from "react";
import { App, Input, Modal, Select, Typography } from "antd";
import { hasApiKey, setApiKey } from "../../lib/ipc";
import { MODELS, useChatStore } from "../../stores/chatStore";

interface Props {
  open: boolean;
  onClose: () => void;
}

export function SettingsModal({ open, onClose }: Props) {
  const { message } = App.useApp();
  const { model, setModel } = useChatStore();
  const [keyDraft, setKeyDraft] = useState("");
  const [keyConfigured, setKeyConfigured] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (open) {
      setKeyDraft("");
      void hasApiKey().then(setKeyConfigured);
    }
  }, [open]);

  const save = async () => {
    setSaving(true);
    try {
      if (keyDraft.trim()) {
        await setApiKey(keyDraft.trim());
      }
      message.success("设置已保存");
      onClose();
    } catch (e) {
      message.error(String(e));
    } finally {
      setSaving(false);
    }
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
        Anthropic API Key
      </Typography.Paragraph>
      <Input.Password
        value={keyDraft}
        onChange={(e) => setKeyDraft(e.target.value)}
        placeholder={keyConfigured ? "已保存在 macOS 钥匙串（输入新值可覆盖）" : "sk-ant-..."}
      />
      <Typography.Paragraph type="secondary" style={{ fontSize: 12, marginTop: 4 }}>
        密钥仅存储在 macOS Keychain，不会写入磁盘明文。
      </Typography.Paragraph>

      <Typography.Paragraph strong>模型</Typography.Paragraph>
      <Select
        value={model}
        onChange={setModel}
        options={MODELS}
        style={{ width: "100%" }}
      />
    </Modal>
  );
}
