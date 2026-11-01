type Listener = (kind: string, id: string | null) => void;

const listeners = new Set<Listener>();
let source: EventSource | null = null;

function connect() {
  if (source) return;
  source = new EventSource('/api/events');
  for (const kind of ['nodes', 'stats', 'users']) {
    source.addEventListener(kind, (event) => {
      let id: string | null = null;
      try {
        id = JSON.parse((event as MessageEvent).data).id ?? null;
      } catch {
        id = null;
      }
      for (const listener of listeners) listener(kind, id);
    });
  }
}

export function onEvent(listener: Listener): () => void {
  connect();
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function closeEvents() {
  source?.close();
  source = null;
}
