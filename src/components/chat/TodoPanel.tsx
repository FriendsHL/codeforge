import { useState } from "react";
import {
  CaretDownOutlined,
  CaretRightOutlined,
  CheckCircleFilled,
  ClockCircleOutlined,
  LoadingOutlined,
} from "@ant-design/icons";
import { useTodoStore } from "../../stores/todoStore";

/** agent 任务清单面板：固定在输入框上方，随 todo_write 实时更新 */
export function TodoPanel() {
  const todos = useTodoStore((s) => s.todos);
  const [open, setOpen] = useState(true);
  if (todos.length === 0) return null;

  const done = todos.filter((t) => t.status === "completed").length;

  return (
    <div className="todo-panel">
      <div className="todo-header" onClick={() => setOpen((o) => !o)}>
        {open ? <CaretDownOutlined /> : <CaretRightOutlined />}
        <span>任务清单</span>
        <span className="todo-progress">
          {done}/{todos.length}
        </span>
      </div>
      {open && (
        <div className="todo-body">
          {todos.map((t, i) => (
            <div key={i} className={`todo-item todo-${t.status}`}>
              {t.status === "completed" ? (
                <CheckCircleFilled style={{ color: "#389e0d" }} />
              ) : t.status === "in_progress" ? (
                <LoadingOutlined style={{ color: "#d46b08" }} />
              ) : (
                <ClockCircleOutlined style={{ color: "#bbb" }} />
              )}
              <span>{t.content}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
