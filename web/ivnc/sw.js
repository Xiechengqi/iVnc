// Authentication and live desktops require a network connection. Never replay
// a protected document or a login response from an earlier browser session.
self.addEventListener('install', e => e.waitUntil(self.skipWaiting()));

self.addEventListener('activate', e => {
  e.waitUntil(caches.keys().then(keys =>
    Promise.all(keys.filter(k => k === 'ivnc-v1' || k.startsWith('ivnc-shell-')).map(k => caches.delete(k)))
  ).then(() => self.clients.claim()));
});
// No fetch interception: native navigation also preserves HTTP auth semantics
// for installations still using Basic's explicit compatibility path.
