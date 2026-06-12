/** 面板间的拖拽分隔条。拖动期间禁用 iframe 鼠标事件（否则 iframe 会吞掉 mousemove） */
export function ResizeHandle({ onDelta }: { onDelta: (dx: number) => void }) {
  const onMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    let lastX = e.clientX;
    document.body.classList.add("resizing");

    const onMove = (ev: MouseEvent) => {
      onDelta(ev.clientX - lastX);
      lastX = ev.clientX;
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      document.body.classList.remove("resizing");
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  return <div className="resize-handle" onMouseDown={onMouseDown} />;
}
