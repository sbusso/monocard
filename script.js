// Monocard landing page. No dependency, no request.
(function () {
  'use strict';
  var doc = document.documentElement;
  doc.classList.add('js');

  // The source address is written once, on #repo in index.html.
  var repo = document.getElementById('repo');
  if (repo) {
    document.querySelectorAll('[data-repo]').forEach(function (a) { a.href = repo.href; });
  }

  // The download address is written once, on #download.
  var download = document.getElementById('download');
  if (download) {
    document.querySelectorAll('[data-download]').forEach(function (a) { a.href = download.href; });
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
