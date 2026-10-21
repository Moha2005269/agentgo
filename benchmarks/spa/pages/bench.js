// Shared harness for the SPA task pages: seed/run from the first URL (kept for
// client-side routes), a seeded random source, a fake backend with latency,
// and the page's own grader (end) and diagnostics (event).
(function () {
  var q = new URLSearchParams(location.search);
  var seed = parseInt(q.get('seed') || sessionStorage.getItem('bench.seed') || '0', 10);
  var run = q.get('run') || sessionStorage.getItem('bench.run') || '';
  sessionStorage.setItem('bench.seed', seed);
  sessionStorage.setItem('bench.run', run);
  var t0 = Date.now();
  function post(path, obj) {
    var body = JSON.stringify(Object.assign({run: run, task: BENCH_CFG.task, seed: seed, t_ms: Date.now() - t0}, obj));
    try { fetch(path, {method: 'POST', body: body, keepalive: true}); } catch (e) {}
  }
  var s = seed >>> 0;
  window.BENCH = {
    seed: seed, run: run, target: BENCH_CFG.target, base: BENCH_CFG.base, done: false,
    rand: function () { // mulberry32
      s = (s + 0x6D2B79F5) >>> 0; var t = s;
      t = Math.imul(t ^ (t >>> 15), t | 1); t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    },
    end: function (reward, reason) {
      if (BENCH.done) return; BENCH.done = true;
      post('/report', {reward: reward, raw_reward: reward, reason: reason});
    },
    event: function (kind, detail) { post('/event', {kind: kind, detail: detail || ''}); },
    api: function (ms, data, status) {
      return fetch('/api/echo?ms=' + Math.round(ms) + '&status=' + (status || 200),
                   {method: 'POST', body: JSON.stringify(data || {})})
        .then(function (r) { if (!r.ok) throw new Error('HTTP ' + r.status); return r.json(); });
    },
    // Query string to keep on client-side routes.
    qs: '?seed=' + seed + '&run=' + encodeURIComponent(run)
  };
})();
