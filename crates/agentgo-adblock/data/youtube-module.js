// AgentGo's YouTube module. It runs in an isolated world installed by the
// browser, so the page can see the DOM changes it makes but none of its
// variables, timers or functions, and it patches no page object.
//
// The network layer already removes ad data from YouTube's responses. This
// module handles what is left: an ad that arrives inside the stream, ad
// overlays, and the "ad blockers are not allowed" wall.
(() => {
  if (!/(^|\.)youtube(-nocookie)?\.com$/.test(location.hostname)) return;
  if (window.__agentgoYoutube) return;
  window.__agentgoYoutube = 1;

  const HIDE = [
    'ytd-ad-slot-renderer', 'ytd-in-feed-ad-layout-renderer', 'ytd-display-ad-renderer',
    'ytd-promoted-sparkles-web-renderer', 'ytd-promoted-video-renderer', 'ytd-banner-promo-renderer',
    'ytd-statement-banner-renderer', 'ytd-companion-slot-renderer', 'ytd-player-legacy-desktop-watch-ads-renderer',
    '#masthead-ad', '#player-ads', '.ytp-ad-overlay-container', '.ytp-ad-image-overlay',
    '.ytp-suggested-action-badge', 'ytm-promoted-sparkles-web-renderer', '.ad-showing .ytp-ad-player-overlay-layout',
  ];
  const SKIP = [
    '.ytp-skip-ad-button', '.ytp-ad-skip-button', '.ytp-ad-skip-button-modern',
    '.ytp-ad-skip-button-container button', 'button.ytp-ad-skip-button-modern',
  ];
  const CLOSE = ['.ytp-ad-overlay-close-button', '.ytp-ad-overlay-close-container'];
  const WALL = 'ytd-enforcement-message-view-model, tp-yt-paper-dialog:has(ytd-enforcement-message-view-model)';

  const put = () => {
    const parent = document.head || document.documentElement;
    if (!parent) return false;
    const style = document.createElement('style');
    style.textContent = HIDE.join(',') + '{display:none!important}';
    parent.appendChild(style);
    return true;
  };
  if (!put()) {
    const wait = new MutationObserver(() => { if (put()) wait.disconnect(); });
    wait.observe(document, { childList: true, subtree: true });
  }

  let held = null;
  const q = (selector, root = document) => root.querySelector(selector);

  const tick = () => {
    const player = q('#movie_player') || q('.html5-video-player');
    const video = q('video.html5-main-video') || q('video');
    if (player && video) {
      const ad = player.classList.contains('ad-showing') || player.classList.contains('ad-interrupting');
      if (ad) {
        for (const selector of SKIP) q(selector)?.click();
        for (const selector of CLOSE) q(selector)?.click();
        if (!held) held = { muted: video.muted, rate: video.playbackRate };
        video.muted = true;
        video.playbackRate = 16;
        // An ad is a short clip of its own; when the whole programme is in the
        // stream (a long duration) only the speed-up applies.
        const duration = video.duration;
        if (Number.isFinite(duration) && duration > 0 && duration < 180 && video.currentTime < duration - 0.2) {
          video.currentTime = duration - 0.1;
        }
      } else if (held) {
        video.muted = held.muted;
        video.playbackRate = held.rate;
        held = null;
      }
    }
    const wall = q(WALL);
    if (wall) {
      (wall.querySelector('#dismiss-button button, [aria-label="Close" i], button[aria-label*="close" i]'))?.click();
      (wall.closest('tp-yt-paper-dialog') || wall).remove();
      document.querySelectorAll('tp-yt-iron-overlay-backdrop').forEach((el) => el.remove());
      document.documentElement.style.overflow = '';
      if (document.body) document.body.style.overflow = '';
      if (video && video.paused && video.readyState > 2) video.play().catch(() => {});
    }
  };

  setInterval(tick, 250);
  new MutationObserver(tick).observe(document, { subtree: true, attributes: true, attributeFilter: ['class'], childList: true });
})();
