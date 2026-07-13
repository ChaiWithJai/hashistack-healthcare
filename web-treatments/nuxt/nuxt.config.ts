// https://nuxt.com/docs/api/configuration/nuxt-config
export default defineNuxtConfig({
  compatibilityDate: '2025-07-15',
  devtools: { enabled: false },
  css: ['~/assets/app.css'],
  nitro: {
    devProxy: {
      '/api': { target: 'http://127.0.0.1:3000/api', changeOrigin: true },
      '/health': { target: 'http://127.0.0.1:3000/health', changeOrigin: true }
    }
  },
  runtimeConfig: {
    public: {
      // Dev-only bearer token. Never hardcode this outside a gitignored
      // .env — copy .env.example to .env and set NUXT_PUBLIC_DEV_TOKEN.
      devToken: process.env.NUXT_PUBLIC_DEV_TOKEN ?? ''
    }
  }
})
