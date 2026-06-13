import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { termSubscribe } from "../../lib/terminal";

export function TerminalPanel() {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!containerRef.current) return;

    const term = new Terminal({
      fontSize: 12,
      fontFamily: '"SF Mono", Menlo, Consolas, monospace',
      theme: { background: "#1e1e1e" },
      convertEol: true,
      disableStdin: true,
      scrollback: 5000,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(containerRef.current);
    fit.fit();

    const unsubscribe = termSubscribe((chunk) => {
      term.write(chunk);
    });
    const observer = new ResizeObserver(() => fit.fit());
    observer.observe(containerRef.current);

    return () => {
      unsubscribe();
      observer.disconnect();
      term.dispose();
    };
  }, []);

  return (
    <div className="terminal-panel">
      <div className="terminal-hint">终端 · agent 命令回显（只读）</div>
      <div className="terminal-body" ref={containerRef} />
    </div>
  );
}
