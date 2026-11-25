// Monocard landing page. No dependency. The only requests are for the views
// of the first picture, when one is asked for.
(function () {
  'use strict';
  var doc = document.documentElement;
  doc.classList.add('js');

  // The download button is for the Mac as it is written. On another
  // system it takes the card in Download: name, logo, address, note.
  function system() {
    var data = navigator.userAgentData;
    var p = (data && data.platform) || navigator.platform || '';
    // An iPad says it is a Mac, with a touch screen.
    if ((data && data.mobile) || /Android|iPhone|iPad|iPod|CrOS/.test(navigator.userAgent) ||
        (/^Mac/i.test(p) && navigator.maxTouchPoints > 1)) { return 'other'; }
    return /^Win/i.test(p) ? 'windows' : /^Mac/i.test(p) ? 'mac' : /^Linux/i.test(p) ? 'linux' : 'other';
  }
  var os = system();
  var row = document.querySelector('[data-os="' + os + '"]');
  if (os !== 'mac') {
    document.querySelectorAll('[data-download]').forEach(function (a) {
      a.href = row ? row.querySelector('a').href : '#downloads';
      a.querySelector('span').textContent = row ? 'Download for ' + row.dataset.name : 'See the downloads';
      var logo = a.querySelector('svg');
      if (row) { logo.firstElementChild.setAttribute('href', '#os-' + os); } else { logo.remove(); }
    });
    document.querySelectorAll('[data-note]').forEach(function (n) {
      n.textContent = row ? row.querySelector('header p').textContent + '. ' + row.querySelector('[data-needs]').textContent + ' New and little tested.'
        : 'For a computer with macOS, Linux or Windows.';
    });
    document.querySelectorAll('[data-others]').forEach(function (a) {
      if (!row) { a.parentNode.remove(); return; }
      a.textContent = os === 'linux' ? 'macOS and Windows' : 'macOS and Linux';
    });
    document.querySelectorAll('.get-card').forEach(function (c) {
      c.querySelector('.btn').classList.toggle('btn-main', c === row);
    });
  }

  // The keys of the Develop tab, on the first picture. They only change which
  // picture shows, and work while it is in view. S, Z, and C turn off again.
  var keys = document.querySelector('.keys');
  if (keys) {
    var shot = keys.closest('figure');
    var shotImg = shot.querySelector('img');
    var buttons = Array.prototype.slice.call(keys.querySelectorAll('button'));
    var view = 'develop';
    var show = function (next) {
      if (next === view) { next = next === 'before' ? 'before' : 'develop'; }
      view = next;
      buttons.forEach(function (b) { b.setAttribute('aria-pressed', String(b.dataset.view === view)); });
      var name = 'assets/app-' + (view === 'develop' ? '' : 'view-') + view;
      var ready = new Image();
      ready.sizes = shotImg.sizes;
      ready.srcset = name + '-1200.webp 1200w, ' + name + '-2400.webp 2400w';
      ready.decode().then(function () {
        if (view === next) { shotImg.srcset = ready.srcset; shotImg.src = name + '-2400.webp'; }
      }, function () {});
    };
    buttons.forEach(function (b) { b.addEventListener('click', function () { show(b.dataset.view); }); });
    document.addEventListener('keydown', function (e) {
      if (e.ctrlKey || e.metaKey || e.altKey || e.shiftKey || e.repeat || /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName)) { return; }
      var r = shot.getBoundingClientRect();
      var b = buttons.find(function (x) { return x.querySelector('kbd').textContent.toLowerCase() === e.key.toLowerCase(); });
      if (b && r.bottom > 0 && r.top < window.innerHeight) { show(b.dataset.view); }
    });
  }

  // The split. The range input is the control: the keyboard moves it, and the
  // grip on the line moves it for a pointer. The picture itself takes no
  // pointer, so a swipe on it scrolls the page.
  document.querySelectorAll('.compare').forEach(function (fig) {
    var range = fig.querySelector('.compare-range');
    var grip = fig.querySelector('.compare-grip');
    var frame = fig.querySelector('.compare-frame');
    if (!range || !grip || !frame) { return; }
    var start = range.value;
    function set(v) {
      if (v !== undefined) { range.value = Math.min(100, Math.max(0, v)).toFixed(1); }
      fig.style.setProperty('--split', range.value + '%');
      range.setAttribute('aria-valuetext', 'Split at ' + Math.round(range.value) + ' percent');
    }
    range.addEventListener('input', function () { set(); });
    // Arrows move 2, with Shift 10.
    range.addEventListener('keydown', function (e) {
      var dir = { ArrowRight: 1, ArrowUp: 1, ArrowLeft: -1, ArrowDown: -1 }[e.key];
      if (!dir) { return; }
      e.preventDefault();
      set(Number(range.value) + dir * (e.shiftKey ? 10 : 2));
    });
    var dragging = false;
    grip.addEventListener('pointerdown', function (e) {
      if (e.button) { return; }
      dragging = true;
      grip.setPointerCapture(e.pointerId);
      e.preventDefault();
    });
    grip.addEventListener('pointermove', function (e) {
      if (!dragging) { return; }
      var r = frame.getBoundingClientRect();
      set((e.clientX - r.left) / r.width * 100);
    });
    function drop() { dragging = false; }
    grip.addEventListener('pointerup', drop);
    grip.addEventListener('pointercancel', drop);
    grip.addEventListener('dblclick', function () { set(Number(start)); });
    set();
  });

  // The mark on the wedge: the section that holds the middle of the view.
  var links = Array.prototype.slice.call(document.querySelectorAll('.rail a'));
  var sections = links.map(function (a) { return document.getElementById(a.hash.slice(1)); });
  var current = -1;
  function mark() {
    var mid = window.innerHeight / 2;
    var at = -1;
    sections.forEach(function (s, i) {
      if (s && s.getBoundingClientRect().top <= mid) { at = i; }
    });
    var last = sections[sections.length - 1];
    if (last && last.getBoundingClientRect().bottom < mid) { at = -1; }
    if (at === current) { return; }
    if (current >= 0) { links[current].removeAttribute('aria-current'); }
    if (at >= 0) { links[at].setAttribute('aria-current', 'true'); }
    current = at;
  }
  var queued = false;
  window.addEventListener('scroll', function () {
    if (queued) { return; }
    queued = true;
    requestAnimationFrame(function () { queued = false; mark(); });
  }, { passive: true });
  window.addEventListener('resize', mark);
  mark();
})();
