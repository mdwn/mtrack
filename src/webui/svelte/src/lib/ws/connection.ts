// Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//

type MessageHandler = (data: unknown) => void;

const RECONNECT_MS = 2000;

let ws: WebSocket | null = null;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
const handlers = new Map<string, MessageHandler[]>();
let onStatusChange: ((connected: boolean) => void) | null = null;

function url(): string {
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  const base = `${protocol}//${location.host}/ws`;
  // Support an optional wsId query parameter for test isolation.
  // Tests navigate to /#/?wsId=xxx which makes the WebSocket connection
  // identifiable by the mock server for targeted message routing.
  const params = new URLSearchParams(location.search);
  const wsId = params.get("wsId");
  return wsId ? `${base}?wsId=${encodeURIComponent(wsId)}` : base;
}

export function onConnectionStatus(cb: (connected: boolean) => void): void {
  onStatusChange = cb;
}

export function on(type: string, cb: MessageHandler): void {
  const list = handlers.get(type) ?? [];
  list.push(cb);
  handlers.set(type, list);
}

export function connect(): void {
  if (reconnectTimer !== null) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }

  if (ws) {
    ws.onclose = null;
    ws.onerror = null;
    ws.close();
    ws = null;
  }

  ws = new WebSocket(url());

  ws.onopen = () => {
    onStatusChange?.(true);
  };

  ws.onclose = () => {
    onStatusChange?.(false);
    reconnectTimer = setTimeout(connect, RECONNECT_MS);
  };

  ws.onerror = () => {
    ws?.close();
  };

  ws.onmessage = (event) => {
    try {
      const msg = JSON.parse(event.data as string);
      const type = msg.type as string;
      const cbs = handlers.get(type);
      if (cbs) {
        for (const cb of cbs) cb(msg);
      }
    } catch {
      // ignore malformed messages
    }
  };
}
