import { useEffect, useRef } from "react";
import { wsUrl, type Status } from "./api";

export type ProgressEvent = { job_id: number; item_id: number; status: Status; progress: number };

/** Assina o WebSocket de progresso, com reconexão exponencial. */
export function useProgress(onEvent: (e: ProgressEvent) => void) {
  const cb = useRef(onEvent);
  cb.current = onEvent;

  useEffect(() => {
    let ws: WebSocket | null = null;
    let retry = 0;
    let timer: ReturnType<typeof setTimeout>;
    let closed = false;

    const open = () => {
      ws = new WebSocket(wsUrl());
      ws.onopen = () => { retry = 0; };
      ws.onmessage = (m) => {
        try { cb.current(JSON.parse(m.data)); } catch { /* mensagem inválida */ }
      };
      ws.onclose = () => {
        if (closed) return;
        timer = setTimeout(open, Math.min(1000 * 2 ** retry++, 15000));
      };
    };
    open();
    return () => { closed = true; clearTimeout(timer); ws?.close(); };
  }, []);
}
