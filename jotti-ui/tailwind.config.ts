import type { Config } from 'tailwindcss';

export default {
  darkMode: 'class',
  content: [
    './src/**/*.{html,js,svelte,ts}',
  ],
  theme: {
    extend: {
      colors: {
        // Map CSS variable tokens to Tailwind color names
        background:  'var(--background)',
        foreground:  'var(--foreground)',
        card: {
          DEFAULT:    'var(--card)',
          foreground: 'var(--card-foreground)',
        },
        primary: {
          DEFAULT:    'var(--primary)',
          foreground: 'var(--primary-foreground)',
        },
        secondary: {
          DEFAULT:    'var(--secondary)',
          foreground: 'var(--secondary-foreground)',
        },
        muted: {
          DEFAULT:    'var(--muted)',
          foreground: 'var(--muted-foreground)',
        },
        destructive: {
          DEFAULT:    'var(--destructive)',
          foreground: 'var(--destructive-foreground)',
        },
        border:  'var(--border)',
        input:   'var(--input)',
        ring:    'var(--ring)',
        // Design system palette tokens
        page:    'var(--bg-page)',
        surface: 'var(--bg-surface)',
        hover:   'var(--bg-hover)',
        accent: {
          DEFAULT: 'var(--accent)',
          hover:   'var(--accent-hover)',
          light:   'var(--accent-light)',
        },
        success: 'var(--success)',
        warning: 'var(--warning)',
        error:   'var(--error)',
        info:    'var(--info)',
      },
      borderRadius: {
        DEFAULT: 'var(--radius)',
        sm:  'calc(var(--radius) - 2px)',
        md:  'var(--radius)',
        lg:  'calc(var(--radius) + 2px)',
        xl:  'calc(var(--radius) + 4px)',
        '2xl': 'calc(var(--radius) + 8px)',
      },
      fontFamily: {
        sans: ['-apple-system', 'BlinkMacSystemFont', '"Segoe UI"', 'Roboto', '"Helvetica Neue"', 'Arial', 'sans-serif'],
        mono: ['"SFMono-Regular"', 'Consolas', '"Liberation Mono"', 'Menlo', 'monospace'],
      },
    },
  },
  plugins: [],
  safelist: [
    'text-blue-500', 'text-emerald-500', 'text-violet-500', 'text-rose-500', 'text-slate-400',
  ],
} satisfies Config;
