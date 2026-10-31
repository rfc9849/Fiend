export function bytes(value: number | null | undefined): string {
  if (value === null || value === undefined) return '—';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let size = Math.max(0, value);
  let unit = 0;
  while (size >= 1000 && unit < units.length - 1) {
    size /= 1000;
    unit += 1;
  }
  return `${size >= 100 || unit === 0 ? Math.round(size) : size.toFixed(1)} ${units[unit]}`;
}

export function rate(value: number): string {
  if (value <= 0) return '0 bit/s';
  const bits = value * 8;
  const units = ['bit/s', 'kbit/s', 'Mbit/s', 'Gbit/s'];
  let size = bits;
  let unit = 0;
  while (size >= 1000 && unit < units.length - 1) {
    size /= 1000;
    unit += 1;
  }
  return `${size >= 100 ? Math.round(size) : size.toFixed(1)} ${units[unit]}`;
}

export function splitUnit(text: string): [string, string] {
  const index = text.lastIndexOf(' ');
  return index < 0 ? [text, ''] : [text.slice(0, index), text.slice(index + 1)];
}

export function ago(seconds: number | null | undefined): string {
  if (!seconds) return 'never';
  const delta = Math.max(0, Math.floor(Date.now() / 1000) - seconds);
  if (delta < 10) return 'now';
  if (delta < 60) return `${delta} s ago`;
  if (delta < 3600) return `${Math.floor(delta / 60)} min ago`;
  if (delta < 86400) return `${Math.floor(delta / 3600)} h ago`;
  return `${Math.floor(delta / 86400)} d ago`;
}

export function date(seconds: number | null | undefined): string {
  if (!seconds) return '—';
  return new Date(seconds * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}

export function day(seconds: number | null | undefined): string {
  if (!seconds) return '—';
  return new Date(seconds * 1000).toLocaleDateString(undefined, { dateStyle: 'medium' });
}

export function percent(used: number, total: number | null | undefined): number {
  if (!total || total <= 0) return 0;
  return Math.min(100, Math.round((used / total) * 100));
}

export function parseBytes(text: string): number | null {
  const match = text.trim().toUpperCase().match(/^(\d+(?:\.\d+)?)\s*(B|KB|MB|GB|TB)?$/);
  if (!match) return null;
  const factor = { B: 1, KB: 1e3, MB: 1e6, GB: 1e9, TB: 1e12 }[(match[2] ?? 'GB') as 'B' | 'KB' | 'MB' | 'GB' | 'TB'];
  return Math.round(Number(match[1]) * factor);
}

export function stateLabel(state: string): string {
  return (
    {
      online: 'Online',
      offline: 'Offline',
      error: 'Error',
      setup: 'Needs a disguise',
      unknown: 'Waiting',
      active: 'Active',
      disabled: 'Disabled',
      expired: 'Expired',
      limited: 'Limit reached',
      on_hold: 'Waiting for first connection'
    } as Record<string, string>
  )[state] ?? state;
}
