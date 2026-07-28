import { writable } from 'svelte/store';
import { browser } from '$app/environment';

const stored = browser ? localStorage.getItem('theme') : null;
export const theme = writable<'light' | 'dark'>(stored === 'dark' ? 'dark' : 'light');

theme.subscribe(value => {
  if (browser) {
    localStorage.setItem('theme', value);
    document.documentElement.classList.toggle('dark', value === 'dark');
  }
});

export function toggleTheme() {
  theme.update(t => t === 'light' ? 'dark' : 'light');
}
