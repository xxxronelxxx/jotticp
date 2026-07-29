<script lang="ts">
  import { page } from '$app/stores';
  import { goto, afterNavigate } from '$app/navigation';
  import { navigating } from '$app/stores';
  import { auth, currentUser } from '$stores/auth';
  import OrbitIcon from '$lib/components/ui/OrbitIcon.svelte';
  import { theme, toggleTheme } from '$stores/theme';
  import { onMount } from 'svelte';
  import { api } from '$api/client';
  import type { Notification } from '$api/client';
  import { currentLang, setLanguage, t as tStore } from '$lib/i18n';
  import { get } from 'svelte/store';

  let contentKey = 0;
  afterNavigate(() => { contentKey++; });

  let navGroups = $derived.by(() => {
    const tr = get(tStore);
    return [
      {
        label: 'Overview',
        color: 'text-blue-500',
        items: [
          { href: '/dashboard',   label: tr('nav.dashboard'), icon: 'home'      },
          { href: '/websites',    label: tr('nav.websites'),  icon: 'globe'     },
          { href: '/servers',     label: tr('nav.servers'),   icon: 'server'    },
        ],
      },
      {
        label: 'Hosting',
        color: 'text-emerald-500',
        items: [
          { href: '/databases',   label: tr('nav.databases'), icon: 'database'  },
          { href: '/email',       label: tr('nav.email'),     icon: 'mail'      },
          { href: '/dns',         label: tr('nav.dns'),       icon: 'globe2'    },
          { href: '/ssl',         label: tr('nav.ssl'),       icon: 'lock'      },
          { href: '/filemanager', label: tr('nav.files'),     icon: 'folder'    },
        ],
      },
      {
        label: 'Tools',
        color: 'text-violet-500',
        items: [
          { href: '/backups',     label: tr('nav.backups'),   icon: 'cloud-up'  },
          { href: '/apps',        label: tr('nav.apps'),      icon: 'grid'      },
          { href: '/cron',        label: tr('nav.cron'),      icon: 'clock'     },
          { href: '/php',         label: tr('nav.php'),       icon: 'code'      },
          { href: '/cache',       label: tr('nav.cache'),     icon: 'zap'       },
          { href: '/webhooks',    label: 'Webhooks',          icon: 'webhook'   },
          { href: '/cloudflare',  label: 'Cloudflare',        icon: 'cloud'     },
          { href: '/migration',   label: 'Migration',         icon: 'import'    },
          { href: '/plugins',     label: 'Plugins',           icon: 'puzzle'    },
        ],
      },
      {
        label: 'Security',
        color: 'text-rose-500',
        items: [
          { href: '/security',    label: 'Security',          icon: 'shield'    },
          { href: '/firewall',    label: tr('nav.firewall'),  icon: 'shield-lock'},
        ],
      },
      {
        label: 'System',
        color: 'text-slate-400',
        items: [
          { href: '/logs',        label: 'Logs',              icon: 'doc-text'  },
          { href: '/audit-log',   label: 'Audit Log',         icon: 'audit'     },
          { href: '/dbmanager',   label: tr('nav.dbmanager'), icon: 'table'     },
          { href: '/users',       label: tr('nav.users'),     icon: 'users'     },
          { href: '/reseller',    label: tr('nav.reseller'),  icon: 'briefcase' },
          { href: '/settings',    label: tr('nav.settings'),  icon: 'settings'  },
        ],
      },
    ];
  });

  let sidebarOpen = false;
  let notifications: Notification[] = [];
  let showMobileSearch = false;
  let impersonationEmail: string | null = null;

  onMount(async () => {
    impersonationEmail = localStorage.getItem('orbit_impersonate_email');
    try {
      notifications = await api.notifications.list({ unread_only: true });
    } catch {}
  });

  let unreadCount = $derived(notifications.filter(n => !n.read_at).length);

  function initials(name: string): string {
    if (!name) return '?';
    return name
      .split(' ')
      .map(w => w[0])
      .join('')
      .toUpperCase()
      .slice(0, 2);
  }

  function initialsFromEmail(email: string): string {
    const parts = email.split('@')[0].split(/[._-]/);
    if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase();
    return email.slice(0, 2).toUpperCase();
  }

  async function handleLogout() {
    await auth.logout();
  }
</script>
