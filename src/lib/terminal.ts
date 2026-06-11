// 终端输出的轻量 pubsub：chatStore 写入，TerminalPanel 订阅。
// 面板未挂载时先缓冲，挂载后一次性回放。

type Listener = (chunk: string) => void;

let listeners: Listener[] = [];
let buffer: string[] = [];
const MAX_BUFFER = 2000;

export function termWrite(chunk: string) {
  if (listeners.length === 0) {
    buffer.push(chunk);
    if (buffer.length > MAX_BUFFER) buffer = buffer.slice(-MAX_BUFFER);
  } else {
    listeners.forEach((l) => l(chunk));
  }
}

export function termSubscribe(listener: Listener): () => void {
  buffer.forEach(listener);
  buffer = [];
  listeners.push(listener);
  return () => {
    listeners = listeners.filter((l) => l !== listener);
  };
}
