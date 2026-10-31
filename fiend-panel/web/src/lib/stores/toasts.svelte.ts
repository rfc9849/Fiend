type Toast = { id: number; text: string; error: boolean };

let next = 1;
export const toasts = $state<Toast[]>([]);

export function toast(text: string, error = false) {
  const id = next++;
  toasts.push({ id, text, error });
  setTimeout(() => {
    const index = toasts.findIndex((item) => item.id === id);
    if (index >= 0) toasts.splice(index, 1);
  }, error ? 7000 : 3500);
}
