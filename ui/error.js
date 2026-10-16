// Shown in place of a page that failed to load. It has no bridge access: it
// only reads its own address.
const query = new URLSearchParams(location.search);
const theme = query.get('theme');
document.documentElement.dataset.themed = '1';
document.documentElement.dataset.theme = theme === 'light' || (theme === 'system' && matchMedia('(prefers-color-scheme: light)').matches) ? 'light' : 'dark';
const url = query.get('url') || '';
const error = (query.get('error') || '').replace(/[^A-Z_0-9]/g, '').slice(0, 60);
const reasons = [
  [/NAME_NOT_RESOLVED/, 'No site was found at this address. Check the spelling.'],
  [/INTERNET_DISCONNECTED|NETWORK_CHANGED|ADDRESS_UNREACHABLE/, 'This computer is not connected to the internet.'],
  [/CONNECTION_REFUSED/, 'The site refused the connection.'],
  [/TIMED_OUT/, 'The site took too long to answer.'],
  [/CONNECTION_(RESET|CLOSED|ABORTED)|EMPTY_RESPONSE/, 'The connection to the site was interrupted.'],
  [/CERT_|SSL_/, 'The site could not prove who it is, so AgentGo did not open it.'],
  [/FILE_NOT_FOUND/, 'This file does not exist.'],
  [/BLOCKED|UNSAFE/, 'AgentGo blocked this address.'],
];
document.getElementById('reason').textContent = (reasons.find(([pattern]) => pattern.test(error)) || [null, 'The site could not be reached.'])[1];
document.getElementById('host').textContent = url;
document.getElementById('code').textContent = error;
if (/^(https?|file):/i.test(url)) {
  const retry = document.getElementById('retry');
  retry.href = url;
  retry.hidden = false;
}

// No connection at all: say so plainly, and offer a game until it is back.
const offline = /INTERNET_DISCONNECTED|NETWORK_CHANGED|NETWORK_IO_SUSPENDED|ADDRESS_UNREACHABLE/.test(error) || !navigator.onLine;
if (offline) {
  document.body.classList.add('offline');
  document.querySelector('.kicker').textContent = 'No internet connection';
  document.getElementById('headline').innerHTML = "You're <em>offline.</em>";
  document.getElementById('reason').textContent = 'The connection dropped. AgentGo will notice the moment it is back. Meanwhile, the wire is yours.';
  document.getElementById('status').hidden = false;
  document.getElementById('play').hidden = false;
  const game = document.createElement('script');
  game.src = 'offline.js';
  game.onload = () => document.getElementById('game').focus();
  document.body.appendChild(game);
  // Back online: confirmed by reaching the site (the browser's own "online"
  // flag only says a network exists). Return at once if no game is going,
  // otherwise let the player choose.
  let announced = false;
  const probe = async () => {
    if (announced || !navigator.onLine || !/^https?:/i.test(url)) return;
    try { await fetch(url, { mode: 'no-cors', cache: 'no-store' }); } catch (_) { return; }
    announced = true;
    const playing = window.wireman && ['play', 'pause', 'dying', 'clear'].includes(window.wireman.state().state);
    if (!playing) { location.href = url; return; }
    document.getElementById('backOnline').hidden = false;
    document.getElementById('status').hidden = true;
    document.getElementById('backLink').href = url;
  };
  addEventListener('online', probe);
  setTimeout(() => setInterval(probe, 3000), 3000);
}
