export type Theme = 'system' | 'light' | 'dark';

export const theme = $state<{ value: Theme }>({ value: 'system' });

export function initTheme() {
  const saved = localStorage.getItem('fiend-theme');
  theme.value = saved === 'light' || saved === 'dark' ? saved : 'system';
  apply();
}

export function setTheme(value: Theme) {
  theme.value = value;
  localStorage.setItem('fiend-theme', value);
  apply();
}

function apply() {
  if (theme.value === 'system') document.documentElement.removeAttribute('data-theme');
  else document.documentElement.setAttribute('data-theme', theme.value);
}
