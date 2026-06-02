import adapter from '@sveltejs/adapter-node';

/** @type {import('@sveltejs/kit').Config} */
export default {
  kit: {
    adapter: adapter({
      out:         'build',
      precompress: true,  // pre-compress assets with gzip + brotli
      envPrefix:   'ORBIT_',  // ORBIT_PORT, ORBIT_ORIGIN, etc.
    }),

    alias: {
      '$api':        'src/lib/api',
      '$stores':     'src/lib/stores',
      '$components': 'src/lib/components',
      '$utils':      'src/lib/utils',
    },

    csrf: {
      trustedOrigins: [],
    },
  },
};
