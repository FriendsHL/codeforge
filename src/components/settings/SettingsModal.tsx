import { useEffect, useState } from "react";
import { App, Input, Modal, Select, Typography } from "antd";
import { hasApiKey, setApiKey } from "../../lib/ipc";
import { MODEL_GROUPS, useChatStore } from "../../stores/chatStore";

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
        模型
      </Typography.Paragraph>
      <Select
        value={model}
        onChange={setModel}
        options={MODEL_GROUPS}
        style={{ width: "100%" }}
      />
      <Typography.Paragraph type="secondary" style={{ fontSize: 12, marginTop: 4 }}>
        火山方舟 / 小米的 API key 从启动终端的环境变量读取（ARK_API_KEY /
        XIAOMI_MIMO_API_KEY），无需在此配置。
      </Typography.Paragraph>

      <Typography.Paragraph strong>
        Anthropic API Key（仅选 Claude 模型时需要）
      </Typography.Paragraph>
      <Input.Password
        value={keyDraft}
        onChange={(e) => setKeyDraft(e.target.value)}
        placeholder={keyConfigured ? "已保存在 macOS 钥匙串（输入新值可覆盖）" : "sk-ant-..."}
      />
      <Typography.Paragraph type="secondary" style={{ fontSize: 12, marginTop: 4 }}>
        密钥仅存储在 macOS Keychain，不会写入磁盘明文。
      </Typography.Paragraph>
    </Modal>
  );
}
